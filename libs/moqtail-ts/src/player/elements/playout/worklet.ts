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

/** What the engine posts to the processor. Times in media seconds. */
export type ProcessorMessage =
  | { type: 'samples'; planes: Float32Array[]; timestamp: number; duration: number }
  | { type: 'rate'; rate: number }
  | { type: 'seek'; time: number }
  | { type: 'flush' }

/** What the processor posts back. Times in seconds. */
export type ProcessorReport =
  /** The first sample played since the start or the last seek or flush, and the context time of its quantum. */
  | { type: 'start'; contextTime: number; mediaTime: number }
  /** Since that start: media time played as silence, and media time that never arrived, as of a quantum. */
  | { type: 'drift'; contextTime: number; drift: number; gap: number }
  | { type: 'starved' }
  | { type: 'fed' }

declare const registerProcessor: (name: string, processor: new () => AudioWorkletProcessorBase) => void
declare const AudioWorkletProcessor: new () => { readonly port: MessagePort }
declare const sampleRate: number
declare const currentTime: number

interface AudioWorkletProcessorBase {
  readonly port: MessagePort
  process(inputs: Float32Array[][], outputs: Float32Array[][]): boolean
}

export const PROCESSOR_NAME = 'moqtail-playout'

/** The module the worklet loads. */
export const PROCESSOR_SOURCE = `(${processorModule.toString()})(${JSON.stringify(PROCESSOR_NAME)})`

/**
 * Runs in the `AudioWorkletGlobalScope` from its source text, so it references nothing outside
 * itself and avoids syntax a bundler lowers to helpers, such as `#private` fields.
 */
function processorModule(name: string): void {
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

    constructor() {
      super()
      this.chunks = []
      this.position = 0
      this.rate = 1
      this.dropBefore = -Infinity
      this.end = undefined
      this.duration = Infinity
      this.starved = false
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
      } else {
        this.chunks = []
        this.position = 0
        this.end = undefined
        this.restart()
      }
    }

    process(_inputs: Float32Array[][], outputs: Float32Array[][]): boolean {
      const output = outputs[0]!
      const length = output[0]!.length
      const skew = this.gap - this.drift
      let played = 0
      const head = this.head()
      if (head && !this.started) {
        this.started = true
        this.gap = 0
        this.port.postMessage({ type: 'start', contextTime: currentTime, mediaTime: this.mediaTime(head) })
      }
      for (; played < length; played++) {
        const chunk = this.head()
        if (!chunk) break
        for (let channel = 0; channel < output.length; channel++) output[channel]![played] = this.sample(chunk, channel)
        this.position += this.rate
      }
      for (let i = played; i < length; i++) for (const channel of output) channel[i] = 0
      this.report(played, length - played, skew)
      return true
    }

    /** The chunk the read position is in, counting the gap before it once it plays. */
    head(): Chunk | undefined {
      let chunk = this.chunks[0]
      while (chunk && this.position >= chunk.planes[0]!.length) {
        this.position -= chunk.planes[0]!.length
        this.chunks.shift()
        chunk = this.chunks[0]
      }
      if (chunk && chunk.gap !== 0) {
        this.gap += chunk.gap
        chunk.gap = 0
      }
      return chunk
    }

    // shortcut: linear interpolation moves the pitch with the rate; a time-stretch such as WSOLA keeps it, upgrade if catch-up is audible
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
        this.chunks.shift()
        this.position = 0
        chunk = this.chunks[0]
      }
      if (chunk && chunk.timestamp < this.dropBefore)
        this.position = Math.max(this.position, (this.dropBefore - chunk.timestamp) * sampleRate)
    }

    /** The next sample played starts the clock again. */
    restart(): void {
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
  let scope: Record<string, unknown>

  beforeEach(() => {
    reports = []
    const port = { onmessage: null, postMessage: (report: ProcessorReport) => reports.push(report) }
    scope = {
      sampleRate: RATE,
      currentTime: 10,
      AudioWorkletProcessor: class {
        readonly port = port
      },
      registerProcessor: (_name: string, Processor: new () => AudioWorkletProcessorBase) => {
        processor = new Processor()
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

  test('plays chunks back to back, then counts silence as drift and reports a stall past a chunk', () => {
    post(samples(100, 1, 2))
    post(samples(100.5, 3, 4))
    expect(quantum()).toEqual([1, 2, 3, 4])
    expect(reports).toEqual([{ type: 'start', contextTime: 10, mediaTime: 100 }])
    expect(quantum(2)).toEqual([0, 0])
    expect(reports.slice(1)).toEqual([{ type: 'drift', contextTime: 10, drift: 0.5, gap: 0 }])
    expect(quantum(1)).toEqual([0])
    expect(reports.slice(2)).toEqual([{ type: 'drift', contextTime: 10, drift: 0.75, gap: 0 }, { type: 'starved' }])
    post(samples(101, 5))
    expect(quantum(2)).toEqual([5, 0])
    expect(reports.slice(4)).toEqual([{ type: 'fed' }, { type: 'drift', contextTime: 10, drift: 1, gap: 0 }])
  })

  test('audio that never arrived counts as a gap once playback reaches it', () => {
    post(samples(100, 1, 2))
    post(samples(101, 3, 4))
    expect(quantum(2)).toEqual([1, 2])
    expect(reports.at(-1)).toEqual({ type: 'start', contextTime: 10, mediaTime: 100 })
    expect(quantum(2)).toEqual([3, 4])
    expect(reports.at(-1)).toEqual({ type: 'drift', contextTime: 10, drift: 0, gap: 0.5 })
  })

  test('a rate other than 1 interpolates between neighbours, across chunks', () => {
    post({ type: 'rate', rate: 1.5 })
    post(samples(100, 0, 2, 4))
    post(samples(100.75, 6, 8, 10))
    expect(quantum()).toEqual([0, 3, 6, 9])
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
