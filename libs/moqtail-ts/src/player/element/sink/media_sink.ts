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

import { SinkElement } from '@/player/element/base'
import type { ElementContext } from '@/player/element/context'
import { CapabilityCode } from '@/player/element/define'
import type { MediaSinkElement } from '@/player/element/interfaces'
import {
  closeFrame,
  type Pad,
  type PadEvent,
  type PadItem,
  type PadProperties,
  type PadSignal,
} from '@/player/element/pad'
import type { PlayerError } from '@/player/error'

/** What a media sink keeps per sink pad, at the least. */
export interface SinkPadState {
  properties: PadProperties
  ended: boolean
}

/**
 * What every media sink shares: one sink pad per role, a state per pad, its properties and
 * the sink-wide `live` flag, `eos`, and a fatal error after which it drops what it receives. The
 * base registers its pad listeners first, so a subclass's run after them.
 */
export abstract class MediaSink<S extends SinkPadState> extends SinkElement implements MediaSinkElement {
  readonly #target = new EventTarget()
  readonly addEventListener = this.#target.addEventListener.bind(this.#target)
  readonly removeEventListener = this.#target.removeEventListener.bind(this.#target)
  readonly dispatchEvent = this.#target.dispatchEvent.bind(this.#target)

  readonly #states = new Map<Pad, S>()
  #negotiated = false
  #live = false
  #fatal: PlayerError | undefined

