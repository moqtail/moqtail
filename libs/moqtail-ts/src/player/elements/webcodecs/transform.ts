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

import type { Remote } from 'comlink'
import { TransformElement } from '@/player/element/base'
import type { ElementContext } from '@/player/element/context'
import { CapabilityCode } from '@/player/element/define'
import { convertedCapabilities } from '@/player/element/negotiation'
import type { Pad, PadItem, PadSignal } from '@/player/element/pad'
import { roleOf } from '@/player/element/sink/media_sink'
import { forTransfer } from '@/player/element/worker/transfer'
import { PlayerError } from '@/player/error'
import { Decoding, type Codecs } from './decoding'

/**
 * Decodes the one `loc` pad it takes into a `raw` pad of `VideoFrame`s or `AudioData`, in the
 * worker when it has the decoder for the pad's role.
 */
export class WebCodecsTransform extends TransformElement {
  readonly #codecs: Codecs
  #output: Pad | undefined
  #decoding: Decoding | Remote<Decoding> | undefined

  constructor(context: ElementContext, codecs: Codecs) {
    super(context)
    this.#codecs = codecs
    this.on('padadded', (pad) => {
      if (pad.direction === 'sink') this.#inputAdded(pad)
    })
    this.on('padremoved', (pad, failed) => {
      if (pad.direction === 'sink') this.#inputRemoved(failed)
    })
  }

  /** Chain#route negotiates and adds a pad synchronously, so a second one can't slip in. */
  negotiate(pad: Pad): boolean {
    return !this.pads.some((p) => p.direction === 'sink') && this.matchesCapabilities(pad)
  }

  async receive(pad: Pad, item: PadItem): Promise<void> {
    await this.#decoding?.receive(forTransfer(item, pad))
  }

  override async destroy(): Promise<void> {
    await this.#decoding?.close()
    await super.destroy()
  }

  #inputAdded(input: Pad): void {
    // Negotiated, so the definition converts it
    const capabilities = convertedCapabilities(this.definition.capabilities, input.capabilities)!
    const worker = this.context.worker
    const inWorker = worker?.has(roleOf(input) === 'audio' ? 'AudioDecoder' : 'VideoDecoder') ? worker : undefined
    const thread = inWorker ? [{ direction: 'out' as const, code: CapabilityCode.THREAD, value: 'worker' }] : []
    const output = this.createPad('src', [...capabilities, ...thread])
    const report = (report: PlayerError | PadSignal) => {
      if (report instanceof PlayerError) this.context.fail(report)
      else output.signal(report)
    }
    const decoding = inWorker
      ? inWorker.create<Decoding>('webcodecs', { outputs: [output], report })
      : new Decoding({ outputs: [(item) => output.write(item)], options: undefined, report }, this.#codecs)
    this.#output = output
    this.#decoding = decoding
    input.onEvent((event) => {
      void decoding.event(event)
      return true
    })
    // Upstream signals from the output need no listener: unhandled, they reach the input
    input.onSignal((signal) => {
      void decoding.signal(signal)
      return false
    })
  }

  /** `failed` carries over, so the chain's `NO_MEDIA_SINK` check still sees a failure. */
  #inputRemoved(failed: boolean): void {
    void this.#decoding?.close()
    if (this.#output) this.removePad(this.#output, failed)
  }
}

