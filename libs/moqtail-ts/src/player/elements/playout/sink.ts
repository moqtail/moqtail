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

import * as Comlink from 'comlink'
import type { ElementContext } from '@/player/element/context'
import type { Pad, PadEvent, PadItem, PadProperties, PadSignal } from '@/player/element/pad'
import { MediaSink, roleOf, type SinkPadState } from '@/player/element/sink/media_sink'
import { PlayerError } from '@/player/error'
import { createAudioOutput, type AudioOutput } from './audio'
import { PlayoutEngine, type EngineReport } from './engine'

/** @public Options of the `Playout` sink, given through `player.sink(canvas)`. */
export interface PlayoutOptions {
  canvas: HTMLCanvasElement
  /**
   * Media seconds queued before playback starts or restarts after a stall. Lower it with liveBuffer's
   * `stallThreshold`, which keeps the forward buffer above it. At least 0, default 0.5.
   */
  preroll?: number
}

interface PlayoutPadState extends SinkPadState {
  received: boolean
}

/** The HTML media element's upper bound for `timeupdate`. */
const TIME_UPDATE_INTERVAL_MS = 250

/**
 * Plays live `raw` pads: video on a canvas, audio through an AudioWorklet, video in sync with
 * the audio when the tracks share the epoch clock. The engine takes the pads' items and renders,
 * in the worker when every pad is a worker pad; this side holds the chain, the
 * `MediaSinkElement` API and the `AudioContext`.
 */
export class PlayoutSink extends MediaSink<PlayoutPadState> {
  readonly preciseRate = true
  readonly #canvas: HTMLCanvasElement
  /** Seconds. */
  readonly #preroll: number
  /** Resolved once the element knows where the engine runs. */
  readonly #engine = Promise.withResolvers<PlayoutEngine | Comlink.Remote<PlayoutEngine>>()
  #placed: { engine: PlayoutEngine } | { engine: Comlink.Remote<PlayoutEngine>; worker: true } | undefined
  #audio: Promise<AudioOutput> | undefined
  #output: AudioOutput | undefined
  #state: 'idle' | 'playing' | 'paused' = 'idle'
  #ended = false
  #seek: PromiseWithResolvers<void> | undefined
  /** Unset until something was presented. */
  #currentTime: number | undefined
  #bufferedEnd: number | undefined
  #playbackRate = 1
  #played = false
  #ready = false
  #timeUpdate = -Infinity

