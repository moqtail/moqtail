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

/** The SAP type timeline of CMSF 3.6, the only built-in event timeline. */

import { defineElement } from '@/player/element/define'
import { eventTimelineCapabilities, EventTimelineElement, type TimelineRecord } from '@/player/element/event_timeline'

export const SAP_EVENT_TYPE = 'org.ietf.moq.cmsf.sap'

/** A record whose `data` is `[sapType, ept]`, the earliest presentation time in milliseconds. */
export type SapRecord = TimelineRecord & { L: NonNullable<TimelineRecord['L']>; data: [number, number] }

export class SapTimelineElement extends EventTimelineElement {
  // CMSF 3.6.2 accumulates records per group
  protected readonly incremental = true
  readonly #records: SapRecord[] = []

  /** Every record delivered so far, sorted by location. */
  get records(): readonly SapRecord[] {
    return this.#records
  }

  /** A location-indexed record whose `data` is valid SAP data (CMSF 3.6.1). */
  protected override parseRecord(raw: Record<string, unknown>): SapRecord | undefined {
    const record = super.parseRecord(raw)
    if (!record?.L || !isSapData(record.data, record.L.object === 0n)) return undefined
    const sap = record as SapRecord
    // Kept by the base class's delivery rule, so the history independent objects repeat isn't stored twice
    const last = this.#records.at(-1)
    if (!last || sap.L.compare(last.L) > 0) this.#records.push(sap)
    return sap
  }
}

/** `[sapType, ept]`: SAP type 0 to 3, and 1 or 2 for an object that starts a group (CMSF 3.6.1). */
function isSapData(data: unknown, startsGroup: boolean): data is [number, number] {
  if (!Array.isArray(data) || data.length !== 2 || !data.every(Number.isInteger)) return false
  const [type] = data as [number, number]
  return startsGroup ? type === 1 || type === 2 : type >= 0 && type <= 3
}

export const SapTimeline = defineElement({
  type: 'source',
  name: 'saptimeline',
  capabilities: [eventTimelineCapabilities('out', SAP_EVENT_TYPE)],
  probe: (ids) => ids.packaging === 'eventtimeline' && ids.eventType === SAP_EVENT_TYPE,
  init: async (context) => {
    const timeline = new SapTimelineElement(context)
    await timeline.start()
    return timeline
  },
})

if (import.meta.vitest) {
  const { test, expect } = import.meta.vitest
  const { FakeClient, testObject } = await import('@/player/testing/client')
  const { testChain, testSource, settle, track } = await import('@/player/testing/elements')
  type TimelineEvent = import('@/player/element/event_timeline').TimelineEvent

  test('the CMSF 3.6.2 example; invalid SAP types are dropped', async () => {
    const client = new FakeClient()
    let sap: SapTimelineElement | undefined
    const sapTrack = { ...track('sap'), packaging: 'eventtimeline', eventType: SAP_EVENT_TYPE }
    const parent = testSource({
      script: async (s) => {
        sap = await s.ctx.load(SapTimeline, { tracks: [sapTrack] })
        s.emit('nomorepads')
      },
    })
    const { chain } = testChain(
      () => [],
      () => client,
    )
    const events: TimelineEvent[] = []
    const warnings: string[] = []
    chain.host.player.on('timelineevent', (e) => events.push(e as TimelineEvent))
    chain.host.player.on('warning', (w) => warnings.push(w.message))
    chain.addSource(parent, {})
    await chain.init(new AbortController().signal)
    const object = (group: number, id: number, records: unknown) =>
      testObject('sap', group, id, new TextEncoder().encode(JSON.stringify(records)))
    client.last.push(
      object(0, 0, [{ L: [0, 0], data: [2, 0] }]),
      object(0, 1, [{ L: [0, 60], data: [3, 2100] }]),
      object(1, 0, [
        { L: [0, 0], data: [2, 0] },
        { L: [0, 60], data: [3, 2100] },
        { L: [1, 0], data: [3, 4000] }, // a group starts with SAP type 1 or 2
        { L: [1, 30], data: [4, 5000] },
        { L: [1, 60], data: [3, 6100] },
      ]),
    )
    await settle()
    const show = (r: TimelineRecord) => `${r.L?.group}.${r.L?.object} ${String(r.data)}`
    expect(events.map((e) => show(e.record))).toEqual(['0.0 2,0', '0.60 3,2100', '1.60 3,6100'])
    expect(sap!.records.map(show)).toEqual(['0.0 2,0', '0.60 3,2100', '1.60 3,6100'])
    expect(warnings).toEqual(['Dropped 2 malformed record(s) of event timeline "sap"'])
  })
}
