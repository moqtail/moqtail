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
 * `EventTimelineElement`: the subscription and record delivery shared by event timeline
 * elements (MSF 8). Each `eventType` gets its own subclass. Records are delivered on arrival,
 * through the player's `timelineevent` event and, when connected, as packets on the
 * `eventtimeline` source pad.
 */

import { KeyValuePair } from '@/model/common/pair'
import { Location } from '@/model/common/location'
import { MoqtObject } from '@/model/data/object'
import { findProperty } from '@/model/property/find'
import { MEDIA_PRIORITY } from '@/player/constants'
import { PlayerError } from '@/player/error'
import { parseJsonArray } from '@/player/util/json'
import { TrackReader } from '@/player/util/track_reader'
import { SourceElement } from './base'
import type { ElementContext } from './context'
import { CapabilityCode, definitionOf, type Capabilities } from './define'
import type { CatalogTrack } from './interfaces'
import type { Pad } from './pad'

/** @public One event timeline record (MSF 8.2): exactly one of `T` (wallclock), `L`, or `M` (media time, ms). */
export interface TimelineRecord {
  T?: number
  L?: Location
  M?: number
  data: unknown
}

/** @public What the player's `timelineevent` event carries. */
export interface TimelineEvent {
  track: CatalogTrack
  eventType: string
  record: TimelineRecord
}

// Odd, so the value is bytes (moqt-22 KeyValuePair)
const MSF_EVENT_TIMELINE_HEADER = 0x7b

/** Track property: header data serialized as the track's `eventType` defines (MSF 12.3.1). */
export class MsfEventTimelineHeaderProperty {
  static readonly TYPE = MSF_EVENT_TIMELINE_HEADER

  constructor(readonly header: Uint8Array) {}

  toKeyValuePair(): KeyValuePair {
    return KeyValuePair.tryNewBytes(MSF_EVENT_TIMELINE_HEADER, this.header)
  }

  static fromKeyValuePair(pair: KeyValuePair): MsfEventTimelineHeaderProperty | undefined {
    if (pair.typeValue !== BigInt(MSF_EVENT_TIMELINE_HEADER) || !(pair.value instanceof Uint8Array)) return undefined
    return new MsfEventTimelineHeaderProperty(pair.value)
  }
}

/** A JSON `[group, object]` as a `Location`, or undefined when it isn't one. */
/** A pad carrying an event timeline of `eventType`. */
export function eventTimelineCapabilities(direction: 'in' | 'out', eventType: string): Capabilities {
  return [
    { direction, code: CapabilityCode.PACKAGING, value: 'eventtimeline' },
    { direction, code: CapabilityCode.EVENT_TYPE, value: eventType },
  ]
}

export function toLocation(value: unknown): Location | undefined {
  if (!Array.isArray(value) || value.length !== 2) return undefined
  if (!value.every((n) => Number.isSafeInteger(n) && n >= 0)) return undefined
  return new Location(value[0], value[1])
}

/** Base of event timeline elements; it isn't an element itself. */
export abstract class EventTimelineElement extends SourceElement {
  /** MSF 8.4: a sequence with incremental updates (true), or one record per group (false). */
  protected abstract readonly incremental: boolean
  readonly track: CatalogTrack
  readonly #pad: Pad
  #reader: TrackReader | undefined
  #headerRead = false
  /** The last delivered record, so the history an independent object repeats isn't delivered again. */
  #last: TimelineRecord | undefined

  constructor(context: ElementContext) {
    super(context)
    this.track = context.tracks[0]!
    this.#pad = this.createPad('src', eventTimelineCapabilities('out', this.track.eventType ?? ''))
  }

  /** The `MSF_EVENT_TIMELINE_HEADER` value, read before the first record. The default ignores it. */
  protected parseHeader(_header: Uint8Array | undefined): void {}

