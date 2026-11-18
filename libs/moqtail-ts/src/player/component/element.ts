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

/**
 * `<moqtail-video>`: a `<video>` in a shadow root, played by a `MOQtailPlayer` built from
 * `src` and `config`. HTMLMediaElement forwarding comes from custom-media-element, tracks and
 * renditions from media-tracks. Without `HTMLElement`, as in a server, the class still loads.
 */

import { CustomVideoElement } from 'custom-media-element'
import { MediaTracksMixin } from 'media-tracks'
import {
  abr,
  liveBuffer,
  MOQtailPlayer,
  PlayerError,
  type MOQtailPlayerOptions,
  type PlayerErrorCategory,
  type StreamType,
} from '@/player'
import { Renditions } from './renditions'

const MEDIA_ERROR_CODES: Record<PlayerErrorCategory, number> = { aborted: 1, network: 2, media: 3, source: 4 }

/** @public A `<video>`-backed element implementing HTMLMediaElement and the media-ui-extensions IDL. */
export class MOQtailVideoElement extends MediaTracksMixin(CustomVideoElement) {
  static override getTemplateHTML = (attrs: Record<string, string>): string => {
    // The sink attaches the media source; a moqt:// src on the inner video would only fail
    const { src: _, ...rest } = attrs
    return CustomVideoElement.getTemplateHTML(rest)
  }

  #config: MOQtailPlayerOptions | undefined
  #player: MOQtailPlayer | undefined
  #renditions: Renditions | undefined
  /** The previous player's teardown; the next one waits for it, since both drive the same video. */
  #destroyed: Promise<void> = Promise.resolve()
  #streamType: StreamType = 'unknown'
  #error: MediaError | null = null

  constructor() {
    super()
    // Set on the instance before the element was defined: move it to the accessor
    if (Object.hasOwn(this, 'config')) {
      const value = this.config
      Reflect.deleteProperty(this, 'config')
      this.config = value
    }
  }

  /**
   * Player options, read on each load. Without plugins, `liveBuffer()` and `abr()` are the ones; a
   * list is taken as written, so one without them plays without a buffer or ABR controller.
   * Changing it doesn't restart playback.
   */
  get config(): MOQtailPlayerOptions | undefined {
    return this.#config
  }

  set config(value: MOQtailPlayerOptions | undefined) {
    this.#config = value
  }

  get player(): MOQtailPlayer | undefined {
    return this.#player
  }

  get streamType(): StreamType {
    return this.#streamType
  }

