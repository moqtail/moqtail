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

import { PREROLL, type ProcessorMessage, type ProcessorReport } from './worklet'

export type Role = 'video' | 'audio'

/** Which tracks play, and which clock video follows. */
export interface PlayoutTracks {
  video: boolean
  audio: boolean
  clock: 'audio' | 'wall'
}

/** What the engine tells the element. Times in media seconds. */
export type EngineReport =
  | { type: 'presentation'; currentTime: number | undefined; bufferedEnd: number | undefined }
  | { type: 'playing' }
  | { type: 'waiting' }
  | { type: 'seeked' }
  | { type: 'ended' }
  | { type: 'error'; error: unknown }

/** `getOutputTimestamp()` with its `performanceTime` made absolute, `performance.timeOrigin` added. */
export interface OutputTimestamp {
  contextTime: number
  time: number
}

/** A clock's media time at a reference time, and the processor's `gap - drift` then. */
interface Anchor {
  media: number
  reference: number
  skew: number
}

/**
 * Frames the render queue holds before `receive()` waits. Each one pins a decoder buffer: Chromium's
 * software decoder kept 200 open 720p frames alive, and 60 is liveBuffer's default 1 s at 60 fps.
 * shortcut: unmeasured on hardware decoders, lower it if one stalls or fails below it
 */
export const RENDER_QUEUE_LIMIT = 60
/** Seconds of audio posted and not yet consumed before `receive()` waits: a context the page won't start consumes none. */
const AUDIO_QUEUE_LIMIT = 2

/**
 * Renders what the element hands it: video on a canvas on each animation frame, and audio by
 * posting samples to the playout processor, whose reports drive the audio clock. Video follows
 * that clock while it runs and the tracks share it, a wall clock otherwise. It's driven only by
 * calls the element awaits and reports through one callback, and uses only what a dedicated
 * worker has.
 */
export class PlayoutEngine {
  readonly #canvas: CanvasRenderingContext2D | OffscreenCanvasRenderingContext2D
  readonly #report: (report: EngineReport) => void
  /** The render queue, oldest first. */
  readonly #frames: VideoFrame[] = []
  readonly #ended = new Set<Role>()
  /** The processor's `drift` reports not heard yet, oldest first. */
  readonly #skews: { contextTime: number; skew: number }[] = []
  readonly #starving = new Set<Role>()
  #port: MessagePort | undefined
  #tracks: PlayoutTracks = { video: false, audio: false, clock: 'wall' }
  #state: 'idle' | 'playing' | 'paused' = 'idle'
  #rate = 1
  /** Resolved and replaced whenever the render queue or the audio queue shrinks. */
  #room = Promise.withResolvers<void>()
  #loop: number | undefined
  /** Media time of the frame on screen. */
  #onScreen: number | undefined
  /** The latest frame received, and its distance to the one before. */
  #latest: number | undefined
  #interval: number | undefined
  #audioEnd: number | undefined
  #audio: Anchor | undefined
  #wall: Anchor | undefined
  #output: OutputTimestamp | undefined
  #skew = 0
  /** Media seconds of audio posted, and of it consumed as the processor reported. */
  #audioPosted = 0
  #audioConsumed = 0
  #seekTarget: number | undefined
  #playingReported = false
  #done = false

  constructor(
    canvas: CanvasRenderingContext2D | OffscreenCanvasRenderingContext2D,
    report: (report: EngineReport) => void,
  ) {
    this.#canvas = canvas
    this.#report = report
  }

  async attachAudio(port: MessagePort): Promise<void> {
    this.#port = port
    port.onmessage = (event: MessageEvent<ProcessorReport>) => this.#processorReport(event.data)
    this.#post({ type: 'rate', rate: this.#rate })
  }

  async setTracks(tracks: PlayoutTracks): Promise<void> {
    this.#tracks = tracks
  }

  async receive(payload: VideoFrame | AudioData): Promise<void> {
    if (this.#state === 'paused' || this.#done) {
      payload.close()
      return
    }
    if (payload instanceof AudioData) await this.#receiveAudio(payload)
    else await this.#receiveVideo(payload)
  }

