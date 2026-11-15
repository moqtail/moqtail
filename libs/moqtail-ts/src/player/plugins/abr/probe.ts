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

/**
 * Live delivery is paced by the publisher, so it never shows capacity above the current bitrate. A
 * probe FETCHes objects that already arrived, which the relay sends as fast as the link allows
 * below live priority, and measures everything that arrives meanwhile.
 */

import type { MOQtailClient } from '@/client/client'
import { GroupOrder, LocationFilterType } from '@/model/control/constant'
import type { FullTrackName } from '@/model/data/full_track_name'
import { LocationFilter } from '@/model/parameter/message/location_filter'
import { CONFIRMING, UP_EVIDENCE, type Observation } from './features'

/** An object that arrived whole. */
export interface KnownObject {
  id: bigint
  bytes: number
}

/** A group of a track whose objects all arrived: the next group of the track had started. */
export interface KnownGroup {
  track: string
  group: bigint
  objects: readonly KnownObject[]
}

interface ObjectLocation {
  group: bigint
  object: bigint
}

/** One FETCH of a probe: `start` to `end` inclusive. */
interface ProbeRange {
  start: ObjectLocation
  end: ObjectLocation
  objects: number
  bytes: number
}

/** An object received, `at` in seconds. */
export interface Arrival {
  at: number
  bytes: number
}

/** The track a probe FETCHes from. */
export interface ProbeTarget {
  name: string
  fullTrackName: FullTrackName
}

/** A probe carries the difference to the next bitrate for this many seconds. */
const PROBE_SECONDS = 1
/** Seconds a probe may take; what arrived by then is measured, the rest is cancelled. */
const PROBE_TIMEOUT = 2
/** Seconds of a probe's round trip before its data can arrive. */
const PROBE_ALLOWANCE = 0.25
/** Below this share of the pace that would show room, a probe can no longer show it, so it ends early. */
const PROBE_BEHIND = 0.5
/** Below live media's 128: the relay sends a probe only when live leaves the link idle. */
const PROBE_PRIORITY = 200

/** Bytes a probe for `next` needs on top of live delivery at `current`, both in bits per second. */
export function probeBytes(current: number, next: number): number {
  return Math.floor(((next - current) / 8) * PROBE_SECONDS)
}

/**
 * The FETCHes that carry closest to `targetBytes`. The known groups are walked oldest first as one
 * run, and the run stops at the object whose total lands closest to the target, at least one
 * object. A target beyond the whole run repeats it: the relay still has those objects cached.
 */
export function planProbe(groups: readonly KnownGroup[], targetBytes: number): ProbeRange[] {
  const objects = groups.flatMap(({ group, objects }) => objects.map(({ id, bytes }) => ({ group, id, bytes })))
  const total = objects.reduce((sum, { bytes }) => sum + bytes, 0)
  if (objects.length === 0 || total <= 0 || targetBytes <= 0) return []
  const prefix = (count: number, bytes: number): ProbeRange => ({
    start: { group: objects[0]!.group, object: objects[0]!.id },
    end: { group: objects[count - 1]!.group, object: objects[count - 1]!.id },
    objects: count,
    bytes,
  })
  const plan = Array.from({ length: Math.floor(targetBytes / total) }, () => prefix(objects.length, total))
  const rest = targetBytes - total * plan.length
  let count = 0
  let bytes = 0
  while (count < objects.length && bytes + objects[count]!.bytes < rest) bytes += objects[count++]!.bytes
  // The object that crosses the target goes in when that lands closer
  if (count < objects.length && bytes + objects[count]!.bytes - rest <= rest - bytes) bytes += objects[count++]!.bytes
  if (count === 0 && plan.length === 0) bytes += objects[count++]!.bytes
  if (count > 0) plan.push(prefix(count, bytes))
  return plan
}

/**
 * Link capacity a probe saw, in bytes per second: every byte that arrived from the request to the
 * last probe object, the probe's and `others`', over that time. Timing from the request, not the
 * first object, keeps bursts out: loss recovery releases held stream data at once, which squeezes
 * arrivals together. It costs about one RTT of underestimate.
 */
export function probeEstimate(probe: readonly Arrival[], others: readonly Arrival[], requestedAt: number): number {
  if (probe.length === 0) return 0
  const last = probe.reduce((latest, { at }) => Math.max(latest, at), -Infinity)
  const within = others.filter(({ at }) => at > requestedAt && at <= last)
  const bytes = [...probe, ...within].reduce((sum, arrival) => sum + arrival.bytes, 0)
  return bytes / Math.max(last - requestedAt, 1e-3)
}

