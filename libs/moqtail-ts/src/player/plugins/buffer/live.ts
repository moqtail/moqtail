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
 * The `liveBuffer` controller, ported from demo-abr's `maintainLiveEdgeDelay`: playback rate
 * moves the forward buffer toward the one that gives the catalog's target latency.
 */

import type { CatalogTrack, MediaSinkElement, TimelineReader } from '@/player/element/interfaces'
import { CatalogElement } from '@/player/elements/catalog/element'
import { liveEdge, rangeAt } from '@/player/elements/mse/gaps'
import type { MOQtailPlayer } from '@/player/player'
import type { BufferController } from '@/player/plugin'
import { renditionId } from '@/player/util/track'

/** What a rate decision looks at, in seconds. */
export interface BufferState {
  /** Buffered ahead of `currentTime`. */
  forward: number
  /** From wallclock data, else the forward buffer. */
  latency: number
  /** `targetLatency ?? buffers.target` (MSF 5.2.9). */
  target: number | undefined
  /** `buffers.min` and `buffers.max` (MSF 5.2.10). */
  min: number | undefined
  max: number | undefined
}

/** A playback rate, or a jump to `jumpTo` seconds behind the end of the buffer; negative is past it. */
export type BufferDecision = { rate: number } | { jumpTo: number }

/** @public */
export interface LiveBufferConfig {
  /** The playback rate when behind the target latency; `2 - catchupRate` when ahead. Between 1 and 2, default 1.1. */
  catchupRate?: number
  /**
   * Seconds of latency to hold when the catalog sets no `targetLatency` or `buffers`. Measured like the
   * catalog's: wallclock latency when the timeline has it, else the forward buffer. Above 0, default 1.
   */
  targetLatency?: number
  /** Seconds the forward buffer may stray from its goal before the rate changes. At least 0, default 0.1. */
  tolerance?: number
  /**
   * Seconds of forward buffer below which playback is about to stall; no goal is lower. At least 0, default 0.5.
   * On a canvas, lower the playout sink's `preroll` with it, else each stall still refills 0.5 s.
   */
  stallThreshold?: number
  /** The slowest rate, when about to stall. Above 0 and at most 1, default 0.6. */
  minRate?: number
  /** A forward buffer this many times its goal is jumped instead of caught up. Above 1, default 4. */
  jumpFactor?: number
  /**
   * On a sink without a precise rate: seconds the forward buffer may stray from its goal before
   * a jump, which is a visible skip. Above 0, default 0.5.
   */
  jumpTolerance?: number
}

export const LIVE_BUFFER_DEFAULTS: Required<LiveBufferConfig> = {
  catchupRate: 1.1,
  targetLatency: 1,
  tolerance: 0.1,
  stallThreshold: 0.5,
  minRate: 0.6,
  jumpFactor: 4,
  jumpTolerance: 0.5,
}

/**
 * The forward buffer that gives the target latency is the goal, never below the stall
 * threshold or `min` nor above `max`. Without a target the latency stays where it is. With
 * `'jump'` correction the rate stays 1 and a forward buffer off its goal jumps to it.
 */
export function decide(
  state: BufferState,
  config: Required<LiveBufferConfig>,
  correction: 'rate' | 'jump' = 'rate',
): BufferDecision {
  const { forward, latency, target = latency, min = 0, max = Infinity } = state
  const { catchupRate, tolerance, stallThreshold, minRate, jumpFactor, jumpTolerance } = config
  const exact = forward + target - latency
  const goal = Math.min(Math.max(exact, min, stallThreshold), max)

  // Far behind the target (a seek back): land at it past the buffer at once, not chase the buffer end
  if (forward > goal * jumpFactor) return { jumpTo: -exact > goal * jumpFactor ? exact : goal }
  if (correction === 'jump') return Math.abs(forward - goal) > jumpTolerance ? { jumpTo: goal } : { rate: 1 }
  if (forward > goal + tolerance) return { rate: catchupRate }
  if (forward >= goal - tolerance) return { rate: 1 }

  const slow = 2 - catchupRate
  return { rate: forward < stallThreshold ? Math.max(minRate, (slow * forward) / stallThreshold) : slow }
}

interface Attached {
  player: MOQtailPlayer
  sink: MediaSinkElement
  abort: AbortController
}

/**
 * Catches up at `config.catchupRate` when behind the target latency, slows to `2 - catchupRate`
 * when ahead; on a sink without a precise rate it jumps instead. Without catalog targets the target latency is `config.targetLatency`.
 */