  /**
   * One record object of the payload, validated: exactly one of `T`, `L`, `M`, and a `data`
   * field (MSF 8.2). Override to normalise or check `data`; undefined drops the record.
   */
  protected parseRecord(raw: Record<string, unknown>): TimelineRecord | undefined {
    const { T, L, M } = raw
    if (!('data' in raw) || [T, L, M].filter((i) => i !== undefined).length !== 1) return undefined
    if (L !== undefined) {
      const location = toLocation(L)
      return location && { L: location, data: raw['data'] }
    }
    const index = T ?? M
    if (typeof index !== 'number' || !Number.isFinite(index)) return undefined
    return T !== undefined ? { T: index, data: raw['data'] } : { M: index, data: raw['data'] }
  }

  /**
   * Subscribes. A rejected subscription is a `TRACK_SKIPPED` warning, since event
   * timelines are optional and shouldn't stop playback.
   */
  async start(): Promise<void> {
    const { namespace, nameBytes } = this.track
    const received = (payload: Uint8Array, _: Location, object: MoqtObject) => this.#received(payload, object)
    this.#reader = new TrackReader(this.context, {
      track: { namespace, name: nameBytes },
      priority: MEDIA_PRIORITY,
      incremental: this.incremental,
      onIndependent: received,
      onIncremental: received,
      onStale: (reason) => this.#warn(`Event timeline "${this.track.name}" went stale: ${reason.message}`, reason),
    })
    try {
      await this.#reader.start()
    } catch (cause) {
      this.#warn(`Event timeline "${this.track.name}" can't be subscribed`, cause)
    }
  }

  override async destroy(): Promise<void> {
    await this.#reader?.stop()
    await super.destroy()
  }

  #received(payload: Uint8Array, object: MoqtObject): void {
    if (!this.#headerRead) {
      this.parseHeader(findProperty(this.#reader?.subscribeOk?.trackProperties, MsfEventTimelineHeaderProperty)?.header)
      this.#headerRead = true
    }
    if (this.#pad.peers.length > 0) {
      const decompressed = MoqtObject.newWithPayload(
        object.fullTrackName,
        object.location,
        object.publisherPriority,
        object.objectForwardingPreference,
        object.subgroupId,
        object.properties,
        payload,
      )
      void this.#pad.write({ type: 'packet', object: decompressed })
    }
    const json = parseJsonArray(payload, 'An event timeline must be a JSON array of records (MSF 8.2)')
    const eventType = this.track.eventType ?? ''
    let dropped = 0
    for (const raw of json) {
      const isObject = typeof raw === 'object' && raw !== null && !Array.isArray(raw)
      const record = isObject ? this.parseRecord(raw as Record<string, unknown>) : undefined
      if (!record) dropped++
      if (!record || (this.incremental && !isAfter(record, this.#last))) continue
      this.#last = record
      this.context.player.emit('timelineevent', { track: this.track, eventType, record })
    }
    if (dropped > 0) this.#warn(`Dropped ${dropped} malformed record(s) of event timeline "${this.track.name}"`)
  }

  #warn(message: string, cause?: unknown): void {
    const element = definitionOf(this)?.name
    this.context.player.emit(
      'warning',
      new PlayerError('TRACK_SKIPPED', message, { cause, ...(element && { element }) }),
    )
  }
}

/** Whether `record` comes after `last`. Records with different index types can't be ordered, so they're kept. */
function isAfter(record: TimelineRecord, last: TimelineRecord | undefined): boolean {
  if (!last) return true
  if (record.L && last.L) return record.L.compare(last.L) > 0
  if (record.T !== undefined && last.T !== undefined) return record.T > last.T
  if (record.M !== undefined && last.M !== undefined) return record.M > last.M
  return true
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest
  const { FakeClient, testObject } = await import('@/player/testing/client')
  const { testChain, testSource, collectingSink, flush, settle, track } = await import('@/player/testing/elements')
  type CollectingSink = import('@/player/testing/elements').CollectingSink
  const { defineElement } = await import('./define')

  const text = (s: string) => new TextEncoder().encode(s)
  const object = (group: number, id: number, records: unknown) =>
    testObject('events', group, id, text(JSON.stringify(records)))

  /** A test event timeline loaded by a top-level source, with an optional sink taking its pad. */
  async function setup(incremental: boolean, options: { header?: Uint8Array; sink?: boolean } = {}) {
    const client = new FakeClient()
    if (options.header) {
      client.respond = () => ({ trackProperties: [new MsfEventTimelineHeaderProperty(options.header!) as never] })
    }
    const headers: (string | undefined)[] = []
    class Events extends EventTimelineElement {
      protected readonly incremental = incremental
      protected override parseHeader(header: Uint8Array | undefined): void {
        headers.push(header && new TextDecoder().decode(header))
      }
    }
    const definition = defineElement({
      type: 'source',
      name: 'events',
      capabilities: [],
      probe: () => true,
      init: async (context) => {
        const element = new Events(context)
        await element.start()
        return element
      },
    })
    const events = { ...track('events'), packaging: 'eventtimeline', eventType: 'x' }
    const parent = testSource({
      script: async (s) => {
        await s.ctx.load(definition, { tracks: [events] })
        s.emit('nomorepads')
      },
    })
    const { chain } = testChain(
      () => [],
      () => client,
    )
    const delivered: TimelineEvent[] = []
    const warnings: string[] = []
    chain.host.player.on('timelineevent', (e) => delivered.push(e as TimelineEvent))
    chain.host.player.on('warning', (w) => warnings.push(w.message))
    chain.addSource(parent, {})
    let sink: CollectingSink | undefined
    if (options.sink) chain.addSink(collectingSink({ setup: (s) => ((sink = s), (s.accept = () => true)) }))
    void chain.init(new AbortController().signal)
    await flush()
    const push = async (...objects: MoqtObject[]) => {
      client.last.push(...objects)
      await settle()
    }
    return { events: delivered, warnings, headers, push, sink: () => sink }
  }

  describe('EventTimelineElement', () => {
    test('MSF 8.5 records by T, L, and M; malformed ones dropped with a warning', async () => {
      const t = await setup(false)
      await t.push(
        object(1, 0, [{ T: 1756885678361, data: { clock: '12:00' } }]),
        object(2, 0, [
          { L: [0, 0], data: [47.1812, 8.4592] },
          { L: [0], data: 1 },
          { M: 1, T: 2, data: 1 },
        ]),
        object(3, 0, [{ M: 23051, data: { class: 'human' } }, { M: 'x', data: 1 }, { M: 1 }, 'record']),
      )
      expect(t.events.map((e) => [e.eventType, e.record])).toEqual([
        ['x', { T: 1756885678361, data: { clock: '12:00' } }],
        ['x', { L: new Location(0n, 0n), data: [47.1812, 8.4592] }],
        ['x', { M: 23051, data: { class: 'human' } }],
      ])
      expect(t.warnings).toEqual([
        'Dropped 2 malformed record(s) of event timeline "events"',
        'Dropped 3 malformed record(s) of event timeline "events"',
      ])
    })

    test('a sequence delivers each record once across independent and incremental objects', async () => {
      const t = await setup(true, { header: text('{"crs":"WGS84"}') })
      const at = (group: number) => ({ L: [group, 0], data: group })
      await t.push(object(1, 0, [at(0), at(1)]), object(1, 1, [at(2)]), object(2, 0, [at(0), at(1), at(2), at(3)]))
      expect(t.events.map((e) => e.record.data)).toEqual([0, 1, 2, 3])
      expect(t.headers).toEqual(['{"crs":"WGS84"}'])
    })

    test('one record per group ignores later objects; a connected pad gets the objects', async () => {
      const t = await setup(false, { sink: true })
      await t.push(
        object(1, 0, [{ M: 0, data: 'a' }]),
        object(1, 1, [{ M: 1, data: 'b' }]),
        object(2, 0, [{ M: 0, data: 'c' }]),
      )
      expect(t.events.map((e) => e.record.data)).toEqual(['a', 'c'])
      expect(t.headers).toEqual([undefined])
      const packets = t
        .sink()!
        .items()
        .filter((i) => i.type === 'packet')
      expect(packets.map((p) => (p.type === 'packet' ? p.object.location.group : -1))).toEqual([1n, 2n])
    })
  })
}