  /** Proposal 0007: where the live edge starts, in seconds; `NaN` on demand. */
  get liveEdgeStart(): number {
    return (this.#player?.mediaSink?.liveEdgeStart ?? NaN) - this.#offset()
  }

  /**
   * Proposal 0007: live, `Infinity` with an accessible window (DVR) and `0` without; `NaN` otherwise.
   * Players such as Video.js treat a media as live only when it has this and `liveEdgeStart`.
   */
  get targetLiveWindow(): number {
    if (this.#streamType !== 'live') return NaN
    return this.#dvrRange ? Infinity : 0
  }

  /**
   * Live with an accessible window, the element's times count from the window's start: seekbars
   * such as Video.js's start at 0. The inner video and the player keep media time.
   */
  override get currentTime(): number {
    return this.nativeEl.currentTime - this.#offset()
  }

  override set currentTime(value: number) {
    // Live without an accessible window has no DVR (MSF 5.2.46)
    if (this.#streamType === 'live' && !this.#dvrRange) return
    this.nativeEl.currentTime = value + this.#offset()
  }

  /** Live, the accessible window, or nothing without one. */
  override get seekable(): TimeRanges {
    if (this.#streamType !== 'live') return this.nativeEl.seekable
    const range = this.#dvrRange
    return timeRanges(range ? [[0, range.end - range.start]] : [])
  }

  override get buffered(): TimeRanges {
    const offset = this.#offset()
    const native = this.nativeEl.buffered
    if (offset === 0) return native
    const pairs: [number, number][] = []
    for (let i = 0; i < native.length; i++) {
      const end = native.end(i) - offset
      if (end > 0) pairs.push([Math.max(0, native.start(i) - offset), end])
    }
    return timeRanges(pairs)
  }

  override get error(): MediaError | null {
    return this.#error
  }

  /** The inner video's `error` is left out: the sink recovers from those, and fatal ones arrive through the player. */
  override handleEvent(event: Event): void {
    if (event.type !== 'error') super.handleEvent(event)
  }

  override attributeChangedCallback(name: string, oldValue?: string | null, newValue?: string | null): void {
    if (name !== 'src') super.attributeChangedCallback(name, oldValue, newValue)
    else if (oldValue !== newValue) this.load()
  }

  /** Destroys the player, then builds a new one from `src` and `config`, as `preload` and `autoplay` ask. */
  override load(): void {
    this.#unload()
    const src = this.getAttribute('src')
    if (!src) return
    let player: MOQtailPlayer
    try {
      player = new MOQtailPlayer(this.#playerOptions())
        .source(src, { switchFrom: this.hasAttribute('switch-from') })
        .sink(this.nativeEl)
    } catch (error) {
      queueMicrotask(() => this.#fail(asPlayerError(error)))
      return
    }
    this.#player = player
    this.#renditions = new Renditions(this, player)
    player.on('error', (error) => this.#fail(error))
    player.on('streamtypechange', (type) => this.#setStreamType(type))
    if (this.autoplay) {
      // A failure is the `error` event, or a blocked autoplay
      void this.play().catch(() => {})
    } else if (this.getAttribute('preload') !== 'none') {
      void this.#init(player).catch(() => {})
    }
  }

  override async play(): Promise<void> {
    const player = this.#player
    if (!player) return super.play()
    await this.#init(player)
    await player.play()
  }

  /** `config`, with `liveBuffer()` and `abr()` as the plugins when it lists none. */
  #playerOptions(): MOQtailPlayerOptions {
    const config = this.#config ?? {}
    if (config.plugins && config.plugins.length > 0) return config
    return { ...config, plugins: [liveBuffer(), abr()] }
  }

  async #init(player: MOQtailPlayer): Promise<void> {
    await this.#destroyed
    await player.init()
    this.#renditions?.sync()
  }

  #unload(): void {
    const player = this.#player
    this.#renditions?.dispose()
    this.#renditions = undefined
    this.#player = undefined
    this.#error = null
    this.#setStreamType('unknown')
    if (player) this.#destroyed = player.destroy().catch(() => {}) // destroy() is callable in any state
  }

  get #dvrRange(): { start: number; end: number } | undefined {
    return this.#streamType === 'live' ? this.#player?.mediaSink?.dvrRange : undefined
  }

  #offset(): number {
    return this.#dvrRange?.start ?? 0
  }

  #setStreamType(type: StreamType): void {
    if (type === this.#streamType) return
    this.#streamType = type
    this.dispatchEvent(new Event('streamtypechange'))
  }

  #fail(error: PlayerError): void {
    this.#error = mediaError(error)
    this.dispatchEvent(new Event('error'))
  }
}

function asPlayerError(error: unknown): PlayerError {
  if (error instanceof PlayerError) return error
  return new PlayerError('UNSUPPORTED_SOURCE', 'The player could not be created', {
    element: 'component',
    cause: error,
  })
}

/** `TimeRanges` can't be constructed either. */
function timeRanges(pairs: readonly [number, number][]): TimeRanges {
  const at = (i: number, edge: 0 | 1) => {
    const pair = pairs[i]
    if (!pair) throw new DOMException(`No range ${i}`, 'IndexSizeError')
    return pair[edge]
  }
  return { length: pairs.length, start: (i) => at(i, 0), end: (i) => at(i, 1) }
}

/** `MediaError` can't be constructed, so this has its shape. */
function mediaError(error: PlayerError): MediaError {
  return {
    code: MEDIA_ERROR_CODES[error.category],
    message: error.message,
    MEDIA_ERR_ABORTED: 1,
    MEDIA_ERR_NETWORK: 2,
    MEDIA_ERR_DECODE: 3,
    MEDIA_ERR_SRC_NOT_SUPPORTED: 4,
  }
}