export class LiveBufferController implements BufferController {
  readonly #config: Required<LiveBufferConfig>
  #attached: Attached | undefined
  /** Of the first playing track that has any; seconds. */
  #targets: Omit<BufferState, 'forward' | 'latency'> | undefined
  #timeline: TimelineReader | undefined
  /** Set once the controller changed the rate, so `detach()` gives 1 back. */
  #adjusted = false
  /** Cleared by a seek behind the live edge, set again by one to it (proposal 0007). */
  #following = true

  constructor(config: Required<LiveBufferConfig>) {
    this.#config = config
  }

  attach(player: MOQtailPlayer, sink: MediaSinkElement): void {
    const abort = new AbortController()
    this.#attached = { player, sink, abort }

    player.on('catalog', this.#refresh)
    player.on('renditionchange', this.#refresh)
    sink.addEventListener('timeupdate', () => this.#tick(sink), { signal: abort.signal })
    sink.addEventListener('seeking', () => this.#seeking(sink), { signal: abort.signal })

    this.#refresh()
  }

  detach(): void {
    const attached = this.#attached
    if (!attached) return

    this.#attached = undefined
    attached.abort.abort()
    attached.player.off('catalog', this.#refresh)
    attached.player.off('renditionchange', this.#refresh)

    if (this.#adjusted) attached.sink.setPlaybackRate(1)
    this.#adjusted = false
    this.#following = true
    this.#targets = undefined
    this.#timeline = undefined
  }

  /** Reads the targets and the timeline of the playing tracks. */
  readonly #refresh = (): void => {
    const attached = this.#attached
    const catalog = attached?.player.sources.find((s): s is CatalogElement => s instanceof CatalogElement)
    if (!attached || !catalog) return

    const active = new Set(
      attached.player.mediaSources.flatMap((s) => s.getRenditions().flatMap((r) => (r.active ? [r.id] : []))),
    )
    const playing = catalog.tracks.filter((t) => active.has(renditionId(t, catalog)))
    const hasTargets = (t: CatalogTrack) => t.targetLatency !== undefined || t.buffers !== undefined

    // Without catalog targets the timeline still gives the fallback target its wallclock latency
    const track = playing.find(hasTargets) ?? playing[0]
    this.#targets = track && hasTargets(track) ? targetsOf(track) : undefined
    if (!track) return

    void catalog.timeline(track).then((timeline) => {
      if (this.#attached === attached) this.#timeline = timeline
    })
  }

  /**
   * Where a seek lands decides, whoever made it: the controller's and the sink's own jumps land at
   * the edge, a viewer's seek back leaves the rate at 1 until a seek to the edge (the Live button).
   */
  #seeking(sink: MediaSinkElement): void {
    // An unknown edge counts as at it
    this.#following = !(sink.currentTime < sink.liveEdgeStart)
    if (this.#following || !this.#adjusted) return

    sink.setPlaybackRate(1)
    this.#adjusted = false
  }

  #tick(sink: MediaSinkElement): void {
    if (sink.paused || sink.seeking || !this.#following) return

    const { currentTime, buffered } = sink
    const range = rangeAt(currentTime, buffered)
    if (range === -1) return // the sink jumps gaps itself

    const forward = buffered.end(range) - currentTime
    const targets = this.#targets ?? { target: this.#config.targetLatency, min: undefined, max: undefined }
    const state = { forward, latency: this.#latency(currentTime, forward), ...targets }

    // A sink whose clock stalls on each rate change would undo a catch-up and start the next one
    const decision = decide(state, this.#config, sink.preciseRate ? 'rate' : 'jump')

    if ('jumpTo' in decision) {
      // A failed jump leaves playback where it was
      sink.seek(liveEdge(buffered, decision.jumpTo)).catch(() => {})
    } else if (decision.rate !== sink.playbackRate) {
      sink.setPlaybackRate(decision.rate)
      this.#adjusted = true
    }
  }

  /** Wallclock latency (MSF 11.6) when the timeline has it and the clocks agree, else the forward buffer. */
  #latency(currentTime: number, forward: number): number {
    const wallclock = this.#timeline?.timeToWallclock(currentTime * 1000)
    const latency = wallclock === undefined ? -1 : (Date.now() - wallclock) / 1000
    return latency >= 0 ? latency : forward
  }
}

function targetsOf(track: CatalogTrack): Omit<BufferState, 'forward' | 'latency'> {
  const seconds = (ms: number | undefined) => (ms === undefined ? undefined : ms / 1000)
  return {
    target: seconds(track.targetLatency ?? track.buffers?.target),
    min: seconds(track.buffers?.min),
    max: seconds(track.buffers?.max),
  }
}

if (import.meta.vitest) {
  const { test, expect } = import.meta.vitest
  const state = (s: Partial<BufferState>): BufferState => ({
    forward: 2,
    latency: 2,
    target: 2,
    min: undefined,
    max: undefined,
    ...s,
  })

  const rate = (s: Partial<BufferState>) => {
    const decision = decide(state(s), LIVE_BUFFER_DEFAULTS)
    return 'rate' in decision ? { rate: Math.round(decision.rate * 100) / 100 } : decision
  }

  test('the forward buffer as latency: 1 within tolerance, faster behind, slower ahead, slowest near a stall', () => {
    expect(rate({})).toEqual({ rate: 1 })
    expect(rate({ forward: 2.5, latency: 2.5 })).toEqual({ rate: 1.1 })
    expect(rate({ forward: 1.5, latency: 1.5 })).toEqual({ rate: 0.9 })
    expect(rate({ forward: 0.25, latency: 0.25 })).toEqual({ rate: 0.6 })
    expect(rate({ forward: 9, latency: 9 })).toEqual({ jumpTo: 2 })
  })

  test('far behind the target jumps straight to it past the buffer, near it stays inside', () => {
    expect(rate({ forward: 3, latency: 30, target: 1 })).toEqual({ jumpTo: -26 })
    expect(rate({ forward: 3, latency: 4.5, target: 1 })).toEqual({ jumpTo: 0.5 }) // 0.5 past the end: not worth it
  })

  test('wallclock latency: the goal is the forward buffer that gives the target', () => {
    expect(rate({ forward: 1.5, latency: 3 })).toEqual({ rate: 1.1 }) // goal 0.5
    expect(rate({ forward: 1, latency: 1.5 })).toEqual({ rate: 0.9 }) // goal 1.5
    expect(rate({ forward: 0.5, latency: 4 })).toEqual({ rate: 1 }) // out of reach: hold at the stall threshold
  })

  test('buffers.min and max bound the goal, and without a target only they apply', () => {
    expect(rate({ forward: 1, latency: 1, target: 0.6, min: 1 })).toEqual({ rate: 1 })
    expect(rate({ forward: 0.8, latency: 0.8, target: 0.6, min: 1 })).toEqual({ rate: 0.9 })
    expect(rate({ forward: 3, latency: 3, target: 4, max: 2.5 })).toEqual({ rate: 1.1 })
    expect(rate({ forward: 3, latency: 5, target: undefined })).toEqual({ rate: 1 })
    expect(rate({ forward: 3, latency: 3, target: undefined, max: 2 })).toEqual({ rate: 1.1 })
  })

  test('the config replaces each default', () => {
    const tuned = (s: Partial<BufferState>, config: LiveBufferConfig) =>
      decide(state(s), { ...LIVE_BUFFER_DEFAULTS, ...config })
    expect(tuned({ forward: 2.2, latency: 2.2 }, { tolerance: 0.3 })).toEqual({ rate: 1 })
    expect(tuned({ forward: 0.1, latency: 0.1 }, { stallThreshold: 0.2, minRate: 0.8 })).toEqual({ rate: 0.8 })
    expect(tuned({ forward: 5, latency: 5 }, { jumpFactor: 2 })).toEqual({ jumpTo: 2 })
  })

  test("'jump' correction keeps rate 1 and jumps to the goal once the forward buffer is 0.5 s off it", () => {
    const jump = (s: Partial<BufferState>) => decide(state(s), LIVE_BUFFER_DEFAULTS, 'jump')
    expect(jump({ forward: 2.25, latency: 2.25 })).toEqual({ rate: 1 })
    expect(jump({ forward: 0.25, latency: 0.25, target: 0.25 })).toEqual({ rate: 1 }) // no slowdown near a stall
    expect(jump({ forward: 2.75, latency: 2.75 })).toEqual({ jumpTo: 2 })
    expect(jump({ forward: 1, latency: 1 })).toEqual({ jumpTo: 2 }) // back into the buffer
    expect(jump({ forward: 9, latency: 9 })).toEqual({ jumpTo: 2 })
  })
}