  constructor(context: ElementContext) {
    super(context)
    this.on('padadded', (pad) => this.#padAdded(pad))
    this.on('padremoved', (pad) => this.#states.delete(pad))
    this.on('nomorepads', () => {
      this.#negotiated = true
    })
  }

  abstract get currentTime(): number
  abstract get buffered(): TimeRanges
  abstract get paused(): boolean
  abstract get seeking(): boolean
  abstract get ended(): boolean
  abstract get playbackRate(): number
  abstract get liveEdgeStart(): number
  abstract get dvrRange(): { start: number; end: number } | undefined

  protected get states(): ReadonlyMap<Pad, S> {
    return this.#states
  }

  protected get negotiated(): boolean {
    return this.#negotiated
  }

  /** The latest `live` property of any pad. */
  protected get live(): boolean {
    return this.#live
  }

  protected get fatal(): PlayerError | undefined {
    return this.#fatal
  }

  abstract setPlaybackRate(rate: number): void
  abstract play(): Promise<void>
  abstract pause(): Promise<void>
  abstract seek(mediaTime: number): Promise<void>

  /** One pad per role. */
  negotiate(pad: Pad): boolean {
    return this.matchesCapabilities(pad) && !this.pads.some((p) => roleOf(p) === roleOf(pad))
  }

  async receive(pad: Pad, item: PadItem): Promise<void> {
    if (this.#fatal) {
      closeFrame(item)
      return
    }
    const state = this.#states.get(pad)
    if (!state) {
      // The pad went away while this was in flight; only the receiver closes a frame
      if (item.type === 'frame') item.frame.payload.close()
      return
    }
    if (item.type !== 'properties') return this.receiveMedia(pad, state, item)
    const previous = state.properties
    state.properties = { ...previous, ...item.properties }
    if (item.properties.live !== undefined) this.#live = item.properties.live
    await this.propertiesChanged(pad, state, item.properties, previous)
  }

  protected abstract newPadState(): S
  protected abstract receiveMedia(pad: Pad, state: S, item: Exclude<PadItem, { type: 'properties' }>): Promise<void>

  /** Called once `changes` are merged into `state.properties`, which were `previous`. */
  protected propertiesChanged(
    _pad: Pad,
    _state: S,
    _changes: Partial<PadProperties>,
    _previous: PadProperties,
  ): void | Promise<void> {}

  /** Called after an `eos` marked the pad ended. */
  protected abstract padEvent(pad: Pad, state: S, event: PadEvent): void
  protected abstract padSignal(pad: Pad, state: S, signal: PadSignal): void

  protected allPadsEnded(): boolean {
    return [...this.#states.values()].every((s) => s.ended)
  }

  protected padProperties(): PadProperties[] {
    return [...this.#states.values()].map((s) => s.properties)
  }

  /** Fails the player, and drops what `receive()` gets from then on. The first error stays. */
  protected fail(error: PlayerError): void {
    if (this.#fatal) return
    this.#fatal = error
    this.context.fail(error)
  }

  #padAdded(pad: Pad): void {
    if (pad.direction !== 'sink') return
    this.#states.set(pad, this.newPadState())
    pad.onEvent((event) => {
      const state = this.#states.get(pad)
      if (!state) return true
      if (event.type === 'eos') state.ended = true
      this.padEvent(pad, state, event)
      return true
    })
    pad.onSignal((signal) => {
      const state = this.#states.get(pad)
      if (state) this.padSignal(pad, state, signal)
      return true
    })
  }
}

export function roleOf(pad: Pad): string | undefined {
  return pad.capabilities.find((c) => c.code === CapabilityCode.ROLE)?.value
}

if (import.meta.vitest) {
  const { test, expect } = import.meta.vitest
  const { defineElement } = await import('@/player/element/define')
  const { cmafCapabilities } = await import('@/player/elements/cmaf/track')
  const { flush, testChain, testSource } = await import('@/player/testing/elements')
  const { timeRanges } = await import('@/player/testing/media')

  class TestSink extends MediaSink<SinkPadState> {
    readonly currentTime = 0
    readonly buffered = timeRanges([])
    readonly paused = true
    readonly seeking = false
    readonly ended = false
    readonly playbackRate = 1
    readonly liveEdgeStart = NaN
    readonly dvrRange = undefined
    readonly received: PadItem[] = []

    constructor(context: ElementContext) {
      super(context)
      this.on('nomorepads', () => this.emit('ready'))
    }

    get isLive(): boolean {
      return this.live
    }

    propertiesOf(pad: Pad): PadProperties | undefined {
      return this.states.get(pad)?.properties
    }

    setPlaybackRate(): void {}
    async play(): Promise<void> {}
    async pause(): Promise<void> {}
    async seek(): Promise<void> {}

    allEnded(): boolean {
      return this.allPadsEnded()
    }

    protected newPadState(): SinkPadState {
      return { properties: {}, ended: false }
    }

    protected async receiveMedia(_pad: Pad, _state: SinkPadState, item: PadItem): Promise<void> {
      this.received.push(item)
    }

    protected padEvent(): void {}
    protected padSignal(): void {}
  }

  test('one pad per role, merged properties with the live flag, and every pad ended', async () => {
    let sink: TestSink | undefined
    const definition = defineElement({
      type: 'sink',
      name: 'test-media-sink',
      capabilities: [cmafCapabilities('in', 'video'), cmafCapabilities('in', 'audio')],
      init: async (context) => (sink = new TestSink(context)),
    })
    const pads: Pad[] = []
    const script = (s: import('@/player/testing/elements').TestSource) => {
      for (const role of ['video', 'video', 'audio']) pads.push(s.addPad(cmafCapabilities('out', role)))
      s.emit('nomorepads')
    }
    const { chain } = testChain()
    chain.addSource(testSource({ script }), {})
    chain.addSink(definition, {})
    await chain.init(new AbortController().signal)
    const [video, , audio] = pads
    expect(sink!.pads.map((p) => roleOf(p))).toEqual(['video', 'audio'])

    await video!.write({ type: 'properties', properties: { track: 'v1', live: true } })
    await video!.write({ type: 'properties', properties: { mimeType: 'video/mp4' } })
    await flush()
    expect(sink!.isLive).toBe(true)
    expect(sink!.propertiesOf(sink!.pads[0]!)).toEqual({ track: 'v1', live: true, mimeType: 'video/mp4' })

    const eos = { type: 'event', event: { type: 'eos' } } as const
    await video!.write(eos)
    await flush()
    expect(sink!.allEnded()).toBe(false)
    await audio!.write(eos)
    await flush()
    expect(sink!.allEnded()).toBe(true)
    await chain.destroy()
  })
}
