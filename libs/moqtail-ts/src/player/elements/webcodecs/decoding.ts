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
/** Frames open past this when the decoder fails hint it ran out of buffers. */
const OPEN_FRAMES_EXPECTED = 2
/** Codecs whose WebCodecs registrations read Annex B without a description. */
const ANNEX_B_CODECS = /^(avc[13]|hev1|hvc1)\./

/**
 * Decodes the items of one `loc` pad and writes them, decoded, through `output`, whose promise
 * is the consumer's backpressure, and reports its first failure outside `receive()` to `fail`.
 * It only uses what a dedicated worker has, so it runs in one too.
 */
export class Decoding {
  readonly #codecs: Codecs
  readonly #output: (item: PadOutput) => Promise<void>
  readonly #fail: (error: PlayerError) => void
  #pending: PendingObject[] = []
  /** Frames written that the consumer hasn't closed yet: each holds a decoder buffer. */
  #open: VideoFrame[] = []
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
  #failed = false

  constructor(codecs: Codecs, output: (item: PadOutput) => Promise<void>, fail: (error: PlayerError) => void) {
    this.#codecs = codecs
    this.#output = output
    this.#fail = fail
  }

  async receive(item: PadItem): Promise<void> {
    await this.#tail
    if (this.#failed) return
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
      error: (error) => this.#reportFailure(error),
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
   * Audio pairs in input order instead, and takes its object's timestamp (see `restamped`).
   */
  #decoded(output: VideoFrame | AudioData): void {
    const audio = this.#kind === 'audio'
    const index = audio ? 0 : this.#pending.findIndex((p) => p.timestamp === output.timestamp)
    const pending = this.#pending[index]
    if (!pending) {
      output.close()
      return
    }
    const { base, timescale } = pending
    // Objects decoded after it stay, so one stray timestamp can't drop the frames behind it
    this.#pending = audio
      ? this.#pending.slice(1)
      : this.#pending.filter((p, i) => i > index || p.timestamp > output.timestamp)
    const payload = audio ? restamped(output as AudioData, pending.timestamp) : output
    // Frames carry no clock of their own (LOC 2.3.1.1, 2.3.1.2)
    if (timescale !== undefined && timescale !== this.#outputTimescale) {
      this.#outputTimescale = timescale
      void this.#output({ type: 'properties', properties: { timescale } })
    }
    if (this.#kind === 'video') {
      this.#open = this.#open.filter((frame) => frame.format !== null)
      this.#open.push(payload as VideoFrame)
    }
    this.#lastWrite = this.#output({ type: 'frame', frame: { ...base, payload } })
  }

  /** Running out of decoder memory shows up here, not in the consumer holding the frames. */
  #failure(): string {
    const open = this.#open.filter((frame) => frame.format !== null)
    if (open.length === 0) return 'Decoding failed'
    const timestamps = open.map((frame) => frame.timestamp)
    const span = (Math.max(...timestamps) - Math.min(...timestamps)) / 1_000_000
    const message = `Decoding failed with ${open.length} decoded frames still open, spanning ${span.toFixed(2)} s`
    if (open.length <= OPEN_FRAMES_EXPECTED) return message
    return `${message}: playback is too far behind for the frames the decoder can keep alive, lower the target latency`
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
    this.#tail = this.#tail.then(step).catch((error: unknown) => this.#reportFailure(error))
  }

  #reportFailure(error: unknown): void {
    if (this.#failed) return
    this.#failed = true
    this.#fail(
      error instanceof PlayerError
        ? error
        : new PlayerError('MEDIA_FAILURE', this.#failure(), { element: 'webcodecs', cause: error }),
    )
  }
}

/**
 * `data` at its object's `timestamp`. Chromium stamps decoded audio from the first chunk on by
 * the samples decoded, not with each chunk's own timestamp, so lost objects and the publisher's
 * clock would drift it off the video's timeline.
 */
function restamped(data: AudioData, timestamp: number): AudioData {
  if (data.timestamp === timestamp) return data
  const { sampleRate, numberOfChannels, numberOfFrames } = data
  const planes = new Float32Array(numberOfChannels * numberOfFrames)
  for (let planeIndex = 0; planeIndex < numberOfChannels; planeIndex++) {
    const plane = planes.subarray(planeIndex * numberOfFrames, (planeIndex + 1) * numberOfFrames)
    data.copyTo(plane, { planeIndex, format: 'f32-planar' })
  }
  data.close()
  return new AudioData({ format: 'f32-planar', sampleRate, numberOfChannels, numberOfFrames, timestamp, data: planes })
}

/**
 * `current` if it's a `Decoder`, else a new one that replaces and closes it, configured with
 * `config` once it's supported.
 */
async function configured<D extends { configure(config: C): void }, C extends { codec: string }>(
  Decoder: {
    new (init: DecoderInit): D
    isConfigSupported(config: C): Promise<{ supported?: boolean }>
  },
  current: VideoDecoder | AudioDecoder | undefined,
  init: DecoderInit,
  config: C,
): Promise<D> {
  const { supported } = await Decoder.isConfigSupported(config)
  if (!supported) {
    throw new PlayerError('MEDIA_FAILURE', `WebCodecs can't decode "${config.codec}"`, { element: 'webcodecs' })
  }
  if (current instanceof Decoder) {
    current.configure(config)
    return current
  }
  if (current && current.state !== 'closed') current.close()
  const decoder = new Decoder(init)
  decoder.configure(config)
  return decoder
}
