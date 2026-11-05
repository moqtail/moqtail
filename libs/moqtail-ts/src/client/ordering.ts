/**
 * Copyright 2026 The MOQtail Authors
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

import type { ObjectOrdering } from '@/client/types'
import { Location } from '@/model/common/location'
import type { MoqtObject } from '@/model/data/object'
import { findProperty } from '@/model/property/find'
import { PriorGroupIdGapProperty, PriorObjectIdGapProperty } from '@/model/property/object_property'
import { logger } from '@/util/logger'

interface Held {
  object: MoqtObject
  /** The object before it on the same fetch stream, which is its predecessor by definition. */
  after: Location | undefined
  /** `performance.now()` when it arrived. */
  at: number
  /** Arrival order, since clock readings can tie. */
  seq: number
}

/** IDs before this object in its group that don't exist (moqt-22 Prior Object ID Gap). */
const objectGap = (object: MoqtObject) => findProperty(object.properties, PriorObjectIdGapProperty)?.gap ?? 0n
/** Group IDs before this object's group that don't exist (moqt-22 Prior Group ID Gap). */
const groupGap = (object: MoqtObject) => findProperty(object.properties, PriorGroupIdGapProperty)?.gap ?? 0n

/**
 * Releases one request's objects in location order, whatever streams or datagrams they
 * arrived on. An object goes out as soon as its predecessor has; one whose predecessor
 * is missing waits until the window is exceeded, then everything up to the oldest held
 * object goes out and the expected locations skipped are reported through `gap`.
 *
 * Nothing is released yet, so the first object goes out at once only when it starts a
 * group: a mid-group first object waits for the start of its group, which a fill fetch
 * stream may still bring.
 */
export class ObjectOrderer {
  #held: Held[] = []
  /** The release point: the location of the last object released in order. */
  #last: Location | undefined
  /** The last Object ID of each group known to be complete. */
  #ends = new Map<bigint, bigint>()
  #timer: ReturnType<typeof setTimeout> | undefined
  #arrivals = 0
  #suspended = false

  constructor(
    readonly ordering: ObjectOrdering,
    readonly release: (object: MoqtObject) => void,
    readonly gap: (from: Location, to: Location) => void,
  ) {}

