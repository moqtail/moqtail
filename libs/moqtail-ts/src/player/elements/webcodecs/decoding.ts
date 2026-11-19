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

import { findProperty } from '@/model/property/find'
import { Timescale } from '@/model/property/timescale'
import type { PacketBase, Packet, PadEvent, PadItem, PadOutput, PadProperties, PadSignal } from '@/player/element/pad'
import { PlayerError } from '@/player/error'
import { deepEqual } from '@/player/util/json'
import { aacDescription, annexB, chunkInit } from './chunk'

/** The WebCodecs classes the engine decodes with, from `globalThis` or a test. */
export interface Codecs {
  VideoDecoder: typeof VideoDecoder
  AudioDecoder: typeof AudioDecoder
  EncodedVideoChunk: typeof EncodedVideoChunk
  EncodedAudioChunk: typeof EncodedAudioChunk
}

/** A chunk sent to the decoder and not yet output. */
interface PendingObject {
  base: PacketBase
  timestamp: number
  timescale: number | undefined
}

interface DecoderInit {
  output: (payload: VideoFrame | AudioData) => void
  error: (error: DOMException) => void
}

/** Chunks waiting in the decoder before a packet holds its connection back. */
const DECODE_QUEUE_LIMIT = 4
/** Codecs whose WebCodecs registrations read Annex B without a description. */
const ANNEX_B_CODECS = /^(avc[13]|hev1|hvc1)\./

/**
 * Decodes the items of one `loc` pad and writes them, decoded, through `output`, whose promise
 * is the consumer's backpressure. It only uses what a dedicated worker has, so it runs in one too.
 */
export class Decoding {
  readonly #codecs: Codecs
  readonly #output: (item: PadOutput) => Promise<void>
  #pending: PendingObject[] = []
  #properties: Partial<PadProperties> = {}
  #decoder: VideoDecoder | AudioDecoder | undefined
  #kind: 'video' | 'audio' = 'video'
  #lengthPrefixed = false
  #awaitingKey = true
  /** Orders packets behind the flushes and events before them. */
  #tail = Promise.resolve()
  #lastWrite = Promise.resolve()
  /** The `timescale` property the output carries. */
  #outputTimescale: number | undefined
  #error: unknown

  constructor(codecs: Codecs, output: (item: PadOutput) => Promise<void>) {
    this.#codecs = codecs
    this.#output = output
  }

