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

import type { Location } from '@/model/common/location'
import type { KeyValuePair } from '@/model/common/pair'
import type { MoqtObject } from '@/model/data/object'
import type { BaseElement } from './base'
import type { Capabilities } from './define'

/** @public An object as elements pass it on, without its media. */
export interface PacketBase {
  /** The `track` property the pad carries for the object's track. */
  readonly track: string
  readonly location: Location
  /** As received, empty when the object has none. */
  readonly properties: readonly KeyValuePair[]
}

/** @public The media of one object, as elements pass it on. */
export interface Packet extends PacketBase {
  readonly payload: Uint8Array
}

/** @public A decoded object. The receiver owns `payload` and closes it. */
export interface Frame extends PacketBase {
  readonly payload: VideoFrame | AudioData
}

/** @public What an element receives on a sink pad. */
export type PadItem =
  | { type: 'packet'; packet: Packet }
  | { type: 'frame'; frame: Frame }
  | { type: 'properties'; properties: Partial<PadProperties> }

/** @public What an element can write to a source pad. */
export type PadOutput = PadItem | { type: 'event'; event: PadEvent }

/** @public Well-known pad properties. Elements add their own by augmenting this interface. */
export interface PadProperties {
  /** The active track of the switching set. */
  track?: string
  /** Including codec, e.g. `video/mp4; codecs="avc1.64001f"`. */
  mimeType?: string
  /**
   * The WebCodecs decoder config of a track without a container, such as LOC (LOC 2.3.2.1, 2.3.3.1).
   * Its `description` is `initData`.
   */
  decoderConfig?: Omit<VideoDecoderConfig, 'description'> | Omit<AudioDecoderConfig, 'description'>
  /** The track's codec configuration: a CMAF Header, or the LOC VIDEO_CONFIG or AUDIO_CONFIG bytes. */
  initData?: Uint8Array
  /** Units per second of the track's LOC Timestamps (LOC 2.3.1.2). Absent or `undefined`, they're µs since the Unix epoch. */
  timescale?: number | undefined
  live?: boolean
  /** Seconds, for VOD `MediaSource.duration`. */
  duration?: number
  /** Seconds: the catalog's `targetLatency`, else `buffers.target`. */
  targetLatency?: number
  /** Seconds: the catalog's `maxGroupDuration`, else the observed group duration. */
  groupDuration?: number
  /** Seconds: the media timeline's range, after `accessibleWindowTime`. */
  seekable?: { start: number; end: number }
  /** Seconds: the catalog's `accessibleWindowTime` (MSF 5.2.46). Live only; absent means no DVR. */
  accessibleWindow?: number
}

/** @public Written to a source pad, delivered in order with the packets around it. */
export type PadEvent =
  { type: 'discontinuity'; reason: 'seek' | 'gap'; mediaTime?: number; failed?: boolean } | { type: 'eos' }

