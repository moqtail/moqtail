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

import { PlayerError } from '@/player/error'

// HTMLMediaElement.HAVE_NOTHING, which node doesn't define
const HAVE_NOTHING = 0
/** Own jumps remembered at most: one that never fired `seeking` must not stay forever. */
const MAX_OWN_JUMPS = 4

/**
 * Tells the media element's `seeking` events apart: the sink's own jumps (start position,
 * gap jumping) are ignored, the first one after `seek()` belongs to its promise, and any
 * other is a native seek. The element's `seeking` listener is the only seek entry point,
 * so `seek()` only sets `currentTime`.
 *
 * Own jumps are recognised by where they land, not counted: a jump that fires no `seeking`
 * would otherwise make the next real seek look like one of them, and it would never be
 * forwarded upstream nor resolve.
 */
export class SeekTracker {
  #pending: { claimed: boolean; resolve: () => void; reject: (error: unknown) => void } | undefined
  /** Where the sink's own jumps land, oldest first, until their `seeking`. */
  #ownJumps: number[] = []

  constructor(readonly video: HTMLVideoElement) {}

  /** Sets `currentTime`. Resolves on `seeked`; a newer seek, native or not, rejects it with `ABORTED`. */
  seek(mediaTime: number): Promise<void> {
    this.#abort()
    const { promise, resolve, reject } = Promise.withResolvers<void>()
    this.#pending = { claimed: false, resolve, reject }
    this.video.currentTime = mediaTime
    return promise
  }

  /** A jump the sink makes itself. */
  jump(mediaTime: number): void {
    // Without metadata it only sets the default start position, which the element seeks to
    // once metadata loads if it's past 0: at 0 no `seeking` follows (HTML › media element › currentTime).
    if (this.video.readyState !== HAVE_NOTHING || mediaTime > 0) {
      this.#ownJumps.push(mediaTime)
      if (this.#ownJumps.length > MAX_OWN_JUMPS) this.#ownJumps.shift()
    }
    this.video.currentTime = mediaTime
  }

  /** Call on `seeking`. True when it's a seek to handle: `seek()` or native, not the sink's own jump. */
  seeking(): boolean {
    const own = this.#ownJumps.findIndex((target) => Math.abs(target - this.video.currentTime) < 0.001)
    if (own !== -1) {
      this.#ownJumps.splice(own, 1)
      return false
    }
    if (this.#pending && !this.#pending.claimed) this.#pending.claimed = true
    else this.#abort() // a native seek supersedes a pending one
    return true
  }

  /** Call on `seeked`. */
  seeked(): void {
    if (!this.#pending?.claimed) return
    this.#pending.resolve()
    this.#pending = undefined
  }

  /** Rejects the pending `seek()`, if any. */
  fail(error: unknown): void {
    this.#pending?.reject(error)
    this.#pending = undefined
  }

  /** For a reloaded element: its queued `seeking` events are gone, so pending jumps and seeks are too. */
  reset(): void {
    this.#ownJumps = []
    this.#abort()
  }

  #abort(): void {
    this.fail(new PlayerError('ABORTED', 'A newer seek superseded this one', { element: 'mse' }))
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest
  const settled = (promise: Promise<void>) => {
    const state = { value: 'pending' }
    promise.then(
      () => (state.value = 'resolved'),
      (e: PlayerError) => (state.value = e.code),
    )
    return state
  }
  const flush = () => new Promise((resolve) => setTimeout(resolve))

  describe('SeekTracker', () => {
    test('a jump to 0 before metadata fires no seeking; one past 0 does once metadata loads', async () => {
      const tracker = new SeekTracker({ currentTime: 0, readyState: HAVE_NOTHING } as HTMLVideoElement)
      tracker.jump(4)
      expect(tracker.seeking()).toBe(false) // the start position's seek, at metadata
      tracker.jump(0)
      const seek = settled(tracker.seek(5))
      expect(tracker.seeking()).toBe(true)
      tracker.seeked()
      await flush()
      expect(seek.value).toBe('resolved')
    })

    test('a jump that fired no seeking does not swallow a seek elsewhere', async () => {
      const tracker = new SeekTracker({ currentTime: 0, readyState: 1 } as HTMLVideoElement)
      tracker.jump(3) // its seeking never comes
      const seek = settled(tracker.seek(9))
      expect(tracker.seeking()).toBe(true)
      tracker.seeked()
      await flush()
      expect(seek.value).toBe('resolved')
    })

    test('own jumps are ignored, a seek resolves on seeked, newer seeks reject older ones', async () => {
      const tracker = new SeekTracker({ currentTime: 0, readyState: 1 } as HTMLVideoElement)
      tracker.jump(3)
      expect(tracker.seeking()).toBe(false)

      const first = settled(tracker.seek(5))
      const second = settled(tracker.seek(6))
      expect(tracker.seeking()).toBe(true)
      expect(tracker.seeking()).toBe(true) // native, supersedes the second
      tracker.seeked()
      const third = settled(tracker.seek(7))
      expect(tracker.seeking()).toBe(true)
      tracker.seeked()
      await flush()
      expect([first.value, second.value, third.value]).toEqual(['ABORTED', 'ABORTED', 'resolved'])
      expect(tracker.video.currentTime).toBe(7)
    })
  })
}
