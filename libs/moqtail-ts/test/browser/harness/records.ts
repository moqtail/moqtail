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
 * What a session saw, from its start or from a mark on. Every entry has `at`, a
 * `performance.now()` timestamp, so a view is the entries at or after its mark.
 */

import { expect } from 'vitest'
import type { DataEvent } from '@/client/data_event'
import type { Location } from '@/model/common/location'
import type { LocationFilter } from '@/model/parameter/message/location_filter'
import type { PlayerEvents, PlayerState } from '@/player'

export interface StateRecord {
  state: PlayerState
  previous: PlayerState
  at: number
}

export interface EventRecord<K extends keyof PlayerEvents = keyof PlayerEvents> {
  name: K
  args: Parameters<PlayerEvents[K]>
  at: number
}

export interface RequestRecord {
  kind: 'subscribe' | 'switch' | 'fetch'
  /** Undefined when the request was rejected. */
  requestId: bigint | undefined
  track: string
  locationFilter?: LocationFilter
  /** REQUEST_ERROR code and reason, when the request was rejected. */
  error?: string
  /** When it was sent. */
  at: number
}

export interface ObjectRecord {
  /** Track name; `?` when the stream's request is unknown. */
  track: string
  /** The request the object answers, once known. */
  requestId: bigint | undefined
  stream: 'subgroup' | 'fetch'
  group: bigint
  object: bigint
  at: number
}

export interface GapRecord {
  /** `ordering`: the client skipped objects (`orderinggap`); `jump`: the sink jumped a hole in the buffer. */
  kind: 'ordering' | 'jump'
  detail: string
  at: number
}

export interface SwitchRecord {
  from: string
  /** The rendition asked for. */
  to: string
  /**
   * The track the switch requested first: `to`, or in a fast switching set the track of `to`'s
   * ladder whose switch point came first, which then climbs to its longest GoP.
   */
  entered: string
  /** From `switch()` to its `renditionchange`: the first packet written. */
  ms: number
  /** From `switch()` until the element shows the target's height. */
  shownMs: number
  /** `currentTime` when `switch()` was called. */
  playhead: number
  at: number
}

/** Everything recorded, appended to as the session runs. */
export interface Log {
  states: StateRecord[]
  events: EventRecord[]
  requests: RequestRecord[]
  data: (DataEvent & { at: number })[]
  objects: ObjectRecord[]
  gaps: GapRecord[]
  switches: SwitchRecord[]
}

/** `video` and `audio` match the tracks of the media source with that role; anything else is a track name. */
export type Who = 'video' | 'audio' | (string & {})

/** A read-only view of a session's log from `from` on. */
export class Records {
  constructor(
    protected readonly log: Log,
    readonly from: number,
    /** The role of a track name, or undefined. */
    protected readonly roleOf: (track: string) => 'video' | 'audio' | undefined,
    readonly onlyNewRequests = false,
  ) {}

  /**
   * This view with only the objects of requests made in it. After a seek or a switch, the
   * objects of the requests before it keep arriving for a while; this leaves them out.
   */
  newRequests(): Records {
    return new Records(this.log, this.from, this.roleOf, true)
  }

  get states(): StateRecord[] {
    return this.#since(this.log.states)
  }
  get events(): EventRecord[] {
    return this.#since(this.log.events)
  }
  get requests(): RequestRecord[] {
    return this.#since(this.log.requests)
  }
  /** Every client `dataevent`, raw. */
  get data(): (DataEvent & { at: number })[] {
    return this.#since(this.log.data)
  }
  get objects(): ObjectRecord[] {
    const objects = this.#since(this.log.objects)
    if (!this.onlyNewRequests) return objects
    const ids = new Set(this.requests.map((r) => r.requestId))
    return objects.filter((o) => o.requestId !== undefined && ids.has(o.requestId))
  }
  get gaps(): GapRecord[] {
    return this.#since(this.log.gaps)
  }
  get switches(): SwitchRecord[] {
    return this.#since(this.log.switches)
  }

  /** The recorded emissions of one player event. */
  eventsOf<K extends keyof PlayerEvents>(name: K): EventRecord<K>[] {
    return this.events.filter((e): e is EventRecord<K> => e.name === name)
  }

  /** Objects of the tracks `who` names. */
  objectsOf(who: Who): ObjectRecord[] {
    return this.objects.filter((o) => this.#matches(o.track, who))
  }

  /** Requests for the tracks `who` names. */
  requestsOf(who: Who): RequestRecord[] {
    return this.requests.filter((r) => this.#matches(r.track, who))
  }

  /** Distinct groups received, ascending. */
  groups(who: Who): bigint[] {
    return [...new Set(this.objectsOf(who).map((o) => o.group))].sort((a, b) => (a < b ? -1 : a > b ? 1 : 0))
  }

  /** The group of the first object received. */
  firstGroup(who: Who): bigint | undefined {
    return this.objectsOf(who)[0]?.group
  }

  /** The highest group received. */
  newestGroup(who: Who): bigint | undefined {
    return this.groups(who).at(-1)
  }

  /** `connecting > ready > playing`. */
  stateTrace(): string {
    return this.states.map((s) => s.state).join(' > ')
  }

  /** No stall, no error, no gap. */
  expectSmooth(): void {
    const problems = [
      ...this.states.filter((s) => s.state === 'stalled' || s.state === 'error').map(describeState(this.from)),
      ...this.gaps.map((g) => `${seconds(g.at - this.from)} gap (${g.kind}) ${g.detail}`),
    ]
    expect(problems, `playback since ${seconds(this.from)} was not smooth`).toEqual([])
  }

  /** Exactly these groups of `who` were received. */
  expectGroups(who: Who, groups: (bigint | number)[]): void {
    expect(this.groups(who), `groups of ${who}`).toEqual(groups.map(BigInt))
  }

  #matches(track: string, who: Who): boolean {
    return track === who || ((who === 'video' || who === 'audio') && this.roleOf(track) === who)
  }

  #since<T extends { at: number }>(entries: T[]): T[] {
    return entries.filter((e) => e.at >= this.from)
  }
}

export const seconds = (ms: number) => `${(ms / 1000).toFixed(2)}s`

const describeState = (from: number) => (s: StateRecord) => `${seconds(s.at - from)} ${s.previous} > ${s.state}`

/** A location as `group.object`. */
export const at = (location: Location) => `${location.group}.${location.object}`
