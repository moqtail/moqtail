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

import { logger } from '../../../util/logger'
import { SinkElement } from '../../element/base'
import type { ElementContext } from '../../element/context'
import { CapabilityCode } from '../../element/define'
import type { MediaSinkElement } from '../../element/interfaces'
import type { Pad, PadEvent, PadItem, PadProperties } from '../../element/pad'
import { PlayerError } from '../../error'
import { BufferQueue } from './buffer_queue'
import { GAP_CHECK_INTERVAL_MS, LIVE_EDGE_DELAY, gapJumpTarget, liveEdge, rangeAt } from './gaps'
import { BACK_BUFFER, FORWARD_CAP, append, bufferedEnd, groupStart, hasRoom, removeBuffered } from './retention'
import { SeekTracker } from './seek'

/** @public Options of the `MSE` sink, given through `player.sink(video, options)`. Seconds. */
export interface MSEOptions {
  video: HTMLVideoElement
  /** Kept behind `currentTime`. Default 30. */
  backBuffer?: number
  /** Most buffered ahead of `currentTime`, more than 0. VOD waits for room, live discards. Default 60. */
  forwardCap?: number
  /** How far behind the end of the buffer the sink's live jumps land. Default 0.75. */
  liveEdgeDelay?: number
}

/** What the sink keeps per sink pad. */
interface PadState {
  properties: PadProperties
  /** Set once the source buffers exist, cleared by a rebuild. */
  queue?: BufferQueue | undefined
  /** Appended before the pad's next packet. */
  initData?: Uint8Array | undefined
  /** Upstream seeks still waiting for their `discontinuity`. Packets are dropped meanwhile. */
  seeks: number
  /** Where the latest upstream seek resumes. Buffered data from there on gets replaced. */
  resumeAt: number
  /** Live only: where buffered data stops since packets were discarded past the forward cap. */
  discardedAt?: number | undefined
  ended: boolean
  /** Start times of the groups in the buffer, so a quota retry never cuts into the playing one. */
  keyframes: number[]
  /** The group of the last appended packet; unset when the next one doesn't continue it. */
  group?: bigint | undefined
}

/** Media element events the sink dispatches on itself. */
const RELAYED_EVENTS = ['playing', 'pause', 'stalled', 'waiting', 'timeupdate', 'seeked', 'ended']

/**
 * Drives an `HTMLVideoElement` through `ManagedMediaSource` or `MediaSource`: one
 * source buffer per pad, created together once negotiation is over, appends through
 * one queue per buffer, retention, seeking, gap jumping, and end of stream.
 */
export class MSESink extends SinkElement implements MediaSinkElement {
  readonly #target = new EventTarget()
  readonly addEventListener = this.#target.addEventListener.bind(this.#target)
  readonly removeEventListener = this.#target.removeEventListener.bind(this.#target)
  readonly dispatchEvent = this.#target.dispatchEvent.bind(this.#target)

  readonly #video: HTMLVideoElement
  readonly #Backend: typeof MediaSource
  readonly #seeks: SeekTracker
  readonly #pads = new Map<Pad, PadState>()
  readonly #remotePlayback: boolean
  readonly #timer: ReturnType<typeof setInterval>
  /** Read on every use, so a later source (catalog, buffer controller) can change them at runtime. */
  readonly #options: Required<Omit<MSEOptions, 'video'>>
  #source!: MediaSource
  #url: string | undefined
  /** Resolved once the source buffers exist; a rebuild replaces it. */
  #buffers = resolvers()
  #created = false
  #negotiated = false
  #live = false
  #duration: number | undefined
  /** Set once playback has its start position; a rebuild clears it. */
  #started = false
  /** Where playback continues after a rebuild, instead of the start position. */
  #resumeAt: number | undefined
  #rebuiltAfterFailure = false
  #fatal: PlayerError | undefined
  /** Resolved and replaced whenever something that may make room happens. */
  #changed = Promise.withResolvers<void>()