  async receive(item: PadItem): Promise<void> {
    await this.#tail
    // shortcut: a decoder error only surfaces on the next receive(), fine while sources keep writing
    if (this.#error) throw this.#error
    if (item.type === 'properties') return this.#applyProperties(item.properties)
    if (item.type === 'packet') await this.#decode(item.packet)
  }

  /** Flushes the decoder and then writes `event`, so it stays behind the frames decoded before it. */
  event(event: PadEvent): void {
    this.#enqueue(async () => {
      await this.#flush()
      await this.#lastWrite
      await this.#output({ type: 'event', event })
    })
  }

  /** A downstream `flush` drops what the decoder holds and starts over at the next key frame. */
  signal(signal: PadSignal): void {
    if (signal.type !== 'flush' || this.#decoder?.state !== 'configured') return
    this.#decoder.reset()
    this.#pending.length = 0
    this.#enqueue(() => this.#configure())
  }

  close(): void {
    if (this.#decoder && this.#decoder.state !== 'closed') this.#decoder.close()
  }

  async #applyProperties(properties: Partial<PadProperties>): Promise<void> {
    const reconfigure =
      ('decoderConfig' in properties && !deepEqual(properties.decoderConfig, this.#properties.decoderConfig)) ||
      ('initData' in properties && !deepEqual(properties.initData, this.#properties.initData))
    this.#properties = { ...this.#properties, ...properties }
    if (reconfigure) await this.#configure()
    if ('timescale' in properties) this.#outputTimescale = properties.timescale
    await this.#output({ type: 'properties', properties })
  }

  async #configure(): Promise<void> {
    const { decoderConfig, initData } = this.#properties
    if (!decoderConfig) return
    const init: DecoderInit = {
      output: (payload) => this.#decoded(payload),
      error: (error) => {
        this.#error = new PlayerError('MEDIA_FAILURE', 'Decoding failed', { element: 'webcodecs', cause: error })
      },
    }
    if ('sampleRate' in decoderConfig) {
      const { codec, sampleRate, numberOfChannels } = decoderConfig
      const description = initData ?? aacDescription(codec, sampleRate, numberOfChannels)
      const config = { ...decoderConfig, ...(description && { description }) }
      this.#decoder = await configured(this.#codecs.AudioDecoder, this.#decoder, init, config)
      this.#kind = 'audio'
    } else {
      // LOC targets interactive media (LOC abstract)
      const config = { ...decoderConfig, ...(initData && { description: initData }), optimizeForLatency: true }
      this.#decoder = await configured(this.#codecs.VideoDecoder, this.#decoder, init, config)
      this.#kind = 'video'
    }
    // With a description, payloads are in the canonical format WebCodecs expects
    this.#lengthPrefixed = !initData && ANNEX_B_CODECS.test(decoderConfig.codec)
    this.#awaitingKey = true
  }

  async #decode(packet: Packet): Promise<void> {
    await this.#lastWrite
    const decoder = this.#decoder
    if (decoder?.state !== 'configured') return
    const { type, timestamp } = chunkInit(packet, this.#kind, this.#properties.timescale)
    if (this.#awaitingKey && type !== 'key') return
    const data = this.#lengthPrefixed ? annexB(packet.payload) : packet.payload
    if (!data) {
      this.#awaitingKey = true
      return
    }
    this.#awaitingKey = false
    const base = { track: packet.track, location: packet.location, properties: packet.properties }
    const timescale = findProperty(packet.properties, Timescale)?.timescale
    this.#pending.push({
      base,
      timestamp,
      timescale: timescale === undefined ? this.#properties.timescale : Number(timescale),
    })
    if (decoder instanceof this.#codecs.AudioDecoder) {
      decoder.decode(new this.#codecs.EncodedAudioChunk({ type, timestamp, data }))
    } else {
      decoder.decode(new this.#codecs.EncodedVideoChunk({ type, timestamp, data }))
    }
    while (decoder.decodeQueueSize > DECODE_QUEUE_LIMIT) {
      await new Promise((resolve) => decoder.addEventListener('dequeue', resolve, { once: true }))
    }
  }

  /**
   * Pairs a frame with the first pending object of its timestamp: WebCodecs gives each output
   * its chunk's timestamp, but B-frames come out in presentation order, and a decoder may skip a
   * corrupt chunk, so pairing in input order would drift. Outputs come in presentation order, so
   * the objects decoded and stamped before a frame will never be output and are dropped with it.
   */
  #decoded(payload: VideoFrame | AudioData): void {
    const index = this.#pending.findIndex((p) => p.timestamp === payload.timestamp)
    if (index === -1) {
      payload.close()
      return
    }
    const { base, timescale } = this.#pending[index]!
    // Objects decoded after it stay, so one stray timestamp can't drop the frames behind it
    this.#pending = this.#pending.filter((p, i) => i > index || p.timestamp > payload.timestamp)
    // Frames carry no clock of their own (LOC 2.3.1.1, 2.3.1.2)
    if (timescale !== undefined && timescale !== this.#outputTimescale) {
      this.#outputTimescale = timescale
      void this.#output({ type: 'properties', properties: { timescale } })
    }
    this.#lastWrite = this.#output({ type: 'frame', frame: { ...base, payload } })
  }

  async #flush(): Promise<void> {
    if (this.#decoder?.state !== 'configured') return
    // WebCodecs requires a key chunk after a flush
    this.#awaitingKey = true
    // A reset or close meanwhile rejects it, and nothing it held is output
    await this.#decoder.flush().catch(() => {})
    // Chunks the decoder didn't output by now never will be
    this.#pending.length = 0
  }

  #enqueue(step: () => Promise<void>): void {
    this.#tail = this.#tail.then(step).catch((error: unknown) => {
      this.#error ??= error
    })
  }
}

/** `current` if it's a `Decoder`, else a new one, configured with `config` once it's supported. */
async function configured<D extends { configure(config: C): void }, C extends { codec: string }>(
  Decoder: {
    new (init: DecoderInit): D
    isConfigSupported(config: C): Promise<{ supported?: boolean }>
  },
  current: unknown,
  init: DecoderInit,
  config: C,
): Promise<D> {
  const { supported } = await Decoder.isConfigSupported(config)
  if (!supported) {
    throw new PlayerError('MEDIA_FAILURE', `WebCodecs can't decode "${config.codec}"`, { element: 'webcodecs' })
  }
  const decoder = current instanceof Decoder ? current : new Decoder(init)
  decoder.configure(config)
  return decoder
}