  async outputTimestamp(timestamp: OutputTimestamp): Promise<void> {
    this.#output = timestamp
  }

  async end(role: Role): Promise<void> {
    this.#ended.add(role)
    this.#starving.delete(role)
    if (role === 'audio') this.#post({ type: 'end' })
  }

  /** Drops what the role has queued: what comes next replaces it. */
  async flush(role: Role): Promise<void> {
    if (role === 'audio') {
      this.#post({ type: 'flush' })
      this.#audio = undefined
      this.#skews.length = 0
      this.#audioEnd = undefined
      return
    }
    for (const frame of this.#frames.splice(0)) frame.close()
    this.#latest = undefined
    this.#interval = undefined
    this.#roomMade()
  }

  /** Resuming after a pause starts over at the live edge. */
  async play(): Promise<void> {
    if (this.#state === 'playing') return
    if (this.#state === 'paused') {
      await this.flush('video')
      await this.flush('audio')
      this.#wall = undefined
      this.#output = undefined
      this.#starving.clear()
    }
    this.#state = 'playing'
    this.#playingReported = false
    this.#loop = requestAnimationFrame(this.#tick)
  }

  /** The picture stays; what's queued goes, so the decoder keeps outputting. */
  async pause(): Promise<void> {
    this.#state = 'paused'
    this.#stopLoop()
    await this.flush('video')
  }

  /** Forward only: drops what's before `time`, and the clock starts again at the first media from there. */
  async seek(time: number): Promise<void> {
    this.#seekTarget = time
    let dropped = 0
    while (this.#frames[0] && seconds(this.#frames[0].timestamp) < time) {
      this.#frames.shift()!.close()
      dropped++
    }
    if (dropped > 0) this.#roomMade()
    this.#post({ type: 'seek', time })
    this.#audio = undefined
    this.#skews.length = 0
    this.#wall = undefined
    // Paused, nothing is presented until play(), which starts over at the live edge
    if (this.#state === 'paused') {
      this.#seekTarget = undefined
      this.#report({ type: 'seeked' })
    }
  }

  /** The clocks go on from where they are, `rate` times as fast as their reference. */
  async setPlaybackRate(rate: number): Promise<void> {
    const heard = this.#heard()
    if (this.#audio && heard !== undefined) {
      this.#audio = { media: this.#audioTime(heard), reference: heard, skew: this.#skew }
    }
    const now = performance.now() / 1000
    if (this.#wall) this.#wall = { media: clockAt(this.#wall, this.#rate, now, 0), reference: now, skew: 0 }
    this.#rate = rate
    this.#post({ type: 'rate', rate })
  }

  async close(): Promise<void> {
    this.#stopLoop()
    this.#done = true
    for (const frame of this.#frames.splice(0)) frame.close()
    this.#roomMade()
    if (this.#port) this.#port.onmessage = null
  }

  async #receiveVideo(frame: VideoFrame): Promise<void> {
    const time = seconds(frame.timestamp)
    if (this.#seekTarget !== undefined && time < this.#seekTarget) {
      frame.close()
      return
    }
    if (this.#latest !== undefined && time > this.#latest) this.#interval = time - this.#latest
    this.#latest = time
    this.#fed('video')
    if (this.#state === 'idle' && this.#onScreen === undefined) {
      // The first picture, so the sink is ready with one
      this.#draw(frame)
      this.#presentation()
      return
    }
    this.#frames.push(frame)
    while (this.#frames.length >= RENDER_QUEUE_LIMIT && !this.#done) await this.#room.promise
  }

  /** Planes copied out and transferred: the processor gets self-contained messages. */
  async #receiveAudio(data: AudioData): Promise<void> {
    const planes = Array.from({ length: data.numberOfChannels }, (_, planeIndex) => {
      const plane = new Float32Array(data.numberOfFrames)
      data.copyTo(plane, { planeIndex, format: 'f32-planar' })
      return plane
    })
    const timestamp = seconds(data.timestamp)
    const duration = seconds(data.duration)
    data.close()
    this.#audioEnd = timestamp + duration
    this.#audioPosted += duration
    // shortcut: allocates per chunk on the audio thread; a SharedArrayBuffer ring when crossOriginIsolated avoids it, upgrade if glitches show up
    this.#post(
      { type: 'samples', planes, timestamp, duration },
      planes.map((p) => p.buffer),
    )
    while (this.#state !== 'paused' && !this.#done && this.#audioQueued() >= AUDIO_QUEUE_LIMIT) {
      await this.#room.promise
    }
  }

  readonly #tick = (): void => {
    this.#loop = requestAnimationFrame(this.#tick)
    try {
      const now = this.#now()
      this.#present(now)
      this.#checkVideoStall(now)
      this.#presentation()
      this.#checkEnded()
    } catch (error) {
      this.#stopLoop()
      this.#report({ type: 'error', error })
    }
  }

  /**
   * Draws the latest due frame and closes the ones before it: drawing one per animation frame
   * falls further behind whenever frames come faster than the display refreshes. Without a clock
   * the next frame is due once `PREROLL` is queued, and the wall clock starts at it.
   */
  #present(now: number | undefined): void {
    let due: VideoFrame | undefined
    if (now === undefined) {
      const [first, last] = [this.#frames[0], this.#frames.at(-1)]
      const prerolled =
        this.#ended.has('video') ||
        this.#frames.length >= RENDER_QUEUE_LIMIT ||
        (first && last && seconds(last.timestamp - first.timestamp) >= PREROLL)
      if (!prerolled) return
      due = this.#frames.shift()
      if (due) this.#wall = { media: seconds(due.timestamp), reference: performance.now() / 1000, skew: 0 }
    } else {
      while (this.#frames[0] && seconds(this.#frames[0].timestamp) <= now) {
        due?.close()
        due = this.#frames.shift()
      }
    }
    if (!due) return
    this.#roomMade()
    this.#draw(due)
    this.#presented()
  }

  #draw(frame: VideoFrame): void {
    const canvas = this.#canvas.canvas
    if (canvas.width !== frame.displayWidth || canvas.height !== frame.displayHeight) {
      canvas.width = frame.displayWidth
      canvas.height = frame.displayHeight
    }
    this.#onScreen = seconds(frame.timestamp)
    try {
      this.#canvas.drawImage(frame, 0, 0)
    } finally {
      frame.close()
    }
  }

  /** Media at the seek target, or the first after `play()`, is playing. */
  #presented(): void {
    if (this.#seekTarget !== undefined) {
      this.#seekTarget = undefined
      this.#report({ type: 'seeked' })
    }
    if (!this.#playingReported) {
      this.#playingReported = true
      this.#report({ type: 'playing' })
    }
  }

  /** Video ran out once the clock is two frame intervals past the latest frame and nothing newer is queued. */
  #checkVideoStall(now: number | undefined): void {
    const latest = this.#latest
    const interval = this.#interval
    if (now === undefined || latest === undefined || interval === undefined || this.#frames.length > 0) return
    // shortcut: frame spacing misreads content whose frame rate drops on purpose, read the catalog's framerate once such content plays
    if (now - latest > 2 * interval) this.#starved('video')
  }

  #checkEnded(): void {
    if (!this.#tracks.video && !this.#tracks.audio) return
    const videoDone = !this.#tracks.video || (this.#ended.has('video') && this.#frames.length === 0)
    // A context that never ran, as one the page won't start, plays nothing more
    const audioDone =
      !this.#tracks.audio || (this.#ended.has('audio') && (this.#audioQueued() === 0 || this.#output === undefined))
    if (!videoDone || !audioDone) return
    this.#stopLoop()
    this.#done = true
    this.#report({ type: 'ended' })
  }

  #presentation(): void {
    const heard = this.#heard()
    const audioTime = heard === undefined ? undefined : this.#audioTime(heard)
    const ends = [this.#latest === undefined ? undefined : this.#latest + (this.#interval ?? 0), this.#audioEnd]
    const known = ends.filter((end) => end !== undefined)
    this.#report({
      type: 'presentation',
      currentTime: this.#seekTarget ?? audioTime ?? this.#onScreen,
      bufferedEnd: known.length ? Math.min(...known) : undefined,
    })
  }

  #processorReport(report: ProcessorReport): void {
    switch (report.type) {
      case 'start':
        this.#skew = 0
        this.#skews.length = 0
        this.#audio = { media: report.mediaTime, reference: report.contextTime, skew: 0 }
        if (!this.#tracks.video) this.#presented()
        break
      case 'drift': {
        this.#skews.push({ contextTime: report.contextTime, skew: report.gap - report.drift })
        // Drained here too: animation frames stop in a hidden tab, and nothing reads them on the wall clock
        const heard = this.#heard()
        if (heard !== undefined) this.#audioTime(heard)
        else if (this.#tracks.clock !== 'audio') this.#skews.length = 0
        break
      }
      case 'starved':
        this.#starved('audio')
        break
      case 'fed':
        this.#fed('audio')
        break
      case 'consumed':
        this.#audioConsumed = report.seconds
        this.#roomMade()
        break
    }
  }

  /** The audio clock while it runs and the tracks share it, else the wall clock once it started. */
  #now(): number | undefined {
    const heard = this.#heard()
    if (heard !== undefined) return this.#audioTime(heard)
    return this.#wall && clockAt(this.#wall, this.#rate, performance.now() / 1000, 0)
  }

  /** The context time heard now, while the audio clock runs and video follows it. */
  #heard(): number | undefined {
    const output = this.#output
    if (this.#tracks.clock !== 'audio' || !this.#audio || !output) return undefined
    return output.contextTime + (performance.timeOrigin + performance.now() - output.time) / 1000
  }

  /** The processor counts silence and gaps as it renders them: they apply once heard. */
  #audioTime(heard: number): number {
    while (this.#skews[0] && this.#skews[0].contextTime <= heard) this.#skew = this.#skews.shift()!.skew
    return clockAt(this.#audio!, this.#rate, heard, this.#skew)
  }

  /** No stalls while paused, before the first `playing`, or on a role that ended. */
  #starved(role: Role): void {
    if (this.#state !== 'playing' || !this.#playingReported || this.#ended.has(role) || this.#starving.has(role)) return
    this.#starving.add(role)
    if (this.#starving.size === 1) this.#report({ type: 'waiting' })
  }

  #fed(role: Role): void {
    if (this.#starving.delete(role) && this.#starving.size === 0) this.#report({ type: 'playing' })
  }

  #audioQueued(): number {
    return this.#audioPosted - this.#audioConsumed
  }

  #roomMade(): void {
    this.#room.resolve()
    this.#room = Promise.withResolvers()
  }

  #post(message: ProcessorMessage, transfer: Transferable[] = []): void {
    this.#port?.postMessage(message, transfer)
  }

  #stopLoop(): void {
    if (this.#loop !== undefined) cancelAnimationFrame(this.#loop)
    this.#loop = undefined
  }
}

