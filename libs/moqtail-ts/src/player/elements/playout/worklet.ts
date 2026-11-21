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

import { createStretch, type Stretch } from './wsola'

/** What the engine posts to the processor. Times in media seconds. */
export type ProcessorMessage =
  | { type: 'samples'; planes: Float32Array[]; timestamp: number; duration: number }
  | { type: 'rate'; rate: number }
  | { type: 'seek'; time: number }
  | { type: 'flush' }
  | { type: 'end' }

/** What the processor posts back. Times in seconds. */
export type ProcessorReport =
  /** The first sample played since the start or the last seek or flush, and the context time of its quantum. */
  | { type: 'start'; contextTime: number; mediaTime: number }
  /** Since that start: media time played as silence, and media time that never arrived, as of a quantum. */
  | { type: 'drift'; contextTime: number; drift: number; gap: number }
  | { type: 'starved' }
  | { type: 'fed' }
  /** Media seconds of the chunks played out or dropped so far: the engine holds samples back on it. */
  | { type: 'consumed'; seconds: number }

declare const registerProcessor: (
  name: string,
  processor: new (options: ProcessorOptions) => AudioWorkletProcessorBase,
) => void
declare const AudioWorkletProcessor: new (options: ProcessorOptions) => { readonly port: MessagePort }
declare const sampleRate: number
declare const currentTime: number

interface AudioWorkletProcessorBase {
  readonly port: MessagePort
  process(inputs: Float32Array[][], outputs: Float32Array[][]): boolean
}

/** What `new AudioWorkletNode()` passes the processor. */
export interface ProcessorOptions {
  processorOptions: { preroll: number }
}

export const PROCESSOR_NAME = 'moqtail-playout'

/** The default preroll: media seconds queued before the clock starts or restarts after a stall; liveBuffer's default stall threshold. */
export const PREROLL = 0.5

/** The module the worklet loads. */
export const PROCESSOR_SOURCE = `(${processorModule.toString()})(${JSON.stringify(PROCESSOR_NAME)}, ${createStretch.toString()})`

/**
 * Runs in the `AudioWorkletGlobalScope` from its source text, so it references nothing outside
 * itself and avoids syntax a bundler lowers to helpers, such as `#private` fields.
 */