  /** Takes an object; `after` is the object before it on the same fetch stream. */
  push(object: MoqtObject, after?: Location): void {
    if (this.#last && object.location.compare(this.#last) <= 0) {
      if (this.ordering.onWindowExceeded !== 'drop') this.release(object)
      return
    }
    const entry = { object, after, at: performance.now(), seq: this.#arrivals++ }
    const index = this.#held.findIndex((held) => held.object.location.compare(object.location) > 0)
    if (index < 0) this.#held.push(entry)
    else this.#held.splice(index, 0, entry)
    this.#drain()
    this.#expire()
  }

  /**
   * While suspended nothing exceeds the window, so held objects wait for what's missing.
   * Resuming releases at once what waited longer than the window.
   */
  suspendWindow(suspended: boolean): void {
    if (this.#suspended === suspended) return
    this.#suspended = suspended
    this.#expire()
  }

  /** A stream carrying the End of Group for `group` ended cleanly with `lastObject`. */
  endGroup(group: bigint, lastObject: bigint): void {
    this.#ends.set(group, lastObject)
    this.#drain()
    this.#schedule()
  }

  /** Releases everything held, in order, and stops the timer. Call before closing the stream. */
  flush(): void {
    clearTimeout(this.#timer)
    this.#timer = undefined
    while (this.#held.length > 0) this.#force(this.#held.shift()!)
  }

  #drain(): void {
    while (this.#held.length > 0 && this.#isNext(this.#held[0]!)) this.#emit(this.#held.shift()!.object)
  }

  #expire(): void {
    while (this.#exceeded()) {
      const oldest = this.#oldest().object.location
      while (this.#held[0] && this.#held[0].object.location.compare(oldest) <= 0) this.#force(this.#held.shift()!)
      this.#drain()
    }
    this.#schedule()
  }

  #exceeded(): boolean {
    if (this.#suspended || this.#held.length === 0) return false
    const { durationMs, objects } = this.ordering.window
    if (objects !== undefined && this.#held.length > objects) return true
    return durationMs !== undefined && performance.now() - this.#oldest().at >= durationMs
  }

  #oldest(): Held {
    return this.#held.reduce((oldest, held) => (held.seq < oldest.seq ? held : oldest))
  }

  /** One timer, armed only while objects are held, for when the oldest one exceeds the window. */
  #schedule(): void {
    clearTimeout(this.#timer)
    this.#timer = undefined
    const { durationMs } = this.ordering.window
    if (this.#suspended || durationMs === undefined || this.#held.length === 0) return
    const wait = this.#oldest().at + durationMs - performance.now()
    this.#timer = setTimeout(
      () => {
        try {
          this.#expire()
        } catch (error) {
          // The consumer cancelled the stream: nothing is left to release to.
          logger.debug('ordering', 'release after the stream closed', error)
          this.#held = []
        }
      },
      Math.max(0, wait),
    )
  }

  #isNext({ object, after }: Held): boolean {
    const last = this.#last
    const { group, object: id } = object.location
    const sparseObjects = this.ordering.sparseObjectIds === true
    const startsGroup = sparseObjects || id === objectGap(object)
    if (!last) return startsGroup
    if (after?.equals(last)) return true
    if (group === last.group) return sparseObjects || id - 1n - objectGap(object) <= last.object
    const followsGroup = this.ordering.sparseGroupIds === true || group - 1n - groupGap(object) <= last.group
    return startsGroup && followsGroup && this.#ends.get(last.group) === last.object
  }

  /** Releases `held` out of turn, reporting the expected locations it skips. */
  #force(held: Held): void {
    const from = this.#skippedFrom(held)
    if (from) this.gap(from, held.object.location)
    this.#emit(held.object)
  }

  /**
   * The first expected location skipped between the release point and `held`, if any.
   * A group's missing tail isn't counted: its last ID is unknown.
   */
  #skippedFrom({ object, after }: Held): Location | undefined {
    const last = this.#last
    if (!last || after?.equals(last)) return undefined
    const { group, object: id } = object.location
    const sparseObjects = this.ordering.sparseObjectIds === true
    if (group === last.group) {
      return !sparseObjects && id - 1n - objectGap(object) > last.object
        ? new Location(group, last.object + 1n)
        : undefined
    }
    if (this.ordering.sparseGroupIds !== true && group - 1n - groupGap(object) > last.group) {
      return new Location(last.group + 1n, 0n)
    }
    return !sparseObjects && id > objectGap(object) ? new Location(group, 0n) : undefined
  }

  #emit(object: MoqtObject): void {
    const { group, object: id } = object.location
    if (this.#last?.group !== group) {
      for (const known of this.#ends.keys()) if (known < group) this.#ends.delete(known)
    }
    this.#last = object.location
    if (object.isEndOfGroup()) this.#ends.set(group, id)
    this.release(object)
  }
}

if (import.meta.vitest) {
  const { describe, test, expect, vi, beforeEach, afterEach } = import.meta.vitest
  const { MoqtObject } = await import('@/model/data/object')
  const { FullTrackName } = await import('@/model/data/full_track_name')
  const { ObjectForwardingPreference, ObjectStatus } = await import('@/model/data/constant')

  const ftn = FullTrackName.tryNew('room', 'video')
  /** `'g.o'`, `'g.o!'` for End of Group, with optional Prior Group / Object ID Gaps. */
  function obj(name: string, gaps: { group?: bigint; object?: bigint } = {}): MoqtObject {
    const [g, o] = name.replace('!', '').split('.').map(BigInt) as [bigint, bigint]
    const properties = [
      ...(gaps.group ? [new PriorGroupIdGapProperty(gaps.group).toKeyValuePair()] : []),
      ...(gaps.object ? [new PriorObjectIdGapProperty(gaps.object).toKeyValuePair()] : []),
    ]
    const args = [ftn, new Location(g, o), 0, ObjectForwardingPreference.Subgroup, 0, properties] as const
    return name.endsWith('!')
      ? MoqtObject.newWithStatus(...args, ObjectStatus.EndOfGroup)
      : MoqtObject.newWithPayload(...args, new Uint8Array([1]))
  }
  const at = (l: Location) => `${l.group}.${l.object}`

  function orderer(ordering: Partial<ObjectOrdering> = {}) {
    const out: string[] = []
    const gaps: string[] = []
    const orderer = new ObjectOrderer(
      { window: { durationMs: 150 }, ...ordering },
      (o) => out.push(at(o.location) + (o.isEndOfGroup() ? '!' : '')),
      (from, to) => gaps.push(`${at(from)}-${at(to)}`),
    )
    const push = (...names: string[]) => names.forEach((name) => orderer.push(obj(name)))
    return { orderer, push, out, gaps }
  }

  describe('ObjectOrderer', () => {
    beforeEach(() => {
      vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'performance'] })
    })
    afterEach(() => {
      vi.useRealTimers()
    })

    test('passes objects in order straight through, across End of Group', () => {
      const { push, out } = orderer()
      push('0.0', '0.1', '0.2!', '1.0', '1.1')
      expect(out).toEqual(['0.0', '0.1', '0.2!', '1.0', '1.1'])
    })

    test('reorders objects reversed within a group', () => {
      const { push, out } = orderer()
      push('0.0', '0.3', '0.2', '0.1')
      expect(out).toEqual(['0.0', '0.1', '0.2', '0.3'])
    })