/** @public Sent with `signal()`, delivered immediately ahead of anything written. */
export type PadSignal =
  | { type: 'seek'; mediaTime: number } // upstream
  | { type: 'flush' } // downstream
  /** Downstream: the seconds of media written to the element upstream and not passed on yet, sent when they change. */
  | { type: 'held'; seconds: number }

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

  /**
   * Source pads only. Resolves once every connection has room, for a frame once each took it: await it to apply backpressure.
   * A frame goes to the first connection and a clone to each other one, and is closed without any.
   */
  async write(item: PadOutput): Promise<void> {
    if (this.direction !== 'src') throw new TypeError('write() is only for source pads')
    if (this.#routed) {
      await this.#routed
      return this.write(item)
    }
    const connections = [...this.#connections.values()]
    if (connections.length === 0) {
      closeFrame(item)
      return
    }
    await Promise.all(connections.map((c, i) => c.push(i === 0 ? item : cloned(item))))
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
    const deliver = (item: PadOutput) =>
      item.type === 'event' ? sink.deliverEvent(item.event) : element.receive(sink, item)
    this.#connections.set(sink, new Connection(deliver, (error) => onError(error, sink)))
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

/** The packet of `object`, carrying `payload`, which may be a rebuilt or decompressed one. */
export function packetOf(track: string, object: MoqtObject, payload: Uint8Array): Packet {
  return { track, location: object.location, properties: object.properties ?? [], payload }
}

export function cloned(item: PadOutput): PadOutput {
  if (item.type !== 'frame') return item
  return { type: 'frame', frame: { ...item.frame, payload: item.frame.payload.clone() } }
}

/** Drops a frame no consumer will receive, since only its receiver would close it. */
export function closeFrame(item: PadOutput): void {
  if (item.type === 'frame') item.frame.payload.close()
}

function remove<T>(list: T[], item: T): boolean {
  const index = list.indexOf(item)
  if (index !== -1) list.splice(index, 1)
  return index !== -1
}

export class Connection {
  readonly #deliver: (item: PadOutput) => void | Promise<void>
  readonly #onError: (error: unknown) => void
  readonly #queue: PadOutput[] = []
  /** Producers waiting for room, oldest first. */
  readonly #waiting: (() => void)[] = []
  /** Resolves the write of each queued frame once it's taken. */
  readonly #taken = new Map<PadOutput, () => void>()
  #wake: (() => void) | undefined
  #closed = false

  constructor(deliver: (item: PadOutput) => void | Promise<void>, onError: (error: unknown) => void) {
    this.#deliver = deliver
    this.#onError = onError
    void this.#run()
  }

  push(item: PadOutput): Promise<void> {
    if (this.#closed) {
      closeFrame(item)
      return Promise.resolve()
    }
    // A queued frame pins a decoder buffer, so its producer decodes no further until the consumer takes it
    const taken = item.type === 'frame' ? new Promise<void>((resolve) => this.#taken.set(item, resolve)) : undefined
    this.#queue.push(item)
    this.#wake?.()
    if (taken) return taken
    if (this.#queue.length < CONNECTION_CAPACITY) return Promise.resolve()
    return new Promise((resolve) => this.#waiting.push(resolve))
  }

  /** Drops the queue, ends the loop, and releases waiting producers so they don't hang. */
  close(): void {
    this.#closed = true
    for (const item of this.#queue.splice(0)) closeFrame(item)
    for (const resolve of this.#waiting.splice(0)) resolve()
    for (const resolve of this.#taken.values()) resolve()
    this.#taken.clear()
    this.#wake?.()
  }

  async #run(): Promise<void> {
    for (;;) {
      const item = this.#queue.shift() ?? (await this.#next())
      if (!item || this.#closed) return
      this.#taken.get(item)?.()
      this.#taken.delete(item)
      if (this.#queue.length < CONNECTION_CAPACITY) this.#waiting.shift()?.()
      try {
        await this.#deliver(item)
      } catch (error) {
        this.#onError(error)
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
  const { connected, flush, packet, testChain, testSource, collectingSink, testTransform } =
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

    test("a frame's write waits until the consumer takes it, and teardown releases it", async () => {
      const { Location } = await import('@/model/common/location')
      const blocked = gate()
      const { chain, pad } = await connected((s) => (s.slow = () => blocked.promise))
      const frame = (): PadOutput => ({
        type: 'frame',
        frame: { track: 'v1', location: new Location(0n, 0n), properties: [], payload: { close() {} } as VideoFrame },
      })
      const written: number[] = []
      for (const n of [1, 2]) void pad.write(frame()).then(() => written.push(n))
      await flush()
      // The first is inside receive(), the second waits to be taken
      expect(written).toEqual([1])
      await chain.destroy()
      await flush()
      expect(written).toEqual([1, 2])
    })

    test('events cross a transform downstream and signals both ways when nobody handles them', async () => {
      const { chain } = testChain()
      let transform: import('@/player/testing/elements').TestTransform | undefined
      let source: import('@/player/testing/elements').TestSource | undefined
      let sink: import('@/player/testing/elements').CollectingSink | undefined
      chain.addSource(testSource({ script: (s) => ((source = s), s.addPad(), s.emit('nomorepads')) }), {})
      chain.addTransform(testTransform({ setup: (t) => (transform = t) }))
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

    test('a frame goes to the first connection and a clone to the others, and is closed once dropped', async () => {
      const { Location } = await import('@/model/common/location')
      const { CapabilityCode } = await import('./define')
      const closed: string[] = []
      const fake = (name: string): Frame['payload'] =>
        ({ name, close: () => closed.push(name), clone: () => fake(`${name} clone`) }) as unknown as Frame['payload']
      const base = { track: 'v1', location: new Location(1n, 0n), properties: [] }
      const frame = (name: string): PadOutput => ({ type: 'frame', frame: { ...base, payload: fake(name) } })

      const sinks: import('@/player/testing/elements').CollectingSink[] = []
      let source: import('@/player/testing/elements').TestSource | undefined
      const { chain } = testChain()
      const script = (s: import('@/player/testing/elements').TestSource) => {
        source = s
        s.addPad()
        s.emit('nomorepads')
      }
      chain.addSource(testSource({ script }), {})
      chain.addSink(collectingSink({ setup: (k) => sinks.push(k) }))
      chain.addSink(collectingSink({ setup: (k) => sinks.push(k) }))
      await chain.init(new AbortController().signal)
      const pad = source!.pads[0]!

      const first = frame('a')
      await pad.write(first)
      await flush()
      expect(sinks[0]!.items()).toEqual([first])
      expect(sinks[1]!.items()).toEqual([
        { type: 'frame', frame: { ...base, payload: expect.objectContaining({ name: 'a clone' }) } },
      ])

      await source!
        .addPad([{ direction: 'out', code: CapabilityCode.PACKAGING, value: 'raw' }])
        .write(frame('unlinked'))
      const blocked = gate()
      for (const sink of sinks) sink.slow = () => blocked.promise
      void pad.write(frame('received'))
      void pad.write(frame('queued'))
      await flush()
      await chain.destroy()
      await pad.write(frame('late'))
      blocked.resolve()
      expect(closed.sort()).toEqual(['late', 'queued', 'queued clone', 'unlinked'])
    })
  })
}