interface InFlight {
  client: MOQtailClient
  track: string
  requestedAt: number
  /** Request ids, once FETCH_OK names them, for cancelling. */
  requestIds: Set<bigint>
  fetches: number
  /** Aborts reading the FETCH streams, which stops their data at the client. */
  abort: AbortController
  arrivals: Arrival[]
  expectedObjects: number
  expectedBytes: number
}

interface Estimate {
  bytesPerSecond: number
  /** Seconds it has aged while the controller was free to act on it. */
  age: number
}

/** Runs one probe at a time and keeps what the last `CONFIRMING` probes found. */
export class Prober {
  #inFlight: InFlight | undefined
  #estimates: Estimate[] = []
  /** When the last probe ended with an estimate; unlike `#endedAt`, forgotten with the estimates. */
  #estimatedAt: number | undefined
  #failures = 0
  #endedAt: number | undefined

  get probing(): boolean {
    return this.#inFlight !== undefined
  }

  /** The median of the recent estimates in bytes per second, if not forgotten. */
  get estimate(): number | undefined {
    return this.#estimates.length > 0 ? median(this.#estimates.map((e) => e.bytesPerSecond)) : undefined
  }

  start(client: MOQtailClient, target: ProbeTarget, ranges: readonly ProbeRange[], now: number): void {
    const inFlight: InFlight = {
      client,
      track: target.name,
      requestedAt: now,
      requestIds: new Set(),
      fetches: ranges.length,
      abort: new AbortController(),
      arrivals: [],
      expectedObjects: ranges.reduce((sum, range) => sum + range.objects, 0),
      expectedBytes: ranges.reduce((sum, range) => sum + range.bytes, 0),
    }
    this.#inFlight = inFlight
    for (const range of ranges) void this.#fetch(inFlight, target.fullTrackName, range)
  }

  /**
   * Counts an object of a fetch stream: by request id, and by track until every FETCH_OK named its
   * request, since on a congested link FETCH_OK can come after the objects. Fill FETCHes of the same
   * track are not the probe's.
   */
  received(track: string | undefined, requestId: bigint | undefined, arrival: Arrival): void {
    const inFlight = this.#inFlight
    if (!inFlight) return
    const ours = requestId !== undefined && inFlight.requestIds.has(requestId)
    const unnamed = inFlight.requestIds.size < inFlight.fetches && track !== undefined && track === inFlight.track
    if (ours || unnamed) inFlight.arrivals.push(arrival)
  }

  /**
   * Ends the probe in flight once its objects all arrived, it fell behind its pace, or it timed out,
   * and returns its estimate. `others` are the live objects received meanwhile; `nextBitrate` is the
   * one probed for, undefined at the top of the ladder.
   */
  settle(now: number, others: readonly Arrival[], nextBitrate: number | undefined): number | undefined {
    const inFlight = this.#inFlight
    if (!inFlight) return undefined
    const deadline = inFlight.requestedAt + PROBE_TIMEOUT
    const arrivals = inFlight.arrivals.filter(({ at }) => at <= deadline)
    const complete = arrivals.length >= inFlight.expectedObjects
    if (!complete && !behindPace(inFlight, arrivals, now) && now < deadline) return undefined
    this.cancel()
    const bytesPerSecond = probeEstimate(arrivals, others, inFlight.requestedAt)
    const room = nextBitrate === undefined || bytesPerSecond * 8 >= nextBitrate
    this.#failures = room ? 0 : this.#failures + 1
    this.#estimates = [...this.#estimates, { bytesPerSecond, age: 0 }].slice(-CONFIRMING)
    this.#estimatedAt = now
    this.#endedAt = now
    return bytesPerSecond
  }

  /** Drops estimates the link has since contradicted. The failures, and so the backoff, stay. */
  forgetEstimates(): void {
    this.#estimates = []
    this.#estimatedAt = undefined
  }

