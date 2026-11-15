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

import type { MOQtailClient } from '@/client/client'
import type { DataEvent, DataHeaderEvent } from '@/client/data_event'
import type { MOQtailPlayer } from '@/player/player'

const decoder = new TextDecoder()

/**
 * @public
 * Runs `attach` on the player's current client and on every later one (reconnects replace it),
 * calling the cleanup `attach` returned for the one before. Returns the function that stops.
 */
export function eachClient(player: MOQtailPlayer, attach: (client: MOQtailClient) => () => void): () => void {
  let detach: (() => void) | undefined
  const use = (client: MOQtailClient) => {
    detach?.()
    detach = attach(client)
  }
  if (player.client) use(player.client)
  player.on('client', use)
  return () => {
    player.off('client', use)
    detach?.()
  }
}

/**
 * @public
 * Calls `listener` with each incoming data event of the player's client, across reconnects, with
 * the name of the track its stream carries and the stream's header. A stream can open before the
 * response naming its track, so `track` stays undefined until a later event of that stream.
 * Streams opened before the call are skipped. Returns the function that stops listening.
 */
export function followData(
  player: MOQtailPlayer,
  listener: (event: DataEvent, track: string | undefined, header: DataHeaderEvent) => void,
): () => void {
  const streams = new Map<number, { header: DataHeaderEvent; track?: string }>()
  let client: MOQtailClient | undefined

  const onData = (event: DataEvent) => {
    if (event.direction !== 'in') return
    if (event.kind === 'header') streams.set(event.streamId, { header: event })
    const stream = streams.get(event.streamId)
    if (!stream) return
    if (event.kind === 'end') streams.delete(event.streamId)
    if (stream.track === undefined) {
      const name = client?.trackNameOf(stream.header)
      if (name) stream.track = decoder.decode(name.name)
    }
    listener(event, stream.track, stream.header)
  }

  return eachClient(player, (next) => {
    client = next
    streams.clear()
    next.on('dataevent', onData)
    return () => next.off('dataevent', onData)
  })
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest
  const { Emitter } = await import('@/util/emitter')
  const { FullTrackName } = await import('@/model/data/full_track_name')

  function fakeClient() {
    return Object.assign(new Emitter<{ dataevent: (e: DataEvent) => void }>(), {
      trackNameOf: () => FullTrackName.tryNew('ns', 'video'),
    })
  }
  const header = (streamId: number): DataHeaderEvent => ({
    kind: 'header',
    stream: 'fetch',
    requestId: 4n,
    direction: 'in',
    streamId,
    at: 0,
  })
  const object = (streamId: number): DataEvent => ({
    kind: 'object',
    groupId: 1n,
    objectId: 0n,
    payloadLength: 10,
    direction: 'in',
    streamId,
    at: 1,
  })

  describe('followData', () => {
    test('names each stream, hands over its header, and moves to a new client', () => {
      const first = fakeClient()
      const player = Object.assign(new Emitter<{ client: (c: unknown) => void }>(), { client: first as unknown })
      const seen: [string, string | undefined, bigint | undefined][] = []
      const stop = followData(player as unknown as MOQtailPlayer, (e, track, h) => {
        seen.push([e.kind, track, h.requestId])
      })
      first.emit('dataevent', object(1)) // stream opened before following: skipped
      first.emit('dataevent', header(2))
      first.emit('dataevent', object(2))
      const second = fakeClient()
      player.client = second
      player.emit('client', second)
      first.emit('dataevent', header(3)) // the replaced client is no longer followed
      second.emit('dataevent', header(5))
      stop()
      second.emit('dataevent', object(5))
      expect(seen).toEqual([
        ['header', 'video', 4n],
        ['object', 'video', 4n],
        ['header', 'video', 4n],
      ])
    })
  })
}
