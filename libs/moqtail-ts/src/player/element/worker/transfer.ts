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
 * How pad items and player errors cross threads: Comlink transfer handlers, registered on both
 * sides. A structured clone drops classes, so the handlers rebuild an item's `Location` and
 * `KeyValuePair`s and a `PlayerError`. Media moves: a packet's payload buffer and a frame's
 * `VideoFrame` or `AudioData` are transferred, and the sender loses them.
 */

import * as Comlink from 'comlink'
import { Location } from '@/model/common/location'
import { KeyValuePair } from '@/model/common/pair'
import type { Frame, Packet, Pad, PadItem } from '@/player/element/pad'
import { PlayerError, type PlayerErrorCode } from '@/player/error'
import { errorMessage } from '@/util/error'

type Media = { type: 'packet'; packet: Packet } | { type: 'frame'; frame: Frame }

interface WirePlayerError {
  code: PlayerErrorCode
  message: string
  element: string | undefined
  cause: unknown
}

const MEDIA_HANDLER = 'moqtail.media'
const ERROR_HANDLER = 'moqtail.error'

/** Each thread registers them before any call crosses. */
export function registerTransferHandlers(): void {
  if (Comlink.transferHandlers.has(MEDIA_HANDLER)) return

  Comlink.transferHandlers.set(MEDIA_HANDLER, {
    canHandle: (value): value is Media => isMedia(value),
    serialize: (item: Media) => serializedMedia(item),
    deserialize: (item: Media) => revived(item),
  })
  Comlink.transferHandlers.set(ERROR_HANDLER, {
    canHandle: (value): value is PlayerError => value instanceof PlayerError,
    serialize: (error: PlayerError) => [wired(error), []],
    deserialize: (wire: WirePlayerError) => unwired(wire),
  })

  // Comlink's own keeps only `name`, `message` and `stack` of what an exposed method throws
  const thrown = Comlink.transferHandlers.get('throw')!
  Comlink.transferHandlers.set('throw', {
    canHandle: (value): value is unknown => thrown.canHandle(value),
    serialize: (value: { value: unknown }) =>
      value.value instanceof PlayerError ? [{ playerError: wired(value.value) }, []] : thrown.serialize(value),
    deserialize: (value: { playerError?: WirePlayerError }) => {
      if (value.playerError) throw unwired(value.playerError)
      return thrown.deserialize(value)
    },
  })
}

/** `item`, its payload copied when `pad`'s source fans out: its connections share one `Packet`. */
export function forTransfer(item: PadItem, pad: Pad): PadItem {
  if (item.type !== 'packet' || (pad.peers[0]?.peers.length ?? 0) <= 1) return item
  return { type: 'packet', packet: { ...item.packet, payload: item.packet.payload.slice() } }
}

function isMedia(value: unknown): value is Media {
  if (typeof value !== 'object' || value === null || !('type' in value)) return false
  return (value.type === 'packet' && 'packet' in value) || (value.type === 'frame' && 'frame' in value)
}

/** A payload that's a view into a larger buffer, as a parsed object's usually is, is copied, not transferred. */
function serializedMedia(item: Media): [Media, Transferable[]] {
  if (item.type === 'frame') return [item, [item.frame.payload]]

  const { payload } = item.packet
  const owned = payload.byteOffset === 0 && payload.byteLength === payload.buffer.byteLength
  const sent = owned && payload.buffer instanceof ArrayBuffer ? payload : payload.slice()

  return [{ type: 'packet', packet: { ...item.packet, payload: sent } }, [sent.buffer as ArrayBuffer]]
}

function revived(item: Media): Media {
  const base = item.type === 'packet' ? item.packet : item.frame
  const location = new Location(base.location.group, base.location.object)
  const properties = base.properties.map(({ typeValue, value }) =>
    typeof value === 'bigint' ? KeyValuePair.newVarInt(typeValue, value) : KeyValuePair.newBytes(typeValue, value),
  )

  if (item.type === 'packet') return { type: 'packet', packet: { ...item.packet, location, properties } }
  return { type: 'frame', frame: { ...item.frame, location, properties } }
}

/** A cause that can't be cloned would fail the whole message, so only errors cross as they are. */
function wired(error: PlayerError): WirePlayerError {
  const { cause } = error
  return {
    code: error.code,
    message: error.message,
    element: error.element,
    cause:
      cause instanceof PlayerError
        ? { playerError: wired(cause) }
        : cause === undefined || cause instanceof Error
          ? cause
          : errorMessage(cause),
  }
}