  /** Ages the estimates as evidence; the controller skips it while a switch settles. */
  age(seconds: number): void {
    for (const estimate of this.#estimates) estimate.age += seconds
  }

  /** A move down: the link dropped, so probing again waits, without escalating the backoff. */
  steppedDown(now: number): void {
    this.#failures = Math.max(this.#failures, 1)
    this.#endedAt = now
  }

  /** A move up the ladder: what failed below says nothing about the next rendition. */
  forgetFailures(): void {
    this.#failures = 0
  }

  /**
   * Cancels the probe in flight: the relay stops serving its FETCHes, and the client stops reading
   * their streams, so data already on its way stops too.
   */
  cancel(): void {
    const inFlight = this.#inFlight
    if (!inFlight) return
    this.#inFlight = undefined
    inFlight.abort.abort()
    for (const id of inFlight.requestIds) {
      // The session may be gone, with the FETCH
      Promise.resolve(inFlight.client.fetchCancel(id)).catch(() => {})
    }
  }

  observe(
    now: number,
  ): Pick<Observation, 'probeEstimate' | 'probeAge' | 'evidence' | 'probing' | 'probeFailures' | 'sinceProbe'> {
    const fresh = this.#estimates.filter(({ age }) => age < UP_EVIDENCE).map((e) => e.bytesPerSecond)
    return {
      probeEstimate: this.estimate ?? null,
      probeAge: this.#estimatedAt === undefined ? null : now - this.#estimatedAt,
      evidence: fresh.length === CONFIRMING ? median(fresh) : null,
      probing: this.probing,
      probeFailures: this.#failures,
      sinceProbe: this.#endedAt === undefined ? null : now - this.#endedAt,
    }
  }

  async #fetch(inFlight: InFlight, fullTrackName: FullTrackName, { start, end }: ProbeRange): Promise<void> {
    const locationFilter = new LocationFilter(
      LocationFilterType.AbsoluteRange,
      start.group,
      start.object,
      end.group,
      end.object,
    )
    try {
      const result = await inFlight.client.fetch({
        priority: PROBE_PRIORITY,
        groupOrder: GroupOrder.Ascending,
        fullTrackName,
        locationFilter,
      })
      // A refused FETCH leaves the probe to time out with what else arrived
      if (!('stream' in result)) return
      if (this.#inFlight !== inFlight) {
        // The probe ended before this FETCH was accepted, so its cancel missed it
        result.stream.cancel().catch(() => {})
        Promise.resolve(inFlight.client.fetchCancel(result.requestId)).catch(() => {})
        return
      }
      inFlight.requestIds.add(result.requestId)
      // Arrivals are counted from the client's data events, so the objects themselves are dropped
      await result.stream.pipeTo(new WritableStream(), { signal: inFlight.abort.signal })
    } catch {
      // A cancelled probe errors its stream
    }
  }
}

/** Under `PROBE_BEHIND` of the pace that would show room, once the round trip is allowed for. */
function behindPace(inFlight: InFlight, arrivals: readonly Arrival[], now: number): boolean {
  const elapsed = now - inFlight.requestedAt - PROBE_ALLOWANCE
  const delivered = arrivals.reduce((sum, { bytes }) => sum + bytes, 0)
  return elapsed > 0 && delivered < ((PROBE_BEHIND * inFlight.expectedBytes) / PROBE_SECONDS) * elapsed
}

function median(values: readonly number[]): number {
  const ordered = [...values].sort((a, b) => a - b)
  const middle = Math.floor(ordered.length / 2)
  return ordered.length % 2 === 1 ? ordered[middle]! : (ordered[middle - 1]! + ordered[middle]!) / 2
}