    test('holds the head of group N+1 until the tail of N, one stream per object', () => {
      const { push, out } = orderer()
      push('0.0', '1.0', '0.1', '1.1', '0.2!')
      expect(out).toEqual(['0.0', '0.1', '0.2!', '1.0', '1.1'])
    })

    test('starts group N+1 when the stream carrying the end of group N ends', () => {
      const { orderer: o, push, out } = orderer()
      push('0.0', '0.1', '1.0')
      expect(out).toEqual(['0.0', '0.1'])
      o.endGroup(0n, 1n)
      expect(out).toEqual(['0.0', '0.1', '1.0'])
    })

    test('a suspended window holds past it; resuming releases what waited longer', () => {
      const { orderer: o, push, out, gaps } = orderer({ window: { durationMs: 150, objects: 1 } })
      o.suspendWindow(true)
      push('0.0', '0.2', '0.3')
      vi.advanceTimersByTime(1000)
      expect(out).toEqual(['0.0'])
      o.suspendWindow(false)
      expect(out).toEqual(['0.0', '0.2', '0.3'])
      expect(gaps).toEqual(['0.1-0.2'])
    })

    test('without an end of group, the next group waits for the window and is no gap', () => {
      const { push, out, gaps } = orderer()
      push('0.0', '1.0')
      vi.advanceTimersByTime(150)
      expect(out).toEqual(['0.0', '1.0'])
      expect(gaps).toEqual([])
    })

    test.each(['flush', 'drop'] as const)('%s: a missing object hits the window, the late one after', (policy) => {
      const { push, out, gaps } = orderer({ onWindowExceeded: policy })
      push('0.0', '0.2', '0.3')
      vi.advanceTimersByTime(149)
      expect(out).toEqual(['0.0'])
      vi.advanceTimersByTime(1)
      expect(out).toEqual(['0.0', '0.2', '0.3'])
      expect(gaps).toEqual(['0.1-0.2'])
      push('0.1', '0.4')
      expect(out).toEqual(['0.0', '0.2', '0.3', ...(policy === 'flush' ? ['0.1'] : []), '0.4'])
    })

    test('the objects limit releases up to the oldest held object', () => {
      const { push, out, gaps } = orderer({ window: { objects: 2 } })
      push('0.0', '0.4', '0.2', '0.5')
      expect(out).toEqual(['0.0', '0.2', '0.4', '0.5'])
      expect(gaps).toEqual(['0.1-0.2', '0.3-0.4'])
    })

    test('skips IDs that gap properties mark as absent, without waiting or a report', () => {
      const { orderer: o, out, gaps } = orderer()
      o.push(obj('0.0'))
      o.push(obj('0.3', { object: 2n }))
      o.push(obj('0.4!'))
      o.push(obj('3.1', { group: 2n, object: 1n }))
      expect(out).toEqual(['0.0', '0.3', '0.4!', '3.1'])
      expect(gaps).toEqual([])
    })

    test('sparse IDs count as next and are not reported', () => {
      const { push, out, gaps } = orderer({ sparseObjectIds: true, sparseGroupIds: true })
      push('0.0', '0.2', '0.5!', '4.3')
      expect(out).toEqual(['0.0', '0.2', '0.5!', '4.3'])
      expect(gaps).toEqual([])
    })

    test('reports a skipped whole group, but not without sparseGroupIds', () => {
      const dense = orderer()
      dense.push('0.0', '0.1!', '2.0')
      vi.advanceTimersByTime(150)
      expect(dense.out).toEqual(['0.0', '0.1!', '2.0'])
      expect(dense.gaps).toEqual(['1.0-2.0'])

      const sparse = orderer({ sparseGroupIds: true })
      sparse.push('0.0', '0.1!', '2.0')
      expect(sparse.out).toEqual(['0.0', '0.1!', '2.0'])
    })

    test('orders a fill fetch stream with the live objects of its subscription', () => {
      const { orderer: o, push, out } = orderer()
      push('5.2', '5.3')
      // The fill: objects that follow each other on one fetch stream are adjacent.
      let after: Location | undefined
      for (const name of ['4.0', '4.1', '5.0', '5.1']) {
        const object = obj(name)
        o.push(object, after)
        after = object.location
      }
      expect(out).toEqual(['4.0', '4.1', '5.0', '5.1', '5.2', '5.3'])
    })

    test('flush releases everything held, in order', () => {
      const { orderer: o, push, out, gaps } = orderer()
      push('0.0', '0.3', '0.2')
      o.flush()
      expect(out).toEqual(['0.0', '0.2', '0.3'])
      expect(gaps).toEqual(['0.1-0.2'])
    })
  })
}
