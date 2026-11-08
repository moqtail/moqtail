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

export const CATCHUP_RATE = 1.1
/** The target latency when the catalog sets none, in seconds. */
export const TARGET_LATENCY = 1
/** How far the forward buffer may stray from its goal before the rate changes. */
const TOLERANCE = 0.1
/** Below this forward buffer playback is about to stall; no goal is lower. */
const STALL_THRESHOLD = 0.5
/** The slowest rate, when about to stall. */
const MIN_RATE = 0.6
/** A forward buffer this many times its goal is jumped instead of caught up. */
const JUMP_FACTOR = 4

/**
 * The forward buffer that gives the target latency is the goal, never below the stall
 * threshold or `min` nor above `max`. Without a target the latency stays where it is.
 */
export function decide(state: BufferState, catchupRate: number): BufferDecision {
  const { forward, latency, target = latency, min = 0, max = Infinity } = state
  const exact = forward + target - latency
  const goal = Math.min(Math.max(exact, min, STALL_THRESHOLD), max)
  // Far behind the target (a seek back): land at it past the buffer at once, not chase the buffer end
  if (forward > goal * JUMP_FACTOR) return { jumpTo: -exact > goal * JUMP_FACTOR ? exact : goal }
  if (forward > goal + TOLERANCE) return { rate: catchupRate }
  if (forward >= goal - TOLERANCE) return { rate: 1 }
  const slow = 2 - catchupRate
  return { rate: forward < STALL_THRESHOLD ? Math.max(MIN_RATE, (slow * forward) / STALL_THRESHOLD) : slow }
}

interface Attached {
  player: MOQtailPlayer
  sink: MediaSinkElement
  abort: AbortController
}

/**
 * Catches up at `catchupRate` when behind the target latency, slows to `2 - catchupRate`
 * when ahead. Without catalog targets the target latency is `targetLatency`.
 */
export class LiveBufferController implements BufferController {
  readonly #catchupRate: number
  /** Seconds. */
  readonly #targetLatency: number
  #attached: Attached | undefined
  /** Of the first playing track that has any; seconds. */
  #targets: Omit<BufferState, 'forward' | 'latency'> | undefined
  #timeline: TimelineReader | undefined
  /** Set once the controller changed the rate, so `detach()` gives 1 back. */
  #adjusted = false

  constructor(catchupRate: number, targetLatency: number) {
    this.#catchupRate = catchupRate
    this.#targetLatency = targetLatency
  }

  attach(player: MOQtailPlayer, sink: MediaSinkElement): void {
    const abort = new AbortController()
    this.#attached = { player, sink, abort }
    player.on('catalog', this.#refresh)
    player.on('renditionchange', this.#refresh)
    sink.addEventListener('timeupdate', () => this.#tick(sink), { signal: abort.signal })
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

  #tick(sink: MediaSinkElement): void {
    if (sink.paused || sink.seeking) return
    const { currentTime, buffered } = sink
    const range = rangeAt(currentTime, buffered)
    if (range === -1) return // the sink jumps gaps itself
    const forward = buffered.end(range) - currentTime
    const targets = this.#targets ?? { target: this.#targetLatency, min: undefined, max: undefined }
    const decision = decide({ forward, latency: this.#latency(currentTime, forward), ...targets }, this.#catchupRate)
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
    const decision = decide(state(s), CATCHUP_RATE)
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
}