if (import.meta.vitest) {
  const { test, expect } = import.meta.vitest
  const { Location } = await import('@/model/common/location')
  const { Timescale } = await import('@/model/property/timescale')
  const { Timestamp } = await import('@/model/property/timestamp')
  const { VideoFrameMarking } = await import('@/model/property/video_frame_marking')
  const { defineElement } = await import('@/player/element/define')
  const { locCapabilities } = await import('@/player/elements/loc/track')
  const { collectingSink, settle, testChain, testSource } = await import('@/player/testing/elements')
  const { WebCodecs } = await import('./index')
  type Packet = import('@/player/element/pad').Packet
  type PadItem = import('@/player/element/pad').PadItem
  type PadOutput = import('@/player/element/pad').PadOutput
  type KeyValuePair = import('@/model/common/pair').KeyValuePair
  type CollectingSink = import('@/player/testing/elements').CollectingSink
  type TestSource = import('@/player/testing/elements').TestSource

  /** WebCodecs fakes that record what they're given. `manual` holds outputs until `emit()`. */
  function fakeCodecs(options: { supported?: boolean; manual?: boolean } = {}) {
    const configs: unknown[] = []
    const chunks: { type: string; timestamp: number; data: Uint8Array }[] = []
    const calls: string[] = []
    const closed: number[] = []
    const decoders: FakeDecoder[] = []
    class FakeDecoder extends EventTarget {
      state = 'unconfigured'
      decodeQueueSize = 0
      readonly #init: { output(payload: unknown): void; error(error: DOMException): void }

      constructor(init: { output(payload: unknown): void; error(error: DOMException): void }) {
        super()
        this.#init = init
        decoders.push(this)
      }

      static async isConfigSupported() {
        return { supported: options.supported ?? true }
      }

      configure(config: unknown) {
        configs.push(config)
        this.state = 'configured'
      }

      decode(chunk: { type: string; timestamp: number; data: Uint8Array }) {
        chunks.push(chunk)
        if (!options.manual) this.emit(chunk.timestamp)
      }

      async flush() {
        calls.push('flush')
      }

      reset() {
        calls.push('reset')
        this.state = 'unconfigured'
      }

      close() {
        this.state = 'closed'
      }

      emit(...timestamps: number[]) {
        for (const timestamp of timestamps) {
          const frame = {
            timestamp,
            format: 'I420' as string | null,
            close: () => {
              frame.format = null
              closed.push(timestamp)
            },
          }
          this.#init.output(frame)
        }
      }

      fail() {
        this.state = 'closed'
        this.#init.error(new DOMException('corrupt', 'EncodingError'))
      }
    }
    class Chunk {
      constructor(init: object) {
        Object.assign(this, init)
      }
    }
    const codecs = {
      VideoDecoder: class extends FakeDecoder {},
      AudioDecoder: class extends FakeDecoder {},
      EncodedVideoChunk: Chunk,
      EncodedAudioChunk: Chunk,
    } as unknown as Codecs
    return { codecs, configs, chunks, calls, closed, decoders }
  }

  /** A test source's `loc` pad through a `webcodecs` transform on fakes into a sink taking `raw` pads. */
  async function decoding(options: { supported?: boolean; manual?: boolean; role?: string } = {}) {
    const fakes = fakeCodecs(options)
    const { chain, failures, warnings } = testChain()
    let source: TestSource | undefined
    let sink: CollectingSink | undefined
    const role = options.role ?? 'video'
    const script = (s: TestSource) => {
      source = s
      s.addPad(locCapabilities('out', role))
      s.emit('nomorepads')
    }
    chain.addSource(testSource({ script }), {})
    let transform: WebCodecsTransform | undefined
    const definition = defineElement({
      ...WebCodecs,
      init: async (context) => {
        transform = new WebCodecsTransform(context, fakes.codecs)
        return transform
      },
    })
    chain.addTransform(definition)
    chain.addSink(collectingSink({ packaging: 'raw', setup: (k) => (sink = k) }))
    await chain.init(new AbortController().signal)
    const pad = source!.pads[0]!
    return {
      ...fakes,
      failures,
      warnings,
      transform: transform!,
      source: source!,
      sink: sink!,
      pad,
      async write(...items: PadOutput[]) {
        for (const item of items) await pad.write(item)
        await settle()
      },
      /** The sink's items in short: `props <keys>`, `<group>.<object> @<timestamp>`, events by type. */
      trace: () =>
        sink!.items().map((item) => {
          if (item.type === 'properties') return `props ${Object.keys(item.properties).sort().join(',')}`
          if (item.type === 'frame') {
            const { location, payload } = item.frame
            return `${location.group}.${location.object} @${payload.timestamp}`
          }
          return item.type === 'event' ? item.event.type : 'packet'
        }),
    }
  }

  const properties = (codec = 'av01.0.04M.08', extra = {}): PadItem => ({
    type: 'properties',
    properties: { track: 'v1', live: true, decoderConfig: { codec }, ...extra },
  })
  /** A packet stamped `timestamp` µs, with any further properties. */
  function packet(
    group: number,
    object: number,
    timestamp: number,
    ...more: { toKeyValuePair(): KeyValuePair }[]
  ): { type: 'packet'; packet: Packet } {
    const packet: Packet = {
      track: 'v1',
      location: new Location(BigInt(group), BigInt(object)),
      properties: [new Timestamp(BigInt(timestamp)), ...more].map((p) => p.toKeyValuePair()),
      payload: new Uint8Array([group, object]),
    }
    return { type: 'packet', packet }
  }

  test('properties configure the decoder once the config is supported and are passed on', async () => {
    const d = await decoding()
    await d.write(properties())
    expect(d.configs).toEqual([{ codec: 'av01.0.04M.08', optimizeForLatency: true }])
    expect(d.sink.items()).toEqual([properties()])
    // The same config again leaves the decoder as it is
    await d.write(properties())
    expect(d.configs).toHaveLength(1)

    const unsupported = await decoding({ supported: false })
    await unsupported.write(properties())
    expect(unsupported.failures).toMatchObject([{ code: 'MEDIA_FAILURE', element: 'webcodecs' }])
    expect(unsupported.sink.items()).toEqual([])
  })

  test('deltas before the first key frame and after a gap are dropped, key frames from frame marking, else object 0', async () => {
    const d = await decoding()
    const independent = new VideoFrameMarking(new Uint8Array([0x20]))
    await d.write(properties(), packet(0, 1, 1), packet(1, 0, 2), packet(1, 1, 3))
    await d.write({ type: 'event', event: { type: 'discontinuity', reason: 'gap' } })
    await d.write(packet(1, 2, 4), packet(2, 3, 5, independent), packet(2, 4, 6))
    expect(d.trace()).toEqual([
      'props decoderConfig,live,track',
      '1.0 @2',
      '1.1 @3',
      'discontinuity',
      '2.3 @5',
      '2.4 @6',
    ])
    expect(d.chunks.map((c) => c.type)).toEqual(['key', 'delta', 'key', 'delta'])
  })

  test('it takes one pad, and a signal from the output reaches the input', async () => {
    const d = await decoding()
    d.source.addPad(locCapabilities('out', 'video'))
    expect(d.transform.pads.filter((p) => p.direction === 'sink')).toHaveLength(1)
    expect(d.warnings).toMatchObject([{ code: 'PAD_UNLINKED' }])
    const upstream: unknown[] = []
    d.pad.onSignal((signal) => {
      upstream.push(signal)
      return true
    })
    expect(d.sink.pads[0]!.signal({ type: 'seek', mediaTime: 5 })).toBe(true)
    expect(upstream).toEqual([{ type: 'seek', mediaTime: 5 }])
  })

  test('eos flushes the decoder before it is passed on', async () => {
    const d = await decoding({ manual: true })
    await d.write(properties(), packet(1, 0, 1))
    // The fake outputs the held frame while it flushes
    d.decoders[0]!.flush = async () => {
      d.calls.push('flush')
      d.decoders[0]!.emit(1)
    }
    await d.write({ type: 'event', event: { type: 'eos' } })
    expect(d.calls).toEqual(['flush'])
    expect(d.trace()).toEqual(['props decoderConfig,live,track', '1.0 @1', 'eos'])
  })

  test('a downstream flush resets the decoder and waits for the next key frame', async () => {
    const d = await decoding()
    await d.write(properties(), packet(1, 0, 1))
    d.pad.signal({ type: 'flush' })
    await d.write(packet(1, 1, 2), packet(2, 0, 3))
    expect(d.calls).toEqual(['reset'])
    expect(d.configs).toHaveLength(2)
    expect(d.trace().slice(1)).toEqual(['1.0 @1', '2.0 @3'])
    expect(d.sink.signals).toEqual([{ type: 'flush' }])
  })

  test('receive() waits once 10 s are held, until a frame is taken', async () => {
    const { codecs } = fakeCodecs()
    const blocked = Promise.withResolvers<void>()
    const output = async (item: PadOutput) => {
      if (item.type === 'frame') await blocked.promise
    }
    const engine = new Decoding({ outputs: [output], options: undefined, report: () => {} }, codecs)
    await engine.receive(properties())
    await settle()
    for (let s = 0; s < 10; s++) await engine.receive(packet(1, s, s * 1_000_000))
    let returned = false
    const last = engine.receive(packet(1, 10, 10_000_000)).then(() => (returned = true))
    await settle()
    // The first frame isn't taken, so 0 s to 10 s are held
    expect(returned).toBe(false)
    blocked.resolve()
    await last
    engine.close()
  })

  test('packets wait encoded while the consumer holds a frame, signalled downstream as held, until a flush drops them', async () => {
    const d = await decoding()
    await d.write(properties())
    const blocked = Promise.withResolvers<void>()
    d.sink.slow = () => blocked.promise
    await d.write(...[0, 1, 2, 3, 4].map((n) => packet(1, n, n * 100_000)))
    // The first frame is inside receive(), the second waits to be taken
    expect(d.chunks).toHaveLength(2)
    expect(d.sink.signals.at(-1)).toEqual({ type: 'held', seconds: 0.3 })
    d.pad.signal({ type: 'flush' })
    expect(d.sink.signals.slice(-2)).toEqual([{ type: 'held', seconds: 0 }, { type: 'flush' }])
    blocked.resolve()
    await settle()
    expect(d.chunks).toHaveLength(2)
    expect(d.trace().slice(1)).toEqual(['1.0 @0', '1.1 @100000'])
  })

  test('an AAC track without AUDIO_CONFIG gets the built AudioSpecificConfig, and length-prefixed avc3 is decoded as Annex B', async () => {
    const audio = await decoding({ role: 'audio' })
    await audio.write({
      type: 'properties',
      properties: { decoderConfig: { codec: 'mp4a.40.2', sampleRate: 48_000, numberOfChannels: 2 } },
    })
    expect(audio.configs).toEqual([
      { codec: 'mp4a.40.2', sampleRate: 48_000, numberOfChannels: 2, description: new Uint8Array([0x11, 0x90]) },
    ])

    const video = await decoding()
    const nal = [0, 0, 0, 2, 0x65, 0x88]
    const item = packet(1, 0, 1)
    await video.write(properties('avc3.64001f'), {
      type: 'packet',
      packet: { ...item.packet, payload: new Uint8Array(nal) },
    })
    expect(video.chunks.map((c) => [...c.data])).toEqual([[0, 0, 0, 1, 0x65, 0x88]])
    // With a VIDEO_CONFIG description, payloads go as they are
    await video.write(properties('avc3.64001f', { initData: new Uint8Array([1]) }), {
      type: 'packet',
      packet: { ...item.packet, payload: new Uint8Array(nal) },
    })
    expect(video.chunks.map((c) => [...c.data])).toEqual([[0, 0, 0, 1, 0x65, 0x88], nal])
  })

  test('an object’s own TIMESCALE reaches the output as the timescale property before its frame', async () => {
    const d = await decoding()
    await d.write(properties('av01.0.04M.08', { timescale: 90_000 }))
    await d.write(packet(1, 0, 1000, new Timescale(1000n)), packet(1, 1, 90_000))
    expect(d.trace()).toEqual([
      'props decoderConfig,live,timescale,track',
      'props timescale',
      '1.0 @1000000',
      'props timescale',
      '1.1 @1000000',
    ])
    expect(
      d.sink
        .items()
        .filter((item) => item.type === 'properties')
        .map((item) => item.properties.timescale),
    ).toEqual([90_000, 1000, 90_000])
  })

  test('each frame carries its object’s track, location and properties, out of order or with a chunk skipped, until a flush', async () => {
    const d = await decoding({ manual: true })
    const objects = [packet(1, 0, 0), packet(1, 1, 2000), packet(1, 2, 1000), packet(1, 3, 1000)]
    await d.write(properties(), ...objects)
    // B-frames come out in presentation order, 2000 is skipped, 5000 is no chunk's
    d.decoders[0]!.emit(0, 1000, 5000, 1000)
    await d.write({ type: 'event', event: { type: 'eos' } })
    d.decoders[0]!.emit(2000)
    await settle()
    const frames = d.sink.items().flatMap((item) => (item.type === 'frame' ? [item.frame] : []))
    const { track, location, properties: objectProperties } = objects[2]!.packet
    expect(frames.map((f) => f.location.object)).toEqual([0n, 2n, 3n])
    expect(frames[1]).toMatchObject({ track, location, properties: objectProperties })
    expect(d.closed).toEqual([5000, 2000])
  })

  test('objects stamped before an output frame are dropped, since outputs come in presentation order', async () => {
    const d = await decoding({ manual: true })
    await d.write(properties(), packet(1, 0, 0), packet(1, 1, 1000), packet(1, 2, 2000))
    // 1000 is skipped, so its object is gone once 2000 is out
    d.decoders[0]!.emit(0, 2000, 1000)
    // Objects decoded after a stray late timestamp keep theirs
    await d.write(packet(2, 0, 9000), packet(2, 1, 3000))
    d.decoders[0]!.emit(9000, 3000)
    await settle()
    expect(d.trace().slice(1)).toEqual(['1.0 @0', '1.2 @2000', '2.0 @9000', '2.1 @3000'])
    expect(d.closed).toEqual([1000])
  })

  test('audio pairs by timestamp when the decoder keeps them, so a skipped chunk shifts nothing', async () => {
    const d = await decoding({ role: 'audio', manual: true })
    const decoderConfig = { codec: 'opus', sampleRate: 48_000, numberOfChannels: 2 }
    await d.write(
      { type: 'properties', properties: { track: 'a1', live: true, decoderConfig } },
      packet(1, 0, 0),
      packet(1, 1, 20_000),
      packet(1, 2, 40_000),
    )
    d.decoders[0]!.emit(0, 40_000)
    await settle()
    expect(d.trace().slice(1)).toEqual(['1.0 @0', '1.2 @40000'])
  })

  test('a decoder error fails with MEDIA_FAILURE at once, naming the frames still open', async () => {
    const d = await decoding()
    await d.write(
      properties(),
      packet(1, 0, 0),
      packet(1, 1, 500_000),
      packet(1, 2, 1_000_000),
      packet(1, 3, 1_500_000),
    )
    const [first] = d.sink.items().flatMap((item) => (item.type === 'frame' ? [item.frame] : []))
    first!.payload.close()
    d.decoders[0]!.fail()
    expect(d.failures).toMatchObject([{ code: 'MEDIA_FAILURE', element: 'webcodecs' }])
    expect(d.failures[0]!.message).toBe('Decoding failed with 3 decoded frames still open, spanning 1.00 s')
    await d.write(packet(1, 4, 2_000_000), properties('vp8'))
    expect(d.failures).toHaveLength(1)
    expect(d.trace().slice(1)).toEqual(['1.0 @0', '1.1 @500000', '1.2 @1000000', '1.3 @1500000'])
  })

  test('a config of the other kind closes the decoder it replaces', async () => {
    const d = await decoding()
    await d.write(properties(), {
      type: 'properties',
      properties: { decoderConfig: { codec: 'opus', sampleRate: 48_000, numberOfChannels: 2 } },
    })
    expect(d.decoders.map((decoder) => decoder.state)).toEqual(['closed', 'configured'])
  })
}