function processorModule(name: string, createStretch: (sampleRate: number) => Stretch): void {
  interface Chunk {
    planes: Float32Array[]
    timestamp: number
    duration: number
    /** Media seconds missing before it, counted once it plays. */
    gap: number
  }

  class PlayoutProcessor extends AudioWorkletProcessor {
    declare chunks: Chunk[]
    /** In media samples into the first chunk; fractional at a rate other than 1. */
    declare position: number
    declare rate: number
    declare started: boolean
    /** Set by a seek: media before it is dropped. */
    declare dropBefore: number
    /** Where the last chunk received ends, to tell how much audio never arrived. */
    declare end: number | undefined
    declare drift: number
    declare gap: number
    /** Media seconds of silence in a row. */
    declare silence: number
    declare starved: boolean
    /** Of the last chunk received: silence past it is a stall. */
    declare duration: number
    declare consumed: number
    declare reported: number
    /** Set by `end`: no preroll to wait for. */
    declare ending: boolean
    declare stretch: Stretch
    /** What `window` copies into, reused: this runs on the audio thread. */
    declare input: Float32Array[]
    declare preroll: number

    constructor(options: ProcessorOptions) {
      super(options)
      this.preroll = options.processorOptions.preroll
      this.chunks = []
      this.position = 0
      this.rate = 1
      this.dropBefore = -Infinity
      this.end = undefined
      this.duration = Infinity
      this.starved = false
      this.consumed = 0
      this.reported = 0
      this.ending = false
      this.stretch = createStretch(sampleRate)
      this.input = []

      this.restart()

      this.port.onmessage = (event: MessageEvent<ProcessorMessage>) => this.receive(event.data)
    }

    receive(message: ProcessorMessage): void {
      if (message.type === 'samples') {
        const { planes, timestamp, duration } = message
        const gap = this.end === undefined ? 0 : timestamp - this.end

        this.end = timestamp + duration
        this.duration = duration

        this.chunks.push({ planes, timestamp, duration, gap })
        this.trim()
      } else if (message.type === 'rate') {
        this.rate = message.rate
      } else if (message.type === 'seek') {
        this.dropBefore = message.time
        this.restart()
        this.trim()
      } else if (message.type === 'end') {
        this.ending = true
      } else {
        while (this.chunks.length > 0) this.shift()

        this.position = 0
        this.end = undefined
        this.ending = false
        this.restart()
      }

      this.reportConsumed()
    }

    process(_inputs: Float32Array[][], outputs: Float32Array[][]): boolean {
      const output = outputs[0]!
      const length = output[0]!.length
      const skew = this.gap - this.drift
      let played = 0
      let head = this.head()

      // Silence while refilling after a stall counts as drift, so the clock holds
      if (head && (!this.started || this.starved) && !this.ending && this.end! - this.mediaTime(head) < this.preroll)
        head = undefined

      if (head && !this.started) {
        this.started = true
        this.gap = 0
        this.port.postMessage({ type: 'start', contextTime: currentTime, mediaTime: this.mediaTime(head) })
      }

      for (; head && played < length; played++) {
        const chunk = this.head()
        if (!chunk) break
        this.write(output, played, chunk)
        this.position += this.rate
      }

      // Ran out: what the segment holds is from before the stall
      if (played < length) this.stretch.reset()
      for (let i = played; i < length; i++) for (const channel of output) channel[i] = 0

      this.reportConsumed()
      this.report(played, length - played, skew)

      return true
    }

    /** The chunk the read position is in, counting the gap before it once it plays. */
    head(): Chunk | undefined {
      let chunk = this.chunks[0]

      while (chunk && this.position >= chunk.planes[0]!.length) {
        this.position -= chunk.planes[0]!.length
        this.shift()
        chunk = this.chunks[0]
      }

      if (chunk && chunk.gap !== 0) {
        this.gap += chunk.gap
        chunk.gap = 0
      }

      return chunk
    }

    write(output: Float32Array[], at: number, chunk: Chunk): void {
      const stretch = this.stretch

      if (stretch.running(this.rate) && stretch.due()) {
        const input = this.window(stretch.span, stretch.needs(this.rate))
        // Too little queued to stretch: played at the rate's pitch until the queue refills
        if (input) {
          // The media the last segment's alignment skips plays ahead of the clock unless counted
          const skipped = stretch.fill(input, this.rate)
          this.position += skipped
          this.drift -= skipped / sampleRate
        } else stretch.reset()
      }

      if (!stretch.due()) stretch.write(output, at)
      else for (let channel = 0; channel < output.length; channel++) output[channel]![at] = this.sample(chunk, channel)
    }

    /** `length` samples per channel from the position on, or undefined if fewer than `needed` are queued. */
    window(length: number, needed: number): Float32Array[] | undefined {
      const start = Math.floor(this.position)
      let queued = -start
      for (const chunk of this.chunks) queued += chunk.planes[0]!.length
      if (queued < needed) return undefined

      const channels = this.chunks[0]!.planes.length

      if (this.input.length !== channels || this.input[0]!.length !== length)
        this.input = Array.from({ length: channels }, () => new Float32Array(length))

      const planes = this.input
      let offset = -start

      for (const chunk of this.chunks) {
        if (offset >= length) break
        for (const [channel, plane] of planes.entries()) {
          const source = chunk.planes[channel] ?? chunk.planes[0]!
          plane.set(source.subarray(Math.max(0, -offset), length - offset), Math.max(0, offset))
        }
        offset += chunk.planes[0]!.length
      }

      return planes
    }

    /** Interpolated: the position is fractional after a rate other than 1. */
    sample(chunk: Chunk, channel: number): number {
      const plane = chunk.planes[channel] ?? chunk.planes[0]!

      const index = Math.floor(this.position)
      const fraction = this.position - index
      const current = plane[index]!
      const next = plane[index + 1] ?? this.chunks[1]?.planes[channel]?.[0] ?? current

      return current + (next - current) * fraction
    }

    mediaTime(chunk: Chunk): number {
      return chunk.timestamp + this.position / sampleRate
    }

    /** `skew` is `gap - drift` before the quantum. */
    report(played: number, silent: number, skew: number): void {
      if (!this.started) return

      if (played > 0) {
        this.silence = 0
        if (this.starved) this.port.postMessage({ type: 'fed' })
        this.starved = false
      }

      const missed = (silent * this.rate) / sampleRate
      this.drift += missed
      this.silence += missed

      if (this.gap - this.drift !== skew) {
        this.port.postMessage({ type: 'drift', contextTime: currentTime, drift: this.drift, gap: this.gap })
      }

      if (!this.starved && this.silence > this.duration) {
        this.starved = true
        this.port.postMessage({ type: 'starved' })
      }
    }

    /** Drops what ends before `dropBefore`, and starts the first chunk there. */
    trim(): void {
      let chunk = this.chunks[0]

      while (chunk && chunk.timestamp + chunk.duration <= this.dropBefore) {
        this.shift()
        this.position = 0
        chunk = this.chunks[0]
      }

      if (chunk && chunk.timestamp < this.dropBefore)
        this.position = Math.max(this.position, (this.dropBefore - chunk.timestamp) * sampleRate)
    }

    shift(): void {
      this.consumed += this.chunks.shift()!.duration
    }

    reportConsumed(): void {
      if (this.consumed === this.reported) return

      this.reported = this.consumed
      this.port.postMessage({ type: 'consumed', seconds: this.consumed })
    }

    /** The next sample played starts the clock again. */
    restart(): void {
      this.stretch.reset()
      this.started = false
      this.drift = 0
      this.gap = 0
      this.silence = 0
    }
  }

  registerProcessor(name, PlayoutProcessor)
}

