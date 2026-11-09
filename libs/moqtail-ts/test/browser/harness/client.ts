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
 * Records a client's requests, received objects, and ordering gaps into a session log. Wraps
 * the client's request methods, so each object can be traced to its request and track.
 */

import type { MOQtailClient } from '@/client/client'
import type { FetchOptions, SubscribeOptions, SwitchOptions } from '@/client/types'
import type { FullTrackName } from '@/model/data/full_track_name'
import { FillParameters } from '@/model/parameter/message/fill_parameters'
import type { LocationFilter } from '@/model/parameter/message/location_filter'
import { RequestError } from '@/model/control/request_error'
import { at, type Log, type RequestRecord } from './records'

const decode = (name: FullTrackName) => new TextDecoder().decode(name.name)

/** Records into `log` from now on. A client shared by two sessions records into both. */
export function recordClient(client: MOQtailClient, log: Log): void {
  const requests = new Map<bigint, RequestRecord>()
  const aliases = new Map<bigint, bigint>()
  const streams = new Map<number, () => RequestRecord | undefined>()
  const streamNames = new Map<number, string>()
  const streamKinds = new Map<number, 'subgroup' | 'fetch'>()
  const record =
    <A, R>(
      kind: RequestRecord['kind'],
      call: (args: A) => Promise<R>,
      options: (args: A) => SubscribeOptions | FetchOptions,
    ) =>
    async (args: A) => {
      const sent = performance.now()
      const result = await call.call(client, args)
      const { fullTrackName, locationFilter, parameters } = options(args)
      // A filled SUBSCRIBE starts where its FILL_PARAMETERS say.
      const fill = parameters?.find((p): p is FillParameters => p instanceof FillParameters)
      const start = (fill?.parameters[0] as LocationFilter | undefined) ?? locationFilter
      const entry: RequestRecord = {
        kind,
        requestId: undefined,
        track: decode(fullTrackName),
        ...(start && { locationFilter: start }),
        at: sent,
      }
      if (result instanceof RequestError) {
        entry.error = `${result.errorCode} ${result.reasonPhrase.phrase}`
      } else if (result && typeof result === 'object' && 'requestId' in result) {
        const { requestId } = result as { requestId: bigint }
        entry.requestId = requestId
        requests.set(requestId, entry)
        const ok = (result as { subscribeOk?: { trackAlias: bigint } }).subscribeOk
        if (ok) aliases.set(ok.trackAlias, requestId)
        // switch() sends its SUBSCRIBE through subscribe(), which was recorded already.
        const inner = kind === 'switch' ? log.requests.findIndex((r) => r.requestId === requestId) : -1
        if (inner !== -1) log.requests.splice(inner, 1)
      }
      log.requests.push(entry)
      return result
    }
  client.subscribe = record('subscribe', client.subscribe, (a: SubscribeOptions) => a)
  client.fetch = record('fetch', client.fetch, (a: FetchOptions) => a)
  client.switch = record('switch', client.switch, (a: SwitchOptions) => a.newSubscribeOptions)
  client.on('dataevent', (event) => {
    log.data.push(event)
    if (event.kind === 'header') {
      const { trackAlias, requestId, fullTrackName } = event
      // Resolved late: a stream can arrive before its request's response.
      streams.set(event.streamId, () => {
        const id = trackAlias !== undefined ? aliases.get(trackAlias) : requestId
        return id === undefined ? undefined : requests.get(id)
      })
      if (fullTrackName) streamNames.set(event.streamId, decode(fullTrackName))
      streamKinds.set(event.streamId, event.stream)
    } else if (event.kind === 'object') {
      const request = streams.get(event.streamId) ?? (() => undefined)
      const name = streamNames.get(event.streamId)
      log.objects.push({
        get track() {
          return request()?.track ?? name ?? '?'
        },
        get requestId() {
          return request()?.requestId
        },
        stream: streamKinds.get(event.streamId) ?? 'subgroup',
        group: event.groupId,
        object: event.objectId,
        at: event.at,
      })
    }
  })
  client.on('orderinggap', (requestId, from, to) => {
    const detail = `${requests.get(requestId)?.track ?? requestId} ${at(from)} to ${at(to)}`
    log.gaps.push({ kind: 'ordering', detail, at: performance.now() })
  })
}
