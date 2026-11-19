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

import type { ProcessorMessage, ProcessorReport } from './worklet'

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
  /** Resolved and replaced whenever the render queue shrinks. */
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
  #audioStarved = false
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
    if (payload instanceof AudioData) this.#receiveAudio(payload)
    else await this.#receiveVideo(payload)
  }

  async outputTimestamp(timestamp: OutputTimestamp): Promise<void> {
    this.#output = timestamp
  }

  async end(role: Role): Promise<void> {
    this.#ended.add(role)
    this.#starving.delete(role)
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
  #receiveAudio(data: AudioData): void {
    const planes = Array.from({ length: data.numberOfChannels }, (_, planeIndex) => {
      const plane = new Float32Array(data.numberOfFrames)
      data.copyTo(plane, { planeIndex, format: 'f32-planar' })
      return plane
    })
    const timestamp = seconds(data.timestamp)
    const duration = seconds(data.duration)
    data.close()
    this.#audioEnd = timestamp + duration
    // shortcut: allocates per chunk on the audio thread; a SharedArrayBuffer ring when crossOriginIsolated avoids it, upgrade if glitches show up
    this.#post(
      { type: 'samples', planes, timestamp, duration },
      planes.map((p) => p.buffer),
    )
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
   * the next frame is due, and the wall clock starts at it.
   */
  #present(now: number | undefined): void {
    let due: VideoFrame | undefined
    if (now === undefined) {
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
    const audioDone = !this.#tracks.audio || (this.#ended.has('audio') && this.#audioStarved)
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
        this.#audioStarved = true
        this.#starved('audio')
        break
      case 'fed':
        this.#audioStarved = false
        this.#fed('audio')
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
