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

import { logger } from '@/util/logger'
import { SinkElement } from '@/player/element/base'
import type { ElementContext } from '@/player/element/context'
import { CapabilityCode } from '@/player/element/define'
import type { MediaSinkElement } from '@/player/element/interfaces'
import type { Pad, PadEvent, PadItem, PadProperties, PadSignal } from '@/player/element/pad'
import { PlayerError } from '@/player/error'
import { handledResolvers } from '@/player/util/async'
import { BufferQueue } from './buffer_queue'
import { GAP_CHECK_INTERVAL_MS, LIVE_EDGE_DELAY, gapJumpTarget, liveEdge, rangeAt } from './gaps'
import { liveEdgeStart, liveSeekable, vodDuration } from './range'
import {
  BACK_BUFFER,
  FORWARD_CAP,
  append,
  bufferedAhead,
  bufferedEnd,
  groupStart,
  hasRoom,
  isQuotaExceeded,
  removeBuffered,
} from './retention'
import { SeekTracker } from './seek'

/** @public Options of the `MSE` sink, given through `player.sink(video, options)`. Seconds. */
export interface MSEOptions {
  video: HTMLVideoElement
  /** Kept behind `currentTime`. Default 90. */
  backBuffer?: number
  /** VOD only: most buffered ahead of `currentTime`, more than 0; appends wait for room. Live keeps everything ahead. Default 60. */
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
  /** Upstream seeks sent so far: a packet that waited for room across one is from before it. */
  epoch: number
  /** Where the latest upstream seek's data replaces what's buffered: removed from there on once answered. */
  replaceFrom: number
  /** Live only: seconds buffered ahead when an append hit the quota with nothing behind to free. */
  quotaAhead?: number | undefined
  ended: boolean
  /** Start times of the groups in the buffer, so a quota retry never cuts into the playing one. */
  keyframes: number[]
  /** The group of the last appended packet; unset when the next one doesn't continue it. */
  group?: bigint | undefined
  /**
   * After a `properties` item changed the track, where the new track starts: the old track's
   * data before it is removed once playback gets there. `'next group'` until its first group
   * is appended.
   */
  replacedBefore?: 'next group' | number | undefined
}

