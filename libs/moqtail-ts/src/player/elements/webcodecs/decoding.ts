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
import type { Engine, EngineInit } from '@/player/element/worker/host'
import { PlayerError } from '@/player/error'
import { deepEqual } from '@/player/util/json'
import { aacDescription, annexB, chunkTimestamp, chunkType } from './chunk'

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

/** A packet with what's read from it on arrival. */
interface Arrival {
  packet: Packet
  timestamp: number
  timescale: number | undefined
  /** The flushes before it arrived: a later one drops it. */
  epoch: number
}

/** Chunks waiting in the decoder before the next one waits too. */
const DECODE_QUEUE_LIMIT = 4
/**
 * Seconds held before `receive()` waits and holds the source back: packets would pile up for a
 * consumer that stopped taking frames, as playout in a hidden page does.
 * shortcut: a target latency past it pins liveBuffer slow again, raise it or let the source report what it holds if one does
 */
const HELD_LIMIT = 10
/** How far the held seconds move before they're signalled again: half of liveBuffer's default tolerance. */
const HELD_REPORT_STEP = 0.05
/** Codecs whose WebCodecs registrations read Annex B without a description. */
const ANNEX_B_CODECS = /^(avc[13]|hev1|hvc1)\./

/**
 * Decodes the items of one `loc` pad and writes them, decoded, to its one output. A frame's write
 * resolves once the consumer takes it, and the next chunk waits for that: packets wait here encoded,
 * not as frames that pin decoder buffers. The seconds they span go downstream as a `held` signal,
 * and its first failure outside `receive()`, a `PlayerError`, through `report`. It only uses what a
 * dedicated worker has, so it runs in one too.
 */
export class Decoding implements Engine {
  readonly #codecs: Codecs
  readonly #output: (item: PadOutput) => Promise<void>
  readonly #report: (report: PlayerError | PadSignal) => void
  #pending: PendingObject[] = []
  /** Frames written that the consumer hasn't closed yet: each holds a decoder buffer. */
  #open: VideoFrame[] = []
  #properties: Partial<PadProperties> = {}
  #decoder: VideoDecoder | AudioDecoder | undefined
  #kind: 'video' | 'audio' = 'video'
  #lengthPrefixed = false
  #awaitingKey = true
  /** Orders each item behind the items, flushes and events before it. */
  #tail = Promise.resolve()
  #lastWrite = Promise.resolve()
  /** The `timescale` property the output carries. */
  #outputTimescale: number | undefined
  /** The latest `timescale` property received, which the packets after it take. */
  #arrivalTimescale: number | undefined
  #epoch = 0
  /** µs since the latest flush: from the newest timestamp passed on, else the first received, to the newest received. */
  #held: { from: number; to: number } | undefined
  /** Seconds last signalled as `held`. */
  #heldSent = 0
  /** Resolved and replaced whenever the held seconds may shrink. */
  #room = Promise.withResolvers<void>()
  #failed = false
  #closed = false

  /** `codecs` default to the thread's own. */
  constructor(init: EngineInit, codecs: Codecs = globalThis) {
    this.#codecs = codecs
    this.#output = init.outputs[0]!
    this.#report = init.report
  }

  async receive(item: PadItem): Promise<void> {
    if (this.#failed) return
    if (item.type === 'properties') {
      if ('timescale' in item.properties) this.#arrivalTimescale = item.properties.timescale
      this.#enqueue(() => this.#applyProperties(item.properties))
    } else if (item.type === 'packet') {
      this.#arrived(item.packet)
    }
    while (this.#heldSeconds() >= HELD_LIMIT && !this.#closed) await this.#room.promise
  }

  /** Flushes the decoder and then writes `event`, so it stays behind the frames decoded before it. */
  event(event: PadEvent): void {
    this.#enqueue(async () => {
      await this.#flush()
      await this.#lastWrite
      await this.#output({ type: 'event', event })
    })
  }