if (import.meta.vitest) {
  const { test, expect, beforeEach, afterEach } = import.meta.vitest
  const RATE = 4
  let processor: AudioWorkletProcessorBase
  let reports: ProcessorReport[]
  let consumed: number[]
  let scope: Record<string, unknown>

  beforeEach(() => {
    reports = []
    consumed = []

    const port = {
      onmessage: null,
      postMessage: (report: ProcessorReport) => {
        if (report.type === 'consumed') consumed.push(report.seconds)
        else reports.push(report)
      },
    }

    scope = {
      sampleRate: RATE,
      currentTime: 10,
      AudioWorkletProcessor: class {
        readonly port = port
      },
      registerProcessor: (_name: string, Processor: new (options: ProcessorOptions) => AudioWorkletProcessorBase) => {
        processor = new Processor({ processorOptions: { preroll: PREROLL } })
      },
    }

    Object.assign(globalThis, scope)
    new Function(PROCESSOR_SOURCE)()
  })

  afterEach(() => {
    for (const key of Object.keys(scope)) delete (globalThis as Record<string, unknown>)[key]
  })

  const post = (message: ProcessorMessage) =>
    (processor.port.onmessage as (event: { data: ProcessorMessage }) => void)({ data: message })
  /** A chunk of `samples` at `timestamp` seconds, one channel. */
  const samples = (timestamp: number, ...values: number[]): ProcessorMessage => ({
    type: 'samples',
    planes: [Float32Array.from(values)],
    timestamp,
    duration: values.length / RATE,
  })
  /** One quantum of `length` samples on one channel. */
  const quantum = (length = RATE) => {
    const output = [new Float32Array(length)]
    processor.process([], [output])
    return [...output[0]!]
  }

  test('waits for PREROLL queued before it starts, unless no more samples come', () => {
    post(samples(100, 1))
    expect(quantum(2)).toEqual([0, 0])
    expect(reports).toEqual([])

    post({ type: 'end' })
    expect(quantum(2)).toEqual([1, 0])
    expect(reports[0]).toEqual({ type: 'start', contextTime: 10, mediaTime: 100 })
  })

  test('plays chunks back to back, counts silence as drift, and refills PREROLL after a stall', () => {
    post(samples(100, 1, 2))
    post(samples(100.5, 3, 4))

    expect(quantum()).toEqual([1, 2, 3, 4])
    expect(reports).toEqual([{ type: 'start', contextTime: 10, mediaTime: 100 }])
    expect(quantum(2)).toEqual([0, 0])
    expect(reports.slice(1)).toEqual([{ type: 'drift', contextTime: 10, drift: 0.5, gap: 0 }])
    expect(quantum(1)).toEqual([0])
    expect(reports.slice(2)).toEqual([{ type: 'drift', contextTime: 10, drift: 0.75, gap: 0 }, { type: 'starved' }])

    post(samples(101, 5))
    expect(quantum(1)).toEqual([0])

    post(samples(101.25, 6))
    expect(quantum(2)).toEqual([5, 6])
    expect(reports.slice(4)).toEqual([{ type: 'drift', contextTime: 10, drift: 1, gap: 0 }, { type: 'fed' }])
  })

  test('audio that never arrived counts as a gap once playback reaches it', () => {
    post(samples(100, 1, 2))
    post(samples(101, 3, 4))

    expect(quantum(2)).toEqual([1, 2])
    expect(reports.at(-1)).toEqual({ type: 'start', contextTime: 10, mediaTime: 100 })
    expect(quantum(2)).toEqual([3, 4])
    expect(reports.at(-1)).toEqual({ type: 'drift', contextTime: 10, drift: 0, gap: 0.5 })
  })

  test('a rate other than 1 is stretched across chunks, and back at 1 plays the samples once that winds down', () => {
    Object.assign(globalThis, { sampleRate: 48_000 })
    new Function(PROCESSOR_SOURCE)()

    // A ramp of 20 ms chunks: played in order, it steps by 1; resampled at 1.5, by 1.5
    for (let i = 0; i < 50; i++) {
      const planes = [Float32Array.from({ length: 960 }, (_, j) => i * 960 + j)]
      post({ type: 'samples', planes, timestamp: 100 + i * 0.02, duration: 0.02 })
    }

    const steps = (length: number) => quantum(length).flatMap((v, i, all) => (i > 0 ? [v - all[i - 1]!] : []))

    post({ type: 'rate', rate: 1.5 })
    expect(steps(128).every((step) => step === 1)).toBe(true)

    for (let i = 0; i < 75; i++) quantum(128)
    post({ type: 'rate', rate: 1 })
    for (let i = 0; i < 40; i++) quantum(128)

    for (const step of steps(128)) expect(step).toBeCloseTo(1, 1)
  })

  test("at rate 2 it stretches only with a segment's worth queued, and a stall drops what the segment held", () => {
    Object.assign(globalThis, { sampleRate: 48_000 })
    new Function(PROCESSOR_SOURCE)()

    const chunk = (timestamp: number, length: number, value?: number): ProcessorMessage => {
      const planes = [Float32Array.from({ length }, (_, j) => value ?? j)]
      return { type: 'samples', planes, timestamp, duration: length / 48_000 }
    }

    post(chunk(100, 3200))
    post({ type: 'end' })
    post({ type: 'rate', rate: 2 })
    quantum(2048)

    post(chunk(100.1, 4000, 7))
    expect(quantum(128).every((v) => v === 7)).toBe(true)
  })

  test('reports the seconds it played out or dropped, once per change', () => {
    post(samples(100, 1, 2))
    post(samples(100.5, 3, 4))
    post(samples(101, 5, 6))

    quantum(2)
    expect(consumed).toEqual([])
    quantum(1)
    expect(consumed).toEqual([0.5])

    post({ type: 'seek', time: 101 })
    post({ type: 'flush' })
    expect(consumed).toEqual([0.5, 1, 1.5])
  })

  test('a seek drops what is before the target, inside a chunk too, and starts the clock at it', () => {
    post(samples(100, 1, 2))
    expect(quantum(1)).toEqual([1])

    post(samples(100.5, 3, 4, 5, 6))
    post({ type: 'seek', time: 101 })
    post(samples(101.5, 7))

    expect(quantum()).toEqual([5, 6, 7, 0])
    expect(reports.filter((r) => r.type === 'start').at(-1)).toEqual({
      type: 'start',
      contextTime: 10,
      mediaTime: 101,
    })
  })
}