if (import.meta.vitest) {
  const { describe, test, expect, vi } = import.meta.vitest

  /** Groups with object ids 0, 1, … and the given sizes. */
  const known = (...groups: [number, ...number[]][]): KnownGroup[] =>
    groups.map(([group, ...sizes]) => ({
      track: 'v',
      group: BigInt(group),
      objects: sizes.map((bytes, id) => ({ id: BigInt(id), bytes })),
    }))
  const summary = (plan: ProbeRange[]) =>
    plan.map((r) => `${r.start.group}.${r.start.object}-${r.end.group}.${r.end.object} ${r.objects}/${r.bytes}`)

  describe('planProbe', () => {
    const groups = known([7, 100, 10, 10], [8, 100, 10, 10])

    test('stops at the object that lands closest to the target, at least one', () => {
      expect(summary(planProbe(groups, 114))).toEqual(['7.0-7.1 2/110'])
      expect(summary(planProbe(groups, 116))).toEqual(['7.0-7.2 3/120'])
      expect(summary(planProbe(groups, 20))).toEqual(['7.0-7.0 1/100'])
    })

    test('repeats the whole run for a target beyond it', () => {
      expect(summary(planProbe(groups, 300))).toEqual(['7.0-8.2 6/240', '7.0-7.0 1/100'])
      expect(summary(planProbe(groups, 250))).toEqual(['7.0-8.2 6/240'])
    })

    test('plans nothing without objects or a target', () => {
      expect(planProbe([], 100)).toEqual([])
      expect(planProbe(groups, 0)).toEqual([])
    })
  })

  test('probeEstimate counts every byte from the request to the last probe object', () => {
    const probe = [
      { at: 0.1, bytes: 1000 },
      { at: 0.5, bytes: 1000 },
    ]
    const live = [
      { at: -0.1, bytes: 500 }, // before the request
      { at: 0.3, bytes: 500 },
      { at: 0.6, bytes: 500 }, // after the last probe object
    ]
    expect(probeEstimate(probe, live, 0)).toBe(2500 / 0.5)
    expect(probeEstimate([], live, 0)).toBe(0)
  })

  describe('Prober', () => {
    const fakeClient = () => {
      let requestId = 0n
      return {
        fetch: vi.fn(async () => ({ requestId: ++requestId, stream: new ReadableStream({ start: (c) => c.close() }) })),
        fetchCancel: vi.fn(async () => {}),
      }
    }
    const target = { name: 'v', fullTrackName: {} as FullTrackName }
    const plan = planProbe(known([1, 1000, 1000]), 2000)

    test('settles once every planned object arrived, and counts probes that found no room', async () => {
      const client = fakeClient()
      const prober = new Prober()
      prober.start(client as unknown as MOQtailClient, target, plan, 10)
      await vi.waitFor(() => expect(client.fetch).toHaveBeenCalledTimes(1))
      prober.received('v', 1n, { at: 10.1, bytes: 1000 })
      expect(prober.settle(10.1, [], 1e6)).toBeUndefined()
      prober.received('other', undefined, { at: 10.15, bytes: 1000 })
      prober.received('v', 1n, { at: 10.2, bytes: 1000 })
      expect(prober.settle(10.2, [], 1e6)).toBeCloseTo(10_000)
      expect(prober.observe(11)).toMatchObject({
        probeEstimate: expect.closeTo(10_000),
        probing: false,
        probeFailures: 1,
        sinceProbe: expect.closeTo(0.8),
      })
    })

    test('ends early once it falls behind the pace that would show room, then cancels', async () => {
      const client = fakeClient()
      const prober = new Prober()
      prober.start(client as unknown as MOQtailClient, target, plan, 0)
      await vi.waitFor(() => expect(client.fetch).toHaveBeenCalled())
      await Promise.resolve()
      expect(prober.settle(0.2, [], 1e6)).toBeUndefined() // within the round trip's allowance
      expect(prober.settle(0.5, [], 1e6)).toBe(0) // nothing yet, half of its pace was due
      expect(client.fetchCancel).toHaveBeenCalledWith(1n)
      prober.forgetEstimates()
      expect(prober.observe(3)).toMatchObject({ probeEstimate: null, probeAge: null, probeFailures: 1 })
      prober.forgetFailures()
      expect(prober.observe(3).probeFailures).toBe(0)
    })

    test('confirms room only with three fresh estimates, by their median', async () => {
      const prober = new Prober()
      const probeOnce = async (now: number, bytes: number) => {
        const client = fakeClient()
        prober.start(client as unknown as MOQtailClient, target, plan, now)
        await vi.waitFor(() => expect(client.fetch).toHaveBeenCalled())
        prober.received('v', 1n, { at: now + 0.1, bytes })
        prober.received('v', 1n, { at: now + 0.2, bytes })
        prober.settle(now + 0.2, [], 1e6)
      }
      await probeOnce(0, 1000)
      await probeOnce(1, 9000) // an outlier
      expect(prober.observe(2).evidence).toBeNull()
      await probeOnce(2, 1000)
      expect(prober.observe(3).evidence).toBeCloseTo(10_000) // the median, not the outlier
      prober.age(10)
      expect(prober.observe(3).evidence).toBeNull() // no longer fresh
    })

    test("counts the track's other FETCHes only until FETCH_OK named the probe's", async () => {
      const client = fakeClient()
      const prober = new Prober()
      prober.start(client as unknown as MOQtailClient, target, plan, 0)
      prober.received('v', 7n, { at: 0.05, bytes: 1000 }) // before FETCH_OK: by track
      await vi.waitFor(() => expect(client.fetch).toHaveBeenCalled())
      await Promise.resolve()
      prober.received('v', 7n, { at: 0.1, bytes: 1000 }) // a fill of the same track
      expect(prober.settle(0.1, [], 1e6)).toBeUndefined()
      prober.received('v', 1n, { at: 0.2, bytes: 1000 })
      expect(prober.settle(0.2, [], 1e6)).toBeCloseTo(10_000)
    })
  })
}