  /** A downstream `flush` drops the packets held and what the decoder holds, and starts over at the next key frame. */
  signal(signal: PadSignal): void {
    if (signal.type !== 'flush') return
    this.#epoch++
    this.#held = undefined
    this.#roomMade()
    this.#sendHeld()
    if (this.#decoder?.state !== 'configured') return
    this.#decoder.reset()
    this.#pending.length = 0
    this.#enqueue(() => this.#configure())
  }

  close(): void {
    this.#closed = true
    this.#roomMade()
    if (this.#decoder && this.#decoder.state !== 'closed') this.#decoder.close()
  }

  #arrived(packet: Packet): void {
    const timestamp = chunkTimestamp(packet, this.#arrivalTimescale)
    const own = findProperty(packet.properties, Timescale)?.timescale
    const timescale = own === undefined ? this.#arrivalTimescale : Number(own)
    const arrival = { packet, timestamp, timescale, epoch: this.#epoch }
    this.#enqueue(() => this.#decode(arrival))
    if (this.#held) this.#held.to = Math.max(this.#held.to, timestamp)
    else this.#held = { from: timestamp, to: timestamp }
    this.#sendHeld()
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

  async #decode({ packet, timestamp, timescale, epoch }: Arrival): Promise<void> {
    await this.#lastWrite
    if (epoch !== this.#epoch) return
    const decoder = this.#decoder
    const type = chunkType(packet, this.#kind)
    if (decoder?.state !== 'configured' || (this.#awaitingKey && type !== 'key')) {
      this.#passed(timestamp)
      return
    }
    const data = this.#lengthPrefixed ? annexB(packet.payload) : packet.payload
    if (!data) {
      this.#awaitingKey = true
      this.#passed(timestamp)
      return
    }
    this.#awaitingKey = false
    const base = { track: packet.track, location: packet.location, properties: packet.properties }
    this.#pending.push({ base, timestamp, timescale })
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
   * Audio from a decoder that restamps it (see `restamped`) pairs in input order, and takes its
   * object's timestamp.
   */
  #decoded(output: VideoFrame | AudioData): void {
    const audio = this.#kind === 'audio'
    const exact = this.#pending.findIndex((p) => p.timestamp === output.timestamp)
    const index = audio && exact === -1 ? 0 : exact
    const pending = this.#pending[index]
    if (!pending) {
      output.close()
      return
    }
    const { base, timescale } = pending
    // Objects decoded after it stay, so one stray timestamp can't drop the frames behind it
    this.#pending = audio
      ? this.#pending.slice(index + 1)
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
    this.#passed(pending.timestamp)
  }

  /** Media up to `timestamp` was written or dropped. */
  #passed(timestamp: number): void {
    if (this.#held) this.#held.from = Math.max(this.#held.from, timestamp)
    this.#roomMade()
    this.#sendHeld()
  }

  #heldSeconds(): number {
    return this.#held ? (this.#held.to - this.#held.from) / 1_000_000 : 0
  }

  #sendHeld(): void {
    const seconds = this.#heldSeconds()
    if (Math.abs(seconds - this.#heldSent) < HELD_REPORT_STEP) return
    this.#heldSent = seconds
    this.#report({ type: 'held', seconds })
  }

  #roomMade(): void {
    this.#room.resolve()
    this.#room = Promise.withResolvers()
  }

  /** Running out of decoder memory shows up here, not in the consumer holding the frames. */
  #failure(): string {
    const open = this.#open.filter((frame) => frame.format !== null)
    if (open.length === 0) return 'Decoding failed'
    const timestamps = open.map((frame) => frame.timestamp)
    const span = (Math.max(...timestamps) - Math.min(...timestamps)) / 1_000_000
    return `Decoding failed with ${open.length} decoded frames still open, spanning ${span.toFixed(2)} s`
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
    this.#tail = this.#tail
      // Steps queued before close() would configure the closed decoder and fail
      .then(() => (this.#closed ? undefined : step()))
      .catch((error: unknown) => this.#reportFailure(error))
  }

  #reportFailure(error: unknown): void {
    if (this.#failed) return
    this.#failed = true
    this.#report(
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
