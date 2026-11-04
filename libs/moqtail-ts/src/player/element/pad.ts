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
 * Pads and the connections between them. A connection is a bounded FIFO from a source
 * pad to one sink pad, drained by a single loop: packets and properties go to the
 * destination's `receive()`, events to the destination pad's `onEvent` listeners.
 */

import type { MoqtObject } from '@/model/data/object'
import type { BaseElement } from './base'
import type { Capabilities } from './define'

/** @public What an element receives on a sink pad. */
export type PadItem =
  { type: 'packet'; object: MoqtObject } | { type: 'properties'; properties: Partial<PadProperties> }

/** @public What an element can write to a source pad. */
export type PadOutput = PadItem | { type: 'event'; event: PadEvent }

/** @public Well-known pad properties. Elements add their own by augmenting this interface. */
export interface PadProperties {
  /** The active track of the switching set. */
  track?: string
  /** Including codec, e.g. `video/mp4; codecs="avc1.64001f"`. */
  mimeType?: string
  initData?: Uint8Array
  live?: boolean
  /** Seconds, for VOD `MediaSource.duration`. */
  duration?: number
}

/** @public Events travelling between pads. */
export type PadEvent =
  | { type: 'seek'; mediaTime: number } // upstream
  | { type: 'discontinuity'; reason: 'seek' | 'gap'; mediaTime?: number; failed?: boolean } // downstream
  | { type: 'eos' } // downstream

/** @public Items a connection holds before `write()` waits. */
export const CONNECTION_CAPACITY = 32

/** A transform, a sink, or a source taking a pad of an element it loaded. */
export type Consumer = BaseElement & { receive(pad: Pad, item: PadItem): void | Promise<void> }
type ErrorHandler = (error: unknown, destination: Pad) => void

/** @public One end of a connection between two elements. */
export class Pad {
  /** Connected pads: sink pads for a source pad, source pads for a sink pad. */
  readonly peers: Pad[] = []
  readonly #listeners = new Set<(event: PadEvent) => boolean>()
  /** Source pads only, keyed by destination pad. */
  readonly #connections = new Map<Pad, Connection>()
  /** Set while the chain hasn't routed this pad yet, so writes made during `init()` aren't lost. */
  #routed: Promise<void> | undefined

  constructor(
    readonly direction: 'src' | 'sink',
    readonly element: BaseElement,
    readonly capabilities: Capabilities,
  ) {}

