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
import { defaultBufferController } from '@/player/controllers/buffer'
import { PlayerError, type PlayerErrorCategory } from '@/player/error'
import { MOQtailPlayer, type MOQtailPlayerOptions, type StreamType } from '@/player/player'
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

  /** Player options, read on each load; `buffer` defaults to `defaultBufferController()`. Changing it doesn't restart playback. */
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
    return this.#player?.mediaSink?.liveEdgeStart ?? NaN
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
      const config = { ...this.#config, buffer: this.#config?.buffer ?? defaultBufferController() }
      player = new MOQtailPlayer(config)
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
    if (this.autoplay)
      void this.play().catch(() => {}) // a failure is the `error` event, or a blocked autoplay
    else if (this.getAttribute('preload') !== 'none') void this.#init(player).catch(() => {})
  }

  override async play(): Promise<void> {
    const player = this.#player
    if (!player) return super.play()
    await this.#init(player)
    await player.play()
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