/** How far before a new track's first group the removal of the old track's data stops, in seconds. */
const SPLICE_TOLERANCE = 0.1

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
  readonly #seekTracker: SeekTracker
  readonly #pads = new Map<Pad, PadState>()
  readonly #remotePlayback: boolean
  readonly #timer: ReturnType<typeof setInterval>
  readonly #options: Required<Omit<MSEOptions, 'video'>>
  #source!: MediaSource
  #url: string | undefined
  /** Resolved once the source buffers exist; a rebuild replaces it. */
  #buffers = handledResolvers()
  #created = false
  #negotiated = false
  #live = false
  /** The live seekable range last set. */
  #seekable: { start: number; end: number } | undefined
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
    this.#seekTracker = new SeekTracker(video)
    this.#remotePlayback = video.disableRemotePlayback
    // WebKit won't open a ManagedMediaSource on an element that could use AirPlay without an alternative source
    if (managed) video.disableRemotePlayback = true

    const on = (target: EventTarget, type: string, listener: () => void) =>
      target.addEventListener(type, listener, { signal: context.signal })
    for (const type of RELAYED_EVENTS) on(video, type, () => this.dispatchEvent(new Event(type)))
    for (const type of ['timeupdate', 'seeking', 'error']) on(video, type, this.#notify)
    for (const type of ['waiting', 'stalled']) on(video, type, () => this.#checkGap())
    on(video, 'seeking', () => this.#onSeeking())
    on(video, 'seeked', () => this.#seekTracker.seeked())
    on(video, 'error', () => {
      if (video.error) this.#mediaFailed(this.#source, video.error)
    })
    if (typeof document !== 'undefined') on(document, 'visibilitychange', () => this.#onVisible())
    // Not registered with the signal: those listeners are removed before `abort` fires
    context.signal.addEventListener('abort', this.#notify, { once: true })
    // Only checks state, so a timer throttled in a hidden page just runs late
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

  get seeking(): boolean {
    return this.#video.seeking
  }

  get ended(): boolean {
    return this.#video.ended
  }

  get playbackRate(): number {
    return this.#video.playbackRate
  }

  get liveEdgeStart(): number {
    if (!this.#live || !this.#seekable) return NaN
    return liveEdgeStart(this.#seekable.end, this.#properties())
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
    return this.#seekTracker.seek(mediaTime)
  }

  /** One pad per role. */
  negotiate(pad: Pad): boolean {
    return this.matchesCapabilities(pad) && !this.pads.some((p) => roleOf(p) === roleOf(pad))
  }

  async receive(pad: Pad, item: PadItem): Promise<void> {
    if (this.#fatal) throw this.#fatal
    const state = this.#pads.get(pad)
    if (!state) return
    if (item.type === 'properties') return this.#applyProperties(state, item.properties)
    const payload = item.object.payload
    if (state.seeks > 0 || !payload) return
    const epoch = state.epoch
    await this.#buffers.promise
    if (!(await this.#makeRoom(pad, state, epoch)) || state.epoch !== epoch || !state.queue) return

    const queue = state.queue
    const previousEnd = state.group === undefined ? undefined : bufferedEnd(queue.buffer)
    if (!(await this.#append(state, queue, payload))) return
    // CMSF groups start at keyframes. An object that added no frames leaves nothing to record.
    const group = item.object.groupId
    if (group !== state.group && queue.buffer.buffered.length > 0) this.#recordGroup(state, queue, group, previousEnd)
    this.#updateSeekable()
    this.#maybeStart()
  }

  /** Appends the pending init data, then `payload`. False when nothing was appended and playback goes on. */
  async #append(state: PadState, queue: BufferQueue, payload: Uint8Array): Promise<boolean> {
    const source = this.#source
    const now = () => this.#video.currentTime
    try {
      const init = state.initData
      state.initData = undefined
      const { backBuffer } = this.#options
      if (init) await append(queue, init, now, state.keyframes, backBuffer)
      await append(queue, payload, now, state.keyframes, backBuffer)
      return true
    } catch (error) {
      // Rebuilt meanwhile: the upstream seek fetches this packet again
      if (queue !== state.queue || this.context.signal.aborted) return false
      if (this.#live && isQuotaExceeded(error)) {
        state.quotaAhead = bufferedAhead(queue.buffer, now())
        return false
      }
      this.#mediaFailed(source, error)
      if (this.#fatal) throw this.#fatal
      return false
    }
  }

  /** The keyframe a new group starts at, which a back buffer trim keeps. */
  #recordGroup(state: PadState, queue: BufferQueue, group: bigint, previousEnd: number | undefined): void {
    const start = queue.buffer.buffered.start(0)
    const keyframe = groupStart(queue.buffer, previousEnd)
    state.keyframes = [...state.keyframes.filter((k) => k >= start), keyframe]
    state.group = group
    if (state.replacedBefore === 'next group') {
      state.replacedBefore = keyframe
      this.#dropReplaced()
    }
  }

  override async destroy(): Promise<void> {
    clearInterval(this.#timer)
    this.#seekTracker.fail(new PlayerError('ABORTED', 'The sink was destroyed', { element: 'mse' }))
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
    this.#pads.set(pad, { properties: {}, seeks: 0, epoch: 0, replaceFrom: 0, ended: false, keyframes: [] })
    pad.onEvent((event) => this.#onEvent(pad, event))
    pad.onSignal((signal) => this.#onSignal(pad, signal))
    if (this.#created) this.#rebuild()
  }

  #padRemoved(pad: Pad): void {
    if (!this.#pads.delete(pad) || this.context.signal.aborted) return
    if (this.#created) this.#rebuild()
    else this.#createBuffers()
  }

  async #applyProperties(state: PadState, changes: Partial<PadProperties>): Promise<void> {
    const previous = state.properties.mimeType
    if (changes.track !== undefined && state.properties.track !== undefined && changes.track !== state.properties.track)
      state.replacedBefore = 'next group'
    state.properties = { ...state.properties, ...changes }
    if (changes.initData) state.initData = changes.initData
    if (changes.live !== undefined) this.#live = changes.live
    if (changes.live !== undefined || changes.duration !== undefined || changes.seekable !== undefined) {
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
   * under the cap, which backs up the connection queue. Live has no cap: a viewer stays
   * near the edge, and discarding only made the source refetch what it dropped. Only after
   * a quota error does live drop packets, until half of what was ahead has played, then it
   * seeks upstream once to where the data stops.
   */
  async #makeRoom(pad: Pad, state: PadState, epoch: number): Promise<boolean> {
    if (this.#live) {
      if (state.quotaAhead === undefined) return true
      const end = (state.queue && bufferedEnd(state.queue.buffer)) ?? this.#video.currentTime
      if (end - this.#video.currentTime >= state.quotaAhead / 2) return false
      this.#seekUpstream(pad, state, end)
      return false
    }
    // Not `seeks`: one answered before this wakes would leave it waiting on a full buffer for good
    while (!this.#hasRoom(state) && state.epoch === epoch) {
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
    this.#seekTracker.jump(this.#resumeAt ?? Math.max(...buffers.map((b) => b.buffered.start(0))))
    this.#resumeAt = undefined
    this.emit('ready')
  }

  #onEvent(pad: Pad, event: PadEvent): boolean {
    const state = this.#pads.get(pad)
    if (!state) return true
    if (event.type === 'eos') {
      state.ended = true
      void this.#endOfStream()
    } else if (event.reason === 'gap') {
      this.#checkGap()
    } else if (state.seeks > 0) {
      this.#seekAnswered(state, event.failed === true)
    }
    return true
  }

  #onSignal(pad: Pad, signal: PadSignal): boolean {
    const state = this.#pads.get(pad)
    if (!state) return true
    if (signal.type === 'flush') {
      // Everything the pad buffered goes once the source answers: what comes next replaces it all
      this.#seekUpstream(pad, state, this.#video.currentTime, 0)
    }
    return true
  }

  /** A seek the sink didn't make: upstream unless the target is buffered and no upstream seek is pending. */
  #onSeeking(): void {
    if (!this.#seekTracker.seeking()) return
    const target = this.#video.currentTime
    const pending = [...this.#pads.values()].some((s) => s.seeks > 0)
    if (!pending && rangeAt(target, this.#video.buffered) !== -1) return
    for (const [pad, state] of this.#pads) this.#seekUpstream(pad, state, target)
  }

  /**
   * The source answers every `seek` with one `discontinuity`, so counting them needs no seek
   * identifier. Once answered, what's buffered from `removeFrom` on is removed.
   */
  #seekUpstream(pad: Pad, state: PadState, mediaTime: number, removeFrom = mediaTime): void {
    state.seeks++
    state.epoch++
    state.replaceFrom = removeFrom
    state.ended = false
    state.quotaAhead = undefined
    pad.signal({ type: 'seek', mediaTime })
    this.#notify()
  }

  #seekAnswered(state: PadState, failed: boolean): void {
    if (failed) {
      state.seeks = 0
      this.#seekTracker.fail(new PlayerError('SEEK_FAILED', 'A source could not seek', { element: 'mse' }))
      return
    }
    if (--state.seeks > 0) return
    const from = state.replaceFrom
    state.keyframes = state.keyframes.filter((k) => k < from)
    state.group = undefined
    // The buffer from here on is replaced, which also takes care of a switch's old data
    state.replacedBefore = undefined
    // A failure here means the buffer is gone, which the next append reports. Packets waiting
    // for room look again once it's done: a plain MediaSource fires nothing on a removal.
    state.queue
      ?.run((b) => removeBuffered(b, from, Infinity))
      .catch(() => {})
      .then(this.#notify)
  }

  #checkGap(): void {
    const video = this.#video
    if (!this.#started) return
    this.#dropReplaced()
    if (video.seeking) return this.#unstrand()
    const target = gapJumpTarget(video.currentTime, video.buffered, this.#live, this.#options.liveEdgeDelay)
    if (target !== undefined && target > video.currentTime) this.#seekTracker.jump(target)
  }

  /**
   * A seek that landed just before the data never completes: a group located a little late, or
   * tracks whose data starts at different times. Once no upstream seek is pending and every
   * buffer has data past it, nothing will fill the spot, so it jumps to where the data starts.
   */
  #unstrand(): void {
    const time = this.#video.currentTime
    const buffered = this.#video.buffered
    if (rangeAt(time, buffered) !== -1) return
    for (const state of this.#pads.values()) {
      if (state.seeks > 0 || !state.queue || (bufferedEnd(state.queue.buffer) ?? 0) <= time) return
    }
    for (let i = 0; i < buffered.length; i++)
      if (buffered.start(i) > time) return this.#seekTracker.jump(buffered.start(i))
  }

  /**
   * After a switch, the old track's data plays out up to the new track's first group. Once
   * playback gets there, everything before it is removed, so a seek back never shows the old
   * track. It stops short of that group by `SPLICE_TOLERANCE`, because `remove()` also takes
   * the frames up to the next keyframe, and the new group may start a little earlier.
   */
  #dropReplaced(): void {
    const time = this.#video.currentTime
    for (const state of this.#pads.values()) {
      const before = state.replacedBefore
      if (typeof before !== 'number' || time < before || !state.queue) continue
      state.replacedBefore = undefined
      state.keyframes = state.keyframes.filter((k) => k >= before)
      state.queue.run((b) => removeBuffered(b, 0, before - SPLICE_TOLERANCE)).catch(() => {})
    }
  }

  /** A live sink that fell behind the seekable range jumps near the end of `buffered`. A paused one stays paused. */
  #onVisible(): void {
    const video = this.#video
    if (document.visibilityState !== 'visible' || !this.#live || !this.#started || !video.buffered.length) return
    const seekable = video.seekable.length ? video.seekable : video.buffered
    if (video.currentTime < seekable.start(0))
      this.#seekTracker.jump(liveEdge(video.buffered, this.#options.liveEdgeDelay))
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

  /** Sets the duration once no source buffer is updating: `Infinity` live, so `seekable` is the live range. */
  async #applyDuration(): Promise<void> {
    const queues = [...this.#pads.values()].flatMap((s) => s.queue ?? [])
    const duration = this.#live ? Infinity : vodDuration(this.#properties())
    const source = this.#source
    if (duration === undefined || queues.length === 0 || source.duration === duration) return
    // Set it at once when every buffer is idle: by the time each queue drains, another append may have started
    if (queues.some((q) => q.buffer.updating)) await Promise.all(queues.map((q) => q.run(() => {})))
    try {
      if (source.readyState === 'open') source.duration = duration
    } catch (error) {
      logger.warn('MSESink', 'Could not set the duration', error) // an append started meanwhile, or data past it
    }
    this.#updateSeekable()
  }

  /**
   * Live only. Native `seekable` is the union of the live range and `buffered` (MSE 10.1), so its
   * end stays at the buffered end; `liveEdgeStart` is taken from this range. The live range is set
   * only from a media timeline: without one, `seekable` starts at 0, and seeks before the buffer
   * reach the relay's cache.
   */
  #updateSeekable(): void {
    const source = this.#source
    if (!this.#live || source.readyState !== 'open') return
    const properties = this.#properties()
    const range = liveSeekable(this.#video.buffered, properties)
    if (!range || (range.start === this.#seekable?.start && range.end === this.#seekable.end)) return
    this.#seekable = range
    if (properties.some((p) => p.seekable)) source.setLiveSeekableRange(range.start, range.end)
  }

  #properties(): PadProperties[] {
    return [...this.#pads.values()].map((s) => s.properties)
  }

  /**
   * An append or decode failure. The first rebuilds the media source, a second is fatal.
   * Failures of a media source that was already replaced are ignored.
   */
  #mediaFailed(source: MediaSource, cause: unknown): void {
    if (source !== this.#source || this.#fatal || this.context.signal.aborted) return
    if (this.#rebuiltAfterFailure) {
      // shortcut: a fatal media error event only surfaces on the next receive(), fine while sources keep writing
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
    this.#seekTracker.reset()
    this.#created = false
    this.#started = false
    this.#resumeAt = resumeAt
    this.#buffers.resolve() // receives still waiting on the old buffers see no queue and drop their packet
    this.#buffers = handledResolvers()
    for (const [pad, state] of this.#pads) {
      state.queue = undefined
      state.initData = state.properties.initData
      state.keyframes = []
      state.group = undefined
      state.replacedBefore = undefined
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