  constructor(context: ElementContext, canvas: HTMLCanvasElement, preroll: number) {
    super(context)
    this.#canvas = canvas
    this.#preroll = preroll

    this.on('padadded', (pad) => {
      this.#place(pad)
      this.#setTracks()
    })
    this.on('padremoved', () => this.#setTracks())
    this.on('nomorepads', () => {
      this.#start()
      this.#maybeReady()
    })

    // Without a worker there's nothing to wait for
    if (!context.worker) this.#start()
  }

  get currentTime(): number {
    return this.#currentTime ?? 0
  }

  get buffered(): TimeRanges {
    const [time, end] = [this.#currentTime, this.#bufferedEnd]
    return time === undefined || end === undefined ? timeRanges([]) : timeRanges([[time, Math.max(end, time)]])
  }

  get paused(): boolean {
    return this.#state !== 'playing'
  }

  get seeking(): boolean {
    return this.#seek !== undefined
  }

  get ended(): boolean {
    return this.#ended
  }

  get playbackRate(): number {
    return this.#playbackRate
  }

  /** No DVR window behind it; `NaN` would read as on demand. */
  get liveEdgeStart(): number {
    return this.#played ? this.currentTime : NaN
  }

  get dvrRange(): undefined {
    return undefined
  }

  setPlaybackRate(rate: number): void {
    if (!(rate > 0 && Number.isFinite(rate))) throw new TypeError(`"playout" playback rate must be > 0, got ${rate}`)

    this.#playbackRate = rate
    void this.#engine.promise.then((engine) => engine.setPlaybackRate(rate))
  }

  async play(): Promise<void> {
    if (this.#state === 'playing' || this.#ended) return

    this.#state = 'playing'
    await (await this.#engine.promise).play()

    // shortcut: a context the page doesn't allow to start keeps resume() pending until a user gesture, with no autoplayblocked for audio yet
    void this.#output?.context.resume()
  }

  async pause(): Promise<void> {
    if (this.#state === 'paused') return

    this.#state = 'paused'
    await (await this.#engine.promise).pause()

    void this.#output?.context.suspend()
    this.dispatchEvent(new Event('pause'))
  }

  /** Forward only, as `liveBuffer` jumps: what's before `mediaTime` is dropped, and playback goes on from there. */
  async seek(mediaTime: number): Promise<void> {
    if (!(mediaTime >= this.currentTime)) throw new TypeError(`"playout" can't seek back: it has no DVR`)

    this.#seek?.resolve()

    const seek = Promise.withResolvers<void>()
    this.#seek = seek
    this.#currentTime = mediaTime

    this.dispatchEvent(new Event('seeking'))
    await (await this.#engine.promise).seek(mediaTime)

    return seek.promise
  }

  override async destroy(): Promise<void> {
    await this.#placed?.engine.close()

    this.#seek?.reject(new PlayerError('ABORTED', 'The sink was destroyed', { element: 'playout' }))
    this.#seek = undefined

    this.#closeAudio()
    await super.destroy()
  }

  protected newPadState(): PlayoutPadState {
    return { properties: {}, ended: false, received: false }
  }

  protected async receiveMedia(
    pad: Pad,
    _state: PlayoutPadState,
    item: Exclude<PadItem, { type: 'properties' }>,
  ): Promise<void> {
    await (await this.#engine.promise).receive(item, roleOf(pad)!)
  }

  protected override async propertiesChanged(
    pad: Pad,
    _state: PlayoutPadState,
    changes: Partial<PadProperties>,
  ): Promise<void> {
    await (await this.#engine.promise).receive({ type: 'properties', properties: changes }, roleOf(pad)!)
  }

  protected padEvent(pad: Pad, _state: PlayoutPadState, event: PadEvent): void {
    void this.#engine.promise.then((engine) => engine.event(event, roleOf(pad)!))
  }

  protected padSignal(pad: Pad, _state: PlayoutPadState, signal: PadSignal): void {
    void this.#engine.promise.then((engine) => engine.signal(roleOf(pad)!, signal))
  }

  /**
   * In the worker when every pad is a worker pad and the canvas can still transfer, so frames never
   * come back to the main thread only to cross again.
   */
  #start(): void {
    if (this.#placed) return
    const worker = this.context.worker
    const pads = this.pads.filter((pad) => pad.direction === 'sink')
    const report = (report: EngineReport) => this.#onReport(report)
    const offscreen =
      worker && pads.length > 0 && pads.every((pad) => worker.inWorker(pad)) && offscreenOf(this.#canvas)

    if (worker && offscreen) {
      const options = { canvas: offscreen, preroll: this.#preroll }
      this.#placed = {
        engine: worker.create<PlayoutEngine>('playout', { options, transfer: [offscreen], report }),
        worker: true,
      }
    } else {
      const failure = contextFailure(this.#canvas)
      if (failure) {
        this.fail(new PlayerError('UNSUPPORTED_SOURCE', failure, { element: 'playout' }))
        return
      }

      this.#placed = {
        engine: new PlayoutEngine({ outputs: [], options: { canvas: this.#canvas, preroll: this.#preroll }, report }),
      }
    }

    this.#engine.resolve(this.#placed.engine)

    for (const pad of pads) this.#place(pad)
  }

  /** A worker pad links to an engine in the worker, else comes to `receive()`, which forwards it. */
  #place(pad: Pad): void {
    const worker = this.context.worker
    if (pad.direction !== 'sink' || !this.#placed || !worker?.inWorker(pad)) return

    if ('worker' in this.#placed) worker.link(pad, this.#placed.engine)
    else worker.bridge(pad)
  }

  /** Created on the first `AudioData`'s format, which the engine reports. */
  #audioOutput(sampleRate: number, numberOfChannels: number): void {
    this.#audio ??= createAudioOutput(sampleRate, numberOfChannels, this.#preroll).then(
      async (output) => {
        this.#output = output

        if (this.context.signal.aborted) {
          this.#closeAudio()
          return output
        }

        output.node.onprocessorerror = (error) =>
          this.fail(
            new PlayerError('MEDIA_FAILURE', 'The audio processor failed', { element: 'playout', cause: error }),
          )

        const engine = await this.#engine.promise
        await engine.attachAudio(Comlink.transfer(output.node.port, [output.node.port]))

        if (this.#state === 'playing') void output.context.resume()

        return output
      },
      (cause: unknown) => {
        throw new PlayerError('MEDIA_FAILURE', 'Could not start the audio output', { element: 'playout', cause })
      },
    )

    this.#audio.catch((error: PlayerError) => this.fail(error))
  }

  #setTracks(): void {
    const roles = this.pads.map((pad) => roleOf(pad)!)
    void this.#engine.promise.then((engine) => engine.setTracks(roles))
  }

  #maybeReady(): void {
    if (this.#ready || !this.negotiated || this.states.size === 0) return
    if (![...this.states.values()].every((s) => s.received)) return

    this.#ready = true
    this.emit('ready')
  }

  #onReport(report: EngineReport): void {
    switch (report.type) {
      case 'first':
        for (const [pad, state] of this.states) if (roleOf(pad) === report.role) state.received = true
        this.#maybeReady()
        break
      case 'audio':
        this.#audioOutput(report.sampleRate, report.numberOfChannels)
        break
      case 'presentation':
        this.#presentation(report.currentTime, report.bufferedEnd)
        break
      case 'seeked':
        this.#seek?.resolve()
        this.#seek = undefined
        this.dispatchEvent(new Event('seeked'))
        break
      case 'ended':
        this.#ended = true
        this.#state = 'paused'
        void this.#output?.context.suspend()
        this.dispatchEvent(new Event('ended'))
        break
      case 'error':
        this.fail(new PlayerError('MEDIA_FAILURE', 'Playout failed', { element: 'playout', cause: report.error }))
        break
      default:
        this.dispatchEvent(new Event(report.type))
    }
  }

  /** Each presentation gives the engine the audio output's latest timestamp. */
  #presentation(currentTime: number | undefined, bufferedEnd: number | undefined): void {
    if (currentTime !== undefined) this.#currentTime = currentTime
    this.#bufferedEnd = bufferedEnd

    const context = this.#output?.context
    if (context?.state === 'running') {
      const { contextTime = 0, performanceTime = 0 } = context.getOutputTimestamp()
      if (performanceTime > 0)
        void this.#placed?.engine.outputTimestamp({ contextTime, time: performance.timeOrigin + performanceTime })
    }

    if (this.#state !== 'playing') return

    this.#played = true
    const now = performance.now()

    if (now - this.#timeUpdate < TIME_UPDATE_INTERVAL_MS) return
    this.#timeUpdate = now
    this.dispatchEvent(new Event('timeupdate'))
  }

  #closeAudio(): void {
    const output = this.#output
    if (!output) return

    output.node.disconnect()
    void output.context.close()
  }
}

/** Why the canvas can't take a 2D context on this thread, if it can't. */
function contextFailure(canvas: HTMLCanvasElement): string | undefined {
  try {
    return canvas.getContext('2d') ? undefined : 'The playout canvas already has a context other than "2d"'
  } catch {
    // Its control went to the worker, which can't be undone
    return "The playout canvas was handed to an earlier player's worker: give each player its own canvas"
  }
}

/** The canvas's control, or undefined once it has a context or was transferred. */
function offscreenOf(canvas: HTMLCanvasElement): OffscreenCanvas | undefined {
  try {
    return canvas.transferControlToOffscreen()
  } catch {
    return undefined
  }
}

/** `TimeRanges` over `[start, end]` pairs, throwing out of range as the platform does. */
function timeRanges(ranges: [number, number][]): TimeRanges {
  const at = (index: number, edge: 0 | 1) => {
    const range = ranges[index]
    if (!range) throw new DOMException(`No range at ${index}`, 'IndexSizeError')
    return range[edge]
  }

  return { length: ranges.length, start: (index) => at(index, 0), end: (index) => at(index, 1) }
}

if (import.meta.vitest) {
  const { test, expect } = import.meta.vitest

  test('timeRanges throws an IndexSizeError out of range', () => {
    const ranges = timeRanges([[1, 2]])
    expect([ranges.start(0), ranges.end(0)]).toEqual([1, 2])
    expect(() => ranges.end(1)).toThrow(expect.objectContaining({ name: 'IndexSizeError' }))
  })
}