/** Media time on `anchor`'s clock at `reference`, with the processor's `gap - drift` at `skew`. */
function clockAt(anchor: Anchor, rate: number, reference: number, skew: number): number {
  return anchor.media + rate * (reference - anchor.reference) + skew - anchor.skew
}

/** WebCodecs timestamps and durations are µs. */
function seconds(micros: number): number {
  return micros / 1_000_000
}

if (import.meta.vitest) {
  const { test, expect, beforeEach, afterEach, vi } = import.meta.vitest
  const { settle } = await import('@/player/testing/elements')
  let now: number
  let frame: (() => void) | undefined
  let reports: EngineReport[]
  let posted: ProcessorMessage[]

  class FakeAudioData {
    readonly numberOfChannels = 1
    readonly numberOfFrames = 1
    readonly timestamp: number
    readonly duration: number
    constructor(time: number, duration: number) {
      this.timestamp = time * 1_000_000
      this.duration = duration * 1_000_000
    }
    copyTo() {}
    close() {}
  }

  beforeEach(() => {
    now = 1000
    frame = undefined
    reports = []
    posted = []
    vi.spyOn(performance, 'now').mockImplementation(() => now)
    vi.stubGlobal('requestAnimationFrame', (callback: () => void) => {
      frame = callback
      return 1
    })
    vi.stubGlobal('cancelAnimationFrame', () => {
      frame = undefined
    })
    vi.stubGlobal('AudioData', FakeAudioData)
  })

  afterEach(() => {
    vi.restoreAllMocks()
    vi.unstubAllGlobals()
  })

  /** A playing engine on the audio clock, and a way to send it processor reports. */
  async function playing(tracks: Partial<PlayoutTracks> = {}) {
    const canvas = { canvas: { width: 0, height: 0 }, drawImage: () => {} } as unknown as CanvasRenderingContext2D
    const engine = new PlayoutEngine(canvas, (report) => reports.push(report))
    const port = {
      onmessage: null as ((event: { data: ProcessorReport }) => void) | null,
      postMessage: (message: ProcessorMessage) => posted.push(message),
    }
    await engine.attachAudio(port as unknown as MessagePort)
    await engine.setTracks({ video: true, audio: true, clock: 'audio', ...tracks })
    await engine.play()
    const processor = (report: ProcessorReport) => port.onmessage!({ data: report })
    /** The context is heard at `contextTime` now, and runs with the wall clock. */
    const running = (contextTime: number) =>
      engine.outputTimestamp({ contextTime, time: performance.timeOrigin + performance.now() })
    return { engine, processor, running }
  }

  const audio = (time: number, duration = 0.5) => new FakeAudioData(time, duration) as unknown as AudioData
  /** The current time an animation frame presents. */
  const presented = () => {
    frame!()
    return reports.filter((report) => report.type === 'presentation').at(-1)!.currentTime
  }
  const ended = () => reports.some((report) => report.type === 'ended')

  test('a skew applies once its quantum is heard, and a rate change re-anchors the clock where it is', async () => {
    const { engine, processor, running } = await playing()
    processor({ type: 'start', contextTime: 5, mediaTime: 100 })
    await running(5)
    expect(presented()).toBeCloseTo(100, 5)
    processor({ type: 'drift', contextTime: 5.6, drift: 0.2, gap: 0 })
    now += 500
    expect(presented()).toBeCloseTo(100.5, 5)
    now += 200
    expect(presented()).toBeCloseTo(100.5, 5)
    await engine.setPlaybackRate(2)
    expect(presented()).toBeCloseTo(100.5, 5)
    now += 100
    expect(presented()).toBeCloseTo(100.7, 5)
  })

  test('a seek holds the time at its target until the processor starts there, and settles at once while paused', async () => {
    const { engine, processor, running } = await playing({ video: false })
    processor({ type: 'start', contextTime: 5, mediaTime: 100 })
    await running(5)
    now += 1000
    expect(presented()).toBeCloseTo(101, 5)
    await engine.seek(103)
    expect(posted.at(-1)).toEqual({ type: 'seek', time: 103 })
    expect(presented()).toBe(103)
    processor({ type: 'start', contextTime: 6, mediaTime: 103 })
    expect(reports.filter((report) => report.type === 'seeked')).toHaveLength(1)
    now += 100
    expect(presented()).toBeCloseTo(103.1, 5)
    await engine.pause()
    await engine.seek(104)
    expect(reports.at(-1)).toEqual({ type: 'seeked' })
  })

  test('audio past the queue limit holds receive() until the processor consumes some, or a pause', async () => {
    const { engine, processor } = await playing()
    for (const time of [0, 0.5, 1]) await engine.receive(audio(time))
    let held = true
    const fourth = engine.receive(audio(1.5)).then(() => {
      held = false
    })
    await settle()
    expect(held).toBe(true)
    processor({ type: 'consumed', seconds: 0.5 })
    await fourth
    const fifth = engine.receive(audio(2))
    await engine.pause()
    await fifth
  })

  test('ended waits for the audio queued on a running context, but not on one that never ran', async () => {
    const never = await playing()
    await never.engine.receive(audio(0))
    for (const role of ['video', 'audio'] as const) await never.engine.end(role)
    frame!()
    expect(ended()).toBe(true)

    reports = []
    const { engine, processor, running } = await playing()
    await engine.receive(audio(0))
    await running(5)
    for (const role of ['video', 'audio'] as const) await engine.end(role)
    frame!()
    expect(ended()).toBe(false)
    processor({ type: 'consumed', seconds: 0.5 })
    frame!()
    expect(ended()).toBe(true)
  })
}