function unwired(wire: WirePlayerError): PlayerError {
  const { cause } = wire
  const nested = typeof cause === 'object' && cause !== null && 'playerError' in cause
  return new PlayerError(wire.code, wire.message, {
    ...(wire.element !== undefined && { element: wire.element }),
    ...(cause !== undefined && { cause: nested ? unwired(cause.playerError as WirePlayerError) : cause }),
  })
}

if (import.meta.vitest) {
  const { test, expect } = import.meta.vitest
  const { Timestamp } = await import('@/model/property/timestamp')
  registerTransferHandlers()

  async function roundTrip<T>(value: T): Promise<T> {
    const { port1, port2 } = new MessageChannel()
    Comlink.expose({ echo: (received: T) => received }, port1)
    const remote = Comlink.wrap<{ echo(value: T): T }>(port2)

    try {
      return (await remote.echo(value as Parameters<typeof remote.echo>[0])) as T
    } finally {
      port1.close()
      port2.close()
    }
  }
  const packet = (payload: Uint8Array): PadItem => ({
    type: 'packet',
    packet: {
      track: 'v1',
      location: new Location(3n, 4n),
      properties: [new Timestamp(5n).toKeyValuePair()],
      payload,
    },
  })

  test('a packet crosses with its classes, its own buffer transferred and a view into a larger one copied', async () => {
    const owned = new Uint8Array([1, 2, 3])
    const received = (await roundTrip(packet(owned))) as Extract<PadItem, { type: 'packet' }>

    expect(owned.byteLength).toBe(0)
    expect([...received.packet.payload]).toEqual([1, 2, 3])
    expect(received.packet.location).toBeInstanceOf(Location)
    expect(received.packet.location.equals(new Location(3n, 4n))).toBe(true)
    expect(received.packet.properties[0]).toBeInstanceOf(KeyValuePair)

    const stream = new Uint8Array([9, 1, 2, 9])
    const view = (await roundTrip(packet(stream.subarray(1, 3)))) as Extract<PadItem, { type: 'packet' }>

    expect([[...stream], [...view.packet.payload]]).toEqual([
      [9, 1, 2, 9],
      [1, 2],
    ])
  })

  test('a frame’s payload is transferred and its base copied', () => {
    const payload = {} as VideoFrame
    const frame: Media = {
      type: 'frame',
      frame: { track: 'v1', location: new Location(1n, 0n), properties: [], payload },
    }
    const [sent, transfer] = serializedMedia(frame)

    expect([sent, transfer]).toEqual([frame, [payload]])
  })

  test('a packet whose source fans out is copied before it crosses', () => {
    const sinkPad = (connections: number) => ({ peers: [{ peers: Array(connections) }] }) as unknown as Pad
    const item = packet(new Uint8Array([1]))
    expect(forTransfer(item, sinkPad(1))).toBe(item)

    const copied = forTransfer(item, sinkPad(2)) as Extract<PadItem, { type: 'packet' }>
    expect(copied.packet.payload).not.toBe((item as typeof copied).packet.payload)
    expect([...copied.packet.payload]).toEqual([1])
  })

  test('a PlayerError crosses as an argument and as a throw, with its code, category, element and cause', async () => {
    const inner = new PlayerError('MEDIA_FAILURE', 'decoder', { element: 'webcodecs', cause: new Error('corrupt') })
    const error = new PlayerError('MEDIA_FAILURE', 'Playout failed', { element: 'playout', cause: inner })

    const expected = {
      name: 'PlayerError',
      code: 'MEDIA_FAILURE',
      category: 'media',
      element: 'playout',
      cause: expect.objectContaining({ code: 'MEDIA_FAILURE', element: 'webcodecs' }),
    }

    const received = await roundTrip(error)

    expect(received).toBeInstanceOf(PlayerError)
    expect(received).toMatchObject(expected)
    expect((received.cause as PlayerError).cause).toMatchObject({ message: 'corrupt' })

    const { port1, port2 } = new MessageChannel()
    Comlink.expose(
      {
        fail: () => {
          throw error
        },
      },
      port1,
    )
    const remote = Comlink.wrap<{ fail(): void }>(port2)

    await expect(remote.fail()).rejects.toSatisfy((e) => e instanceof PlayerError && e.element === 'playout')
    await expect(remote.fail()).rejects.toMatchObject(expected)

    port1.close()
    port2.close()
  })
}