  constructor(context: ElementContext, options: MSEOptions, Backend: typeof MediaSource, managed: boolean) {
    super(context)
    const { video, backBuffer = BACK_BUFFER, forwardCap = FORWARD_CAP, liveEdgeDelay = LIVE_EDGE_DELAY } = options
    this.#options = { backBuffer, forwardCap, liveEdgeDelay }
    this.#video = video
    this.#Backend = Backend
    this.#seeks = new SeekTracker(video)
    this.#remotePlayback = video.disableRemotePlayback
    // WebKit won't open a ManagedMediaSource on an element that could use AirPlay without an alternative source.
    if (managed) video.disableRemotePlayback = true

    const on = (target: EventTarget, type: string, listener: () => void) =>
      target.addEventListener(type, listener, { signal: context.signal })
    for (const type of RELAYED_EVENTS) on(video, type, () => this.dispatchEvent(new Event(type)))
    for (const type of ['timeupdate', 'seeking', 'error']) on(video, type, this.#notify)
    for (const type of ['waiting', 'stalled']) on(video, type, () => this.#checkGap())
    on(video, 'seeking', () => this.#onSeeking())
    on(video, 'seeked', () => this.#seeks.seeked())
    on(video, 'error', () => video.error && this.#mediaFailed(this.#source, video.error))
    if (typeof document !== 'undefined') on(document, 'visibilitychange', () => this.#onVisible())
    // Not registered with the signal: those listeners are removed before `abort` fires.
    context.signal.addEventListener('abort', this.#notify, { once: true })
    // Only checks state, so a timer throttled in a hidden page just runs late.
    this.#timer = setInterval(() => this.#checkGap(), GAP_CHECK_INTERVAL_MS)

    this.on('padadded', (pad) => this.#padAdded(pad))
    this.on('padremoved', (pad) => this.#padRemoved(pad))
    this.on('nomorepads', () => {
      this.#negotiated = true
      this.#createBuffers()
    })
    this.#attach()
  }

  get currentTime(): number {
    return this.#video.currentTime
  }

  get buffered(): TimeRanges {
    return this.#video.buffered
  }

  get paused(): boolean {
    return this.#video.paused
  }

  get playbackRate(): number {
    return this.#video.playbackRate
  }

  setPlaybackRate(rate: number): void {
    this.#video.playbackRate = rate
  }

  play(): Promise<void> {
    return this.#video.play()
  }

  async pause(): Promise<void> {
    this.#video.pause()
  }

  /** Only sets `currentTime`; the element's `seeking` listener does the rest. */
  seek(mediaTime: number): Promise<void> {
    return this.#seeks.seek(mediaTime)
  }

  /** One pad per role. */
  negotiate(pad: Pad): boolean {
    return this.matchesCapabilities(pad) && !this.pads.some((p) => roleOf(p) === roleOf(pad))
  }

  async receive(pad: Pad, item: PadItem): Promise<void> {
    const state = this.#pads.get(pad)
    if (this.#fatal) throw this.#fatal
    if (!state) return
    if (item.type === 'properties') return this.#applyProperties(state, item.properties)
    const payload = item.object.payload
    if (state.seeks > 0 || !payload) return
    await this.#buffers.promise
    if (!(await this.#makeRoom(pad, state)) || state.seeks > 0 || !state.queue) return

    const source = this.#source
    const queue = state.queue
    const now = () => this.#video.currentTime
    const group = item.object.groupId
    const previousEnd = state.group === undefined ? undefined : bufferedEnd(queue.buffer)
    try {
      const init = state.initData
      state.initData = undefined
      const { backBuffer } = this.#options
      if (init) await append(queue, init, now, state.keyframes, backBuffer)
      await append(queue, payload, now, state.keyframes, backBuffer)
    } catch (error) {
      // Rebuilt meanwhile: the upstream seek fetches this packet again.
      if (queue !== state.queue || this.context.signal.aborted) return
      this.#mediaFailed(source, error)
      if (this.#fatal) throw this.#fatal
      return
    }
    // CMSF groups start at keyframes. An object that added no frames leaves nothing to record.
    if (group !== state.group && queue.buffer.buffered.length > 0) {
      const start = queue.buffer.buffered.start(0)
      state.keyframes = [...state.keyframes.filter((k) => k >= start), groupStart(queue.buffer, previousEnd)]
      state.group = group
    }
    this.#maybeStart()
  }

  override async destroy(): Promise<void> {
    clearInterval(this.#timer)
    this.#seeks.fail(new PlayerError('ABORTED', 'The sink was destroyed', { element: 'mse' }))
    this.#detach()
    this.#video.disableRemotePlayback = this.#remotePlayback
    await super.destroy()
  }

  readonly #notify = () => {
    this.#changed.resolve()
    this.#changed = Promise.withResolvers()
  }

  #padAdded(pad: Pad): void {
    if (pad.direction !== 'sink') return
    this.#pads.set(pad, { properties: {}, seeks: 0, resumeAt: 0, ended: false, keyframes: [] })
    pad.onEvent((event) => this.#onEvent(pad, event))
    if (this.#created) this.#rebuild()
  }

  #padRemoved(pad: Pad): void {
    if (!this.#pads.delete(pad) || this.context.signal.aborted) return
    if (this.#created) this.#rebuild()
    else this.#createBuffers()
  }

  async #applyProperties(state: PadState, changes: Partial<PadProperties>): Promise<void> {
    const previous = state.properties.mimeType
    state.properties = { ...state.properties, ...changes }
    if (changes.initData) state.initData = changes.initData
    if (changes.live !== undefined) this.#live = changes.live
    if (changes.duration !== undefined) {
      this.#duration = changes.duration
      void this.#applyDuration()
    }
    const mimeType = changes.mimeType
    if (state.queue && mimeType && mimeType !== previous) await state.queue.run((b) => b.changeType(mimeType))
    this.#createBuffers()
  }

  /** Creates every source buffer together, once negotiation is over and each pad has its `mimeType`. */
  #createBuffers(): void {
    const states = [...this.#pads.values()]
    if (this.#created || !this.#negotiated || this.#source.readyState !== 'open') return
    if (states.length === 0 || states.some((s) => !s.properties.mimeType)) return
    this.#created = true
    try {
      for (const state of states)
        state.queue = new BufferQueue(this.#source.addSourceBuffer(state.properties.mimeType!))
    } catch (cause) {
      this.#buffers.reject(
        new PlayerError('MEDIA_FAILURE', 'Could not create a source buffer', { element: 'mse', cause }),
      )
      return
    }
    void this.#applyDuration()
    this.#buffers.resolve()
  }

  /**
   * Whether to append the pad's next packet. VOD waits until the forward buffer drops
   * under the cap, which backs up the connection queue. Live never waits: past the cap it
   * discards, and once there's room again it seeks upstream to where the data stops.
   */
  async #makeRoom(pad: Pad, state: PadState): Promise<boolean> {
    if (this.#live) {
      if (!this.#hasRoom(state)) {
        state.discardedAt ??= bufferedEnd(state.queue!.buffer) ?? this.#video.currentTime
        return false
      }
      if (state.discardedAt === undefined) return true
      this.#seekUpstream(pad, state, state.discardedAt)
      return false
    }
    while (!this.#hasRoom(state) && state.seeks === 0) {
      await this.#changed.promise
      this.context.signal.throwIfAborted()
      if (this.#fatal) throw this.#fatal
    }
    return true
  }

  /** `endstreaming` counts as reaching the cap. Before the start position there's always room. */
  #hasRoom(state: PadState): boolean {
    const streaming = (this.#source as { streaming?: boolean }).streaming !== false
    return (
      !this.#started ||
      !state.queue ||
      (streaming && hasRoom(state.queue.buffer, this.#video.currentTime, this.#options.forwardCap))
    )
  }

  /** Once every source buffer has data, jumps to the latest of their starts (or the rebuild's position). */
  #maybeStart(): void {
    if (this.#started || !this.#created) return
    const buffers = [...this.#pads.values()].map((s) => s.queue!.buffer)
    if (buffers.some((b) => b.buffered.length === 0)) return
    this.#started = true
    this.#seeks.jump(this.#resumeAt ?? Math.max(...buffers.map((b) => b.buffered.start(0))))
    this.#resumeAt = undefined
    this.emit('ready')
  }

  #onEvent(pad: Pad, event: PadEvent): boolean {
    const state = this.#pads.get(pad)
    if (!state) return true
    if (event.type === 'eos') {
      state.ended = true
      void this.#endOfStream()
    } else if (event.type === 'discontinuity' && event.reason === 'gap') {
      this.#checkGap()
    } else if (event.type === 'discontinuity' && state.seeks > 0) {
      this.#seekAnswered(state, event.failed === true)
    }
    return true
  }

  /** A seek the sink didn't make: upstream unless the target is buffered and no upstream seek is pending. */
  #onSeeking(): void {
    if (!this.#seeks.seeking()) return
    const target = this.#video.currentTime
    const pending = [...this.#pads.values()].some((s) => s.seeks > 0)
    if (!pending && rangeAt(target, this.#video.buffered) !== -1) return
    for (const [pad, state] of this.#pads) this.#seekUpstream(pad, state, target)
  }

  /** The source answers every `seek` with one `discontinuity`, so counting them needs no seek identifier. */
  #seekUpstream(pad: Pad, state: PadState, mediaTime: number): void {
    state.seeks++
    state.resumeAt = mediaTime
    state.ended = false
    state.discardedAt = undefined
    pad.sendEvent({ type: 'seek', mediaTime })
    this.#notify()
  }

  #seekAnswered(state: PadState, failed: boolean): void {
    if (failed) {
      state.seeks = 0
      this.#seeks.fail(new PlayerError('SEEK_FAILED', 'A source could not seek', { element: 'mse' }))
      return
    }
    if (--state.seeks > 0) return
    const from = state.resumeAt
    state.keyframes = state.keyframes.filter((k) => k < from)
    state.group = undefined
    // A failure here means the buffer is gone, which the next append reports.
    state.queue?.run((b) => removeBuffered(b, from, Infinity)).catch(() => {})
  }

  #checkGap(): void {
    const video = this.#video
    if (!this.#started || video.seeking) return
    const target = gapJumpTarget(video.currentTime, video.buffered, this.#live, this.#options.liveEdgeDelay)
    if (target !== undefined && target > video.currentTime) this.#seeks.jump(target)
  }

  /** A live sink that fell behind the seekable range jumps near the end of `buffered`. A paused one stays paused. */
  #onVisible(): void {
    const video = this.#video
    if (document.visibilityState !== 'visible' || !this.#live || !this.#started || !video.buffered.length) return
    const seekable = video.seekable.length ? video.seekable : video.buffered
    if (video.currentTime < seekable.start(0)) this.#seeks.jump(liveEdge(video.buffered, this.#options.liveEdgeDelay))
  }

  /** Ends the stream once every pad received `eos`, behind pending appends. */
  async #endOfStream(): Promise<void> {
    const states = () => [...this.#pads.values()]
    if (!this.#created || !states().every((s) => s.ended)) return
    const source = this.#source
    await Promise.all(states().map((s) => s.queue?.run(() => {})))
    if (source !== this.#source || source.readyState !== 'open' || !states().every((s) => s.ended)) return
    try {
      source.endOfStream()
    } catch (error) {
      logger.warn('MSESink', 'endOfStream failed', error)
    }
  }

  /** Sets a VOD duration once no source buffer is updating. */
  async #applyDuration(): Promise<void> {
    const queues = [...this.#pads.values()].flatMap((s) => s.queue ?? [])
    const duration = this.#duration
    if (this.#live || duration === undefined || queues.length === 0) return
    const source = this.#source
    await Promise.all(queues.map((q) => q.run(() => {})))
    try {
      if (source.readyState === 'open') source.duration = duration
    } catch (error) {
      logger.warn('MSESink', 'Could not set the duration', error) // an append started meanwhile, or data past it
    }
  }

  /**
   * An append or decode failure. The first rebuilds the media source, a second is fatal.
   * Failures of a media source that was already replaced are ignored.
   */
  #mediaFailed(source: MediaSource, cause: unknown): void {
    if (source !== this.#source || this.#fatal || this.context.signal.aborted) return
    if (this.#rebuiltAfterFailure) {
      // shortcut: a fatal media error event only surfaces on the next receive(), fine while sources keep writing.
      this.#fatal = new PlayerError('MEDIA_FAILURE', 'Playback failed again after a rebuild', { element: 'mse', cause })
      this.#notify()
      return
    }
    logger.warn('MSESink', 'Rebuilding the media source after a failure', cause)
    this.#rebuiltAfterFailure = true
    this.#rebuild()
  }

  /** A new media source with the current pads, resuming from `currentTime` through an upstream seek. */
  #rebuild(): void {
    const resumeAt = this.#started ? this.#video.currentTime : undefined
    const playing = !this.#video.paused
    this.#detach()
    this.#seeks.reset()
    this.#created = false
    this.#started = false
    this.#resumeAt = resumeAt
    this.#buffers.resolve() // receives still waiting on the old buffers see no queue and drop their packet
    this.#buffers = resolvers()
    for (const [pad, state] of this.#pads) {
      state.queue = undefined
      state.initData = state.properties.initData
      state.keyframes = []
      state.group = undefined
      if (resumeAt !== undefined) this.#seekUpstream(pad, state, resumeAt)
    }
    this.#attach()
    if (playing) this.#video.play().catch(() => {})
    this.#notify()
  }

  #attach(): void {
    const source = new this.#Backend()
    this.#source = source
    const signal = this.context.signal
    source.addEventListener('sourceopen', () => this.#createBuffers(), { signal })
    for (const type of ['startstreaming', 'endstreaming', 'bufferedchange']) {
      source.addEventListener(type, this.#notify, { signal })
    }
    try {
      this.#video.srcObject = source // ManagedMediaSource in Safari requires it
    } catch {
      this.#url = URL.createObjectURL(source)
      this.#video.src = this.#url
    }
  }

  #detach(): void {
    this.#video.removeAttribute('src')
    this.#video.srcObject = null
    if (this.#url) URL.revokeObjectURL(this.#url)
    this.#url = undefined
    this.#video.load()
  }
}

function roleOf(pad: Pad): string | undefined {
  return pad.capabilities.find((c) => c.code === CapabilityCode.ROLE)?.value
}

/** Resolvers whose rejection is handled, since nobody may be waiting when it rejects. */
function resolvers(): PromiseWithResolvers<void> {
  const r = Promise.withResolvers<void>()
  r.promise.catch(() => {})
  return r
}
