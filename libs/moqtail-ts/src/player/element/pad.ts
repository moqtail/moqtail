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
 * destination's `receive()`, events to the destination pad's `onEvent` listeners. Signals
 * bypass connections and reach the peer pad's `onSignal` listeners immediately.
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
  /** Seconds: the catalog's `targetLatency`, else `buffers.target`. */
  targetLatency?: number
  /** Seconds: the catalog's `maxGroupDuration`, else the observed group duration. */
  groupDuration?: number
  /** Seconds: the media timeline's range, after `accessibleWindowTime`. */
  seekable?: { start: number; end: number }
}

/** @public Written to a source pad, delivered in order with the packets around it. */
export type PadEvent =
  { type: 'discontinuity'; reason: 'seek' | 'gap'; mediaTime?: number; failed?: boolean } | { type: 'eos' }

/** @public Sent with `signal()`, delivered immediately ahead of anything written. */
export type PadSignal =
  | { type: 'seek'; mediaTime: number } // upstream
  | { type: 'flush' } // downstream

/** @public Items a connection holds before `write()` waits. */
export const CONNECTION_CAPACITY = 32

/** A transform, a sink, or a source taking a pad of an element it loaded. */
export type Consumer = BaseElement & { receive(pad: Pad, item: PadItem): void | Promise<void> }
type ErrorHandler = (error: unknown, destination: Pad) => void

/** @public One end of a connection between two elements. */
export class Pad {
  /** Connected pads: sink pads for a source pad, source pads for a sink pad. */
  readonly peers: Pad[] = []
  readonly #eventListeners = new Set<(event: PadEvent) => boolean>()
  readonly #signalListeners = new Set<(signal: PadSignal) => boolean>()
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
    if (this.#routed) {
      await this.#routed
      return this.write(item)
    }
    await Promise.all(Array.from(this.#connections.values(), (c) => c.push(item)))
  }

  /**
   * Delivers `signal` immediately to the connected pads: upstream from a sink pad, downstream
   * from a source pad, ahead of anything written. An element that doesn't handle it passes it
   * on through its other pads. Returns whether any element handled it.
   */
  signal(signal: PadSignal): boolean {
    let handled = false
    for (const peer of this.peers) if (peer.#deliverSignal(signal)) handled = true
    return handled
  }

  /** Sink pads only. Return true when the event is handled; false forwards it past this pad's element. */
  onEvent(listener: (event: PadEvent) => boolean): () => void {
    this.#eventListeners.add(listener)
    return () => this.#eventListeners.delete(listener)
  }

  /** Return true when the signal is handled; false forwards it past this pad's element. */
  onSignal(listener: (signal: PadSignal) => boolean): () => void {
    this.#signalListeners.add(listener)
    return () => this.#signalListeners.delete(listener)
  }

  /** @internal Delivers an event; unhandled ones land behind what the element already wrote. */
  async deliverEvent(event: PadEvent): Promise<void> {
    for (const listener of this.#eventListeners) if (listener(event)) return
    await Promise.all(this.#otherSide().map((pad) => pad.write({ type: 'event', event })))
  }

  /** @internal Holds writes until the returned function is called, once the pad is routed. */
  holdWrites(): () => void {
    const { promise, resolve } = Promise.withResolvers<void>()
    // Registered first, so it clears the hold before the held writes run, keeping their order
    this.#routed = promise.then(() => {
      this.#routed = undefined
    })
    return resolve
  }

  /** @internal Connects this source pad to a new sink pad on `element`, which then emits `padadded`. */
  connect(element: Consumer, onError: ErrorHandler): void {
    const sink = new Pad('sink', element, this.capabilities)
    element.pads.push(sink)
    this.#connections.set(sink, new Connection(sink, onError))
    this.peers.push(sink)
    sink.peers.push(this)
    element.emit('padadded', sink)
  }

  /** @internal */
  unlink(peer: Pad): void {
    const [source, sink] = this.direction === 'src' ? [this, peer] : [peer, this]
    source.#connections.get(sink)?.close()
    source.#connections.delete(sink)
    remove(source.peers, sink)
    remove(sink.peers, source)
  }

  #otherSide(): Pad[] {
    return this.element.pads.filter((pad) => pad.direction !== this.direction)
  }

  #deliverSignal(signal: PadSignal): boolean {
    for (const listener of this.#signalListeners) if (listener(signal)) return true
    let handled = false
    for (const pad of this.#otherSide()) if (pad.signal(signal)) handled = true
    return handled
  }
}

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
  readonly #destination: Pad
  readonly #onError: ErrorHandler
  readonly #queue: PadOutput[] = []
  /** Producers waiting for room, oldest first. */
  readonly #waiting: (() => void)[] = []
  #wake: (() => void) | undefined
  #closed = false

  constructor(destination: Pad, onError: ErrorHandler) {
    this.#destination = destination
    this.#onError = onError
    void this.#run()
  }

  push(item: PadOutput): Promise<void> {
    if (this.#closed) return Promise.resolve()
    this.#queue.push(item)
    this.#wake?.()
    if (this.#queue.length < CONNECTION_CAPACITY) return Promise.resolve()
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
      if (this.#queue.length < CONNECTION_CAPACITY) this.#waiting.shift()?.()
      try {
        if (item.type === 'event') await this.#destination.deliverEvent(item.event)
        else await (this.#destination.element as Consumer).receive(this.#destination, item)
      } catch (error) {
        this.#onError(error, this.#destination)
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
      // One item is inside receive(), the queue holds the rest up to capacity
      expect(resolved).toBe(CONNECTION_CAPACITY)
      await chain.destroy()
      await Promise.all(writes)
      expect(resolved).toBe(CONNECTION_CAPACITY + 8)
      blocked.resolve()
      await flush()
      expect(sink.items()).toEqual([])
    })

    test('events cross a transform downstream and signals both ways when nobody handles them', async () => {
      const { chain } = testChain()
      const transform = new TestTransform(detachedContext())
      let source: import('@/player/testing/elements').TestSource | undefined
      let sink: import('@/player/testing/elements').CollectingSink | undefined
      chain.addSource(testSource({ script: (s) => ((source = s), s.addPad(), s.emit('nomorepads')) }), {})
      chain.addTransform(transform)
      chain.addSink(collectingSink({ setup: (k) => (sink = k) }))
      await chain.init(new AbortController().signal)

      const upstream: PadSignal[] = []
      source!.pads[0]!.onSignal((signal) => (upstream.push(signal), true))
      await source!.pads[0]!.write(packet(1))
      await source!.pads[0]!.write(eos)
      await flush()
      await flush()
      expect(sink!.pads[0]!.peers[0]!.element).toBe(transform)
      expect(sink!.items()).toEqual([packet(1), eos])

      expect(sink!.pads[0]!.signal({ type: 'seek', mediaTime: 5 })).toBe(true)
      expect(upstream).toEqual([{ type: 'seek', mediaTime: 5 }])

      // A signal skips what's written: it reaches the sink before the packet the source can't push yet
      sink!.slow = () => new Promise(() => {})
      void source!.pads[0]!.write(packet(2))
      expect(source!.pads[0]!.signal({ type: 'flush' })).toBe(true)
      expect(sink!.signals).toEqual([{ type: 'flush' }])
      expect(sink!.items()).toEqual([packet(1), eos])
      sink!.consumeSignals = false
      expect(source!.pads[0]!.signal({ type: 'flush' })).toBe(false)
    })
  })
}