  /** Source pads only. Resolves once every connection has room, await it to apply backpressure. */
  async write(item: PadOutput): Promise<void> {
    if (this.direction !== 'src') throw new TypeError('write() is only for source pads')
    if (this.#routed) return this.#routed.then(() => this.write(item))
    return Promise.all(Array.from(this.#connections.values(), (c) => c.push(item))).then(() => undefined)
  }

  /** Sink pads only. Delivers an upstream event immediately to the connected source pads. */
  sendEvent(event: PadEvent): void {
    if (this.direction !== 'sink') throw new TypeError('sendEvent() is only for sink pads')
    for (const peer of this.peers) peer.#deliverUpstream(event)
  }

  /** Return true when the event is handled; false forwards it past this pad's element. */
  onEvent(listener: (event: PadEvent) => boolean): () => void {
    this.#listeners.add(listener)
    return () => this.#listeners.delete(listener)
  }

  #handled(event: PadEvent): boolean {
    for (const listener of this.#listeners) if (listener(event)) return true
    return false
  }

  #otherSide(): Pad[] {
    return this.element.pads.filter((pad) => pad.direction !== this.direction)
  }

  #deliverUpstream(event: PadEvent): void {
    if (this.#handled(event)) return
    for (const pad of this.#otherSide()) pad.sendEvent(event)
  }

  /** @internal Delivers a downstream event; unhandled ones land behind what the element already wrote. */
  async deliverDownstream(event: PadEvent): Promise<void> {
    if (this.#handled(event)) return
    await Promise.all(this.#otherSide().map((pad) => pad.write({ type: 'event', event })))
  }

  /** @internal Holds writes until the returned function is called, once the pad is routed. */
  holdWrites(): () => void {
    const { promise, resolve } = Promise.withResolvers<void>()
    // Registered first, so it clears the hold before the held writes run, keeping their order.
    this.#routed = promise.then(() => {
      this.#routed = undefined
    })
    return resolve
  }

  /** @internal Connects this source pad to a new sink pad on `element`, which then emits `padadded`. */
  connect(element: Consumer, onError: ErrorHandler, capacity?: number): Pad {
    const sink = new Pad('sink', element, this.capabilities)
    element.pads.push(sink)
    this.#connections.set(sink, new Connection(sink, onError, capacity))
    this.peers.push(sink)
    sink.peers.push(this)
    element.emit('padadded', sink)
    return sink
  }

  /** @internal Closes the connection between this pad and `peer`. */
  unlink(peer: Pad): void {
    const [source, sink] = this.direction === 'src' ? [this, peer] : [peer, this]
    source.#connections.get(sink)?.close()
    source.#connections.delete(sink)
    remove(source.peers, sink)
    remove(sink.peers, source)
  }
}

/** Adds a pad to its element and emits `padadded`. */
export function addPad(element: BaseElement, direction: 'src' | 'sink', capabilities: Capabilities): Pad {
  const pad = new Pad(direction, element, capabilities)
  element.pads.push(pad)
  element.emit('padadded', pad)
  return pad
}

/**
 * Removes a pad from its element, closes its connections, and emits `padremoved`.
 * Removing a source pad also removes the sink pads it fed, since they carry nothing anymore.
 */
export function removePad(pad: Pad, failed = false): void {
  if (!remove(pad.element.pads, pad)) return
  for (const peer of [...pad.peers]) {
    pad.unlink(peer)
    if (pad.direction === 'src') removePad(peer, failed)
  }
  pad.element.emit('padremoved', pad, failed)
}

function remove<T>(list: T[], item: T): boolean {
  const index = list.indexOf(item)
  if (index !== -1) list.splice(index, 1)
  return index !== -1
}

class Connection {
  readonly #queue: PadOutput[] = []
  /** Producers waiting for room, oldest first. */
  readonly #waiting: (() => void)[] = []
  #wake: (() => void) | undefined
  #closed = false

  constructor(
    readonly destination: Pad,
    readonly onError: ErrorHandler,
    readonly capacity = CONNECTION_CAPACITY,
  ) {
    void this.#run()
  }

  push(item: PadOutput): Promise<void> {
    if (this.#closed) return Promise.resolve()
    this.#queue.push(item)
    this.#wake?.()
    if (this.#queue.length < this.capacity) return Promise.resolve()
    return new Promise((resolve) => this.#waiting.push(resolve))
  }

  /** Drops the queue, ends the loop, and releases waiting producers so they don't hang. */
  close(): void {
    this.#closed = true
    this.#queue.length = 0
    for (const resolve of this.#waiting.splice(0)) resolve()
    this.#wake?.()
  }

  async #run(): Promise<void> {
    for (;;) {
      const item = this.#queue.shift() ?? (await this.#next())
      if (!item || this.#closed) return
      if (this.#queue.length < this.capacity) this.#waiting.shift()?.()
      try {
        if (item.type === 'event') await this.destination.deliverDownstream(item.event)
        else await (this.destination.element as Consumer).receive(this.destination, item)
      } catch (error) {
        this.onError(error, this.destination)
      }
    }
  }

  #next(): Promise<PadOutput | undefined> {
    if (this.#closed) return Promise.resolve(undefined)
    return new Promise((resolve) => {
      this.#wake = () => {
        this.#wake = undefined
        resolve(this.#queue.shift())
      }
    })
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest
  const { connected, flush, packet, testChain, testSource, collectingSink, TestTransform, detachedContext } =
    await import('@/player/testing/elements')
  const properties: PadItem = { type: 'properties', properties: { track: 'v1' } }
  const eos: PadOutput = { type: 'event', event: { type: 'eos' } }
  /** A promise the test resolves by hand. */
  const gate = () => Promise.withResolvers<void>()

  describe('Pad', () => {
    test('packets, properties, and events arrive in order, events only after the previous receive()', async () => {
      const { pad, sink } = await connected((s) => (s.slow = flush))
      void pad.write(properties)
      void pad.write(packet(1))
      void pad.write(eos)
      await pad.write(packet(2))
      await flush()
      await flush()
      await flush()
      expect(sink.items()).toEqual([properties, packet(1), eos, packet(2)])
    })

    test('a slow pad does not block another pad of the same element', async () => {
      const blocked = gate()
      const { source, sink, pad: slowPad } = await connected()
      const fastPad = source.addPad()
      sink.slow = (pad) => (pad === sink.pads[0] ? blocked.promise : Promise.resolve())
      void slowPad.write(packet(1))
      await fastPad.write(packet(2))
      await flush()
      expect(sink.items(sink.pads[1])).toEqual([packet(2)])
      expect(sink.items(sink.pads[0])).toEqual([])
      blocked.resolve()
      await flush()
      expect(sink.items(sink.pads[0])).toEqual([packet(1)])
    })

    test('write() waits once the connection is at capacity, and teardown releases it', async () => {
      const blocked = gate()
      const { chain, pad, sink } = await connected((s) => (s.slow = () => blocked.promise))
      let resolved = 0
      const writes = Array.from({ length: CONNECTION_CAPACITY + 8 }, (_, i) =>
        pad.write(packet(i)).then(() => resolved++),
      )
      await flush()
      // One item is inside receive(), the queue holds the rest up to capacity.
      expect(resolved).toBe(CONNECTION_CAPACITY)
      await chain.destroy()
      await Promise.all(writes)
      expect(resolved).toBe(CONNECTION_CAPACITY + 8)
      blocked.resolve()
      await flush()
      expect(sink.items()).toEqual([])
    })

    test('events cross a transform both ways when nobody handles them', async () => {
      const { chain } = testChain()
      const transform = new TestTransform(detachedContext())
      let source: import('@/player/testing/elements').TestSource | undefined
      let sink: import('@/player/testing/elements').CollectingSink | undefined
      chain.addSource(testSource({ script: (s) => ((source = s), s.addPad(), s.emit('nomorepads')) }), {})
      chain.addTransform(transform)
      chain.addSink(collectingSink({ setup: (k) => (sink = k) }))
      await chain.init(new AbortController().signal)

      const upstream: PadEvent[] = []
      source!.pads[0]!.onEvent((event) => (upstream.push(event), true))
      await source!.pads[0]!.write(packet(1))
      await source!.pads[0]!.write(eos)
      await flush()
      await flush()
      expect(sink!.pads[0]!.peers[0]!.element).toBe(transform)
      expect(sink!.items()).toEqual([packet(1), eos])

      sink!.pads[0]!.sendEvent({ type: 'seek', mediaTime: 5 })
      expect(upstream).toEqual([{ type: 'seek', mediaTime: 5 }])
    })
  })
}
