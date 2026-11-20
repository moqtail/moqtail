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

import type { ElementContext } from '@/player/element/context'
import type { Pad, PadEvent, PadItem, PadProperties, PadSignal } from '@/player/element/pad'
import { MediaSink, roleOf, type SinkPadState } from '@/player/element/sink/media_sink'
import { PlayerError } from '@/player/error'
import { createAudioOutput, type AudioOutput } from './audio'
import { PlayoutEngine, type EngineReport, type Role } from './engine'

/** @public Options of the `Playout` sink, given through `player.sink(canvas)`. */
export interface PlayoutOptions {
  canvas: HTMLCanvasElement
}

interface PlayoutPadState extends SinkPadState {
  /** Set once the pad's first frame went to the engine. */
  received: boolean
}

/** The HTML media element's upper bound for `timeupdate`. */
const TIME_UPDATE_INTERVAL_MS = 250

/**
 * Plays live `raw` pads: video on a canvas, audio through an AudioWorklet, video in sync with
 * the audio when the tracks share the epoch clock. The engine renders; this side holds the
 * chain, the `MediaSinkElement` API and the `AudioContext`.
 */
export class PlayoutSink extends MediaSink<PlayoutPadState> {
  readonly #engine: PlayoutEngine
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

  constructor(context: ElementContext, canvas: CanvasRenderingContext2D) {
    super(context)
    this.#engine = new PlayoutEngine(canvas, (report) => this.#onReport(report))
    this.on('padadded', () => this.#setTracks())
    this.on('padremoved', () => this.#setTracks())
    this.on('nomorepads', () => this.#maybeReady())
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
    void this.#engine.setPlaybackRate(rate)
  }

  async play(): Promise<void> {
    if (this.#state === 'playing' || this.#ended) return
    this.#state = 'playing'
    await this.#engine.play()
    // shortcut: a context the page doesn't allow to start keeps resume() pending until a user gesture, with no autoplayblocked for audio yet
    void this.#output?.context.resume()
  }

  async pause(): Promise<void> {
    if (this.#state === 'paused') return
    this.#state = 'paused'
    await this.#engine.pause()
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
    await this.#engine.seek(mediaTime)
    return seek.promise
  }

  override async destroy(): Promise<void> {
    await this.#engine.close()
    this.#seek?.reject(new PlayerError('ABORTED', 'The sink was destroyed', { element: 'playout' }))
    this.#seek = undefined
    this.#closeAudio()
    await super.destroy()
  }

  protected newPadState(): PlayoutPadState {
    return { properties: {}, ended: false, received: false }
  }

  protected async receiveMedia(
    _pad: Pad,
    state: PlayoutPadState,
    item: Exclude<PadItem, { type: 'properties' }>,
  ): Promise<void> {
    if (item.type !== 'frame') return
    const payload = item.frame.payload
    if (payload instanceof AudioData) {
      try {
        await this.#audioOutput(payload)
      } catch (cause) {
        payload.close()
        throw new PlayerError('MEDIA_FAILURE', 'Could not start the audio output', { element: 'playout', cause })
      }
    }
    await this.#engine.receive(payload)
    if (state.received) return
    state.received = true
    this.#maybeReady()
  }

  /** Liveness can't be checked in `negotiate`: pads are routed before their properties arrive. */
  protected override propertiesChanged(_state: PlayoutPadState, changes: Partial<PadProperties>): void {
    if (changes.live === false)
      throw new PlayerError('UNSUPPORTED_SOURCE', 'The playout sink plays live streams only', { element: 'playout' })
    if ('timescale' in changes) this.#setTracks()
  }

  protected padEvent(pad: Pad, _state: PlayoutPadState, event: PadEvent): void {
    if (event.type === 'eos') void this.#engine.end(roleOf(pad) as Role)
  }

  protected padSignal(pad: Pad, _state: PlayoutPadState, signal: PadSignal): void {
    if (signal.type === 'flush') void this.#engine.flush(roleOf(pad) as Role)
  }

  /** Created on the first `AudioData`, at its sample rate. */
  #audioOutput(data: AudioData): Promise<AudioOutput> {
    this.#audio ??= createAudioOutput(data.sampleRate, data.numberOfChannels).then(async (output) => {
      this.#output = output
      if (this.context.signal.aborted) {
        this.#closeAudio()
        return output
      }
      output.node.onprocessorerror = (error) =>
        this.fail(new PlayerError('MEDIA_FAILURE', 'The audio processor failed', { element: 'playout', cause: error }))
      await this.#engine.attachAudio(output.node.port)
      if (this.#state === 'playing') void output.context.resume()
      return output
    })
    return this.#audio
  }

  /** Video follows the audio clock when both are on the epoch clock. */
  #setTracks(): void {
    // shortcut: a Timescale's epoch is the application's (LOC 2.3.1.2), so such tracks play unsynced on the wall clock
    const roles = this.pads.map(roleOf)
    const audio = roles.includes('audio')
    const shared = this.padProperties().every((p) => p.timescale === undefined)
    void this.#engine.setTracks({ video: roles.includes('video'), audio, clock: audio && shared ? 'audio' : 'wall' })
  }

  #maybeReady(): void {
    if (this.#ready || !this.negotiated || this.states.size === 0) return
    if (![...this.states.values()].every((s) => s.received)) return
    this.#ready = true
    this.emit('ready')
  }

  #onReport(report: EngineReport): void {
    switch (report.type) {
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
        void this.#engine.outputTimestamp({ contextTime, time: performance.timeOrigin + performanceTime })
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
