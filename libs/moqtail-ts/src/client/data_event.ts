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

import { FetchObject, SubgroupObject } from '../model/data'
import { ObjectStatus } from '../model/data/constant'
import { FullTrackName } from '../model/data/full_track_name'
import { Header } from '../model/data/header'
import { KeyValuePair } from '../model/common/pair'
import { Timestamp } from '../model/property/timestamp'
import { StreamResetCode } from '../model/error/stream_reset'
import { logger } from '../util/logger'

/** `in` for a stream the peer opened, `out` for one this client opened. */
export type DataDirection = 'in' | 'out'

/** What every data event carries. */
export type DataEventBase = {
  direction: DataDirection
  /** Local to this client, unique per data stream; every later event on the stream carries it. */
  streamId: number
  /**
   * When the client wrote or read it, as `performance.now()` in ms: high resolution and
   * monotonic, so differences between events are exact. Add `performance.timeOrigin` for
   * wall-clock time.
   */
  at: number
}

/** A data stream opened: what its header says. */
export type DataHeaderEvent = DataEventBase & {
  kind: 'header'
  stream: 'subgroup' | 'fetch'
  /** Subgroup streams only. */
  trackAlias?: bigint
  /** Subgroup streams only. */
  groupId?: bigint
  /** Subgroup streams only; unknown here when the header defers it to the first Object ID. */
  subgroupId?: bigint
  /** Fetch streams only. */
  requestId?: bigint
  /** The track the stream belongs to, when the client knows it at header time. */
  fullTrackName?: FullTrackName
}

/** One object written or read. Only its identity and size are reported, never its payload. */
export type DataObjectEvent = DataEventBase & {
  kind: 'object'
  groupId: bigint
  subgroupId?: bigint
  objectId: bigint
  payloadLength: number
  /** Set for an object that carries a status instead of a payload. */
  objectStatus?: ObjectStatus
  /**
   * The object's LOC Timestamp property, when it carries one: the sender's media time,
   * in whatever unit and clock the sender chose.
   */
  locTimestamp?: bigint
}

/** A data stream ended: `fin` when it closed cleanly, `reset` when either side reset it. */
export type DataStreamEndEvent = DataEventBase & {
  kind: 'end'
  reason: 'fin' | 'reset' | 'error'
  resetCode?: StreamResetCode
}

export type DataEvent = DataHeaderEvent | DataObjectEvent | DataStreamEndEvent

export type DataEventObserver = (event: DataEvent) => void

/** What a data stream is handed so it can report itself. */
export type DataStreamObserver = {
  onDataEvent: DataEventObserver
  /** Names the track a header belongs to, if known. */
  fullTrackNameOf?: (header: Header) => FullTrackName | undefined
}

let lastStreamId = 0

function locTimestampOf(properties: KeyValuePair[] | null): bigint | undefined {
  if (!properties) return undefined
  for (const pair of properties) {
    const timestamp = Timestamp.fromKeyValuePair(pair)
    if (timestamp) return timestamp.timestamp
  }
  return undefined
}

/**
 * Turns one data stream's activity into {@link DataEvent}s. A throwing observer is
 * logged and otherwise ignored, so it can never break the stream it watches.
 */
export class DataStreamReporter {
  readonly streamId = ++lastStreamId
  #hasEnded = false

  constructor(
    private readonly direction: DataDirection,
    private readonly observer?: DataStreamObserver,
  ) {}

  header(header: Header): void {
    if (!this.observer) return
    const fullTrackName = this.observer.fullTrackNameOf?.(header)
    const event: DataHeaderEvent = Header.isFetch(header)
      ? { ...this.#base('header'), stream: 'fetch', requestId: header.requestId }
      : {
          ...this.#base('header'),
          stream: 'subgroup',
          trackAlias: header.trackAlias,
          groupId: header.groupId,
          ...(header.subgroupId !== undefined && { subgroupId: header.subgroupId }),
        }
    if (fullTrackName) event.fullTrackName = fullTrackName
    this.#emit(event)
  }

  /** `header` is the stream's own: a subgroup object takes its group and subgroup from it. */
  object(header: Header, object: SubgroupObject | FetchObject): void {
    if (!this.observer) return
    let event: DataObjectEvent
    if (object instanceof FetchObject) {
      // An end-of-range marker is a gap in the range, not an object.
      if (object.kind !== 'object') return
      event = {
        ...this.#base('object'),
        groupId: object.location.group,
        subgroupId: object.subgroupId,
        objectId: object.location.object,
        payloadLength: object.payload?.byteLength ?? 0,
      }
      const locTimestamp = locTimestampOf(object.properties)
      if (locTimestamp !== undefined) event.locTimestamp = locTimestamp
    } else {
      if (Header.isFetch(header)) return
      event = {
        ...this.#base('object'),
        groupId: header.groupId,
        objectId: object.objectId,
        payloadLength: object.payload?.byteLength ?? 0,
        ...(header.subgroupId !== undefined && { subgroupId: header.subgroupId }),
        ...(object.objectStatus !== null && { objectStatus: object.objectStatus }),
      }
      const locTimestamp = locTimestampOf(object.properties)
      if (locTimestamp !== undefined) event.locTimestamp = locTimestamp
    }
    this.#emit(event)
  }

  /** Reports how the stream ended. Only the first call counts. */
  end(reason: DataStreamEndEvent['reason'], resetCode?: StreamResetCode): void {
    if (this.#hasEnded) return
    this.#hasEnded = true
    if (!this.observer) return
    this.#emit({ ...this.#base('end'), reason, ...(resetCode !== undefined && { resetCode }) })
  }

  #base<K extends DataEvent['kind']>(kind: K) {
    return { kind, direction: this.direction, streamId: this.streamId, at: performance.now() }
  }

  #emit(event: DataEvent): void {
    try {
      this.observer!.onDataEvent(event)
    } catch (error) {
      logger.warn('data_event', 'onDataEvent threw', error)
    }
  }
}
