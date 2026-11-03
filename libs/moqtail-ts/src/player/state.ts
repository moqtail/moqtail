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

/** @public The player's lifecycle state. */
export type PlayerState =
  'idle' | 'connecting' | 'ready' | 'playing' | 'paused' | 'stalled' | 'reconnecting' | 'ended' | 'error' | 'destroyed'

// PLAN.md › Lifecycle And Errors. `reconnecting` also returns to the state it interrupted.
const NEXT: Record<PlayerState, readonly PlayerState[]> = {
  idle: ['connecting', 'destroyed'],
  connecting: ['ready', 'ended', 'error', 'destroyed'],
  ready: ['playing', 'paused', 'stalled', 'reconnecting', 'ended', 'error', 'destroyed'],
  playing: ['paused', 'stalled', 'reconnecting', 'ended', 'error', 'destroyed'],
  paused: ['playing', 'stalled', 'reconnecting', 'ended', 'error', 'destroyed'],
  stalled: ['playing', 'paused', 'reconnecting', 'ended', 'error', 'destroyed'],
  reconnecting: ['error', 'destroyed'],
  ended: ['playing', 'reconnecting', 'error', 'destroyed'],
  error: ['destroyed'],
  destroyed: [],
}

/** Whether `from → to` is in the table. `resume` is where `reconnecting` returns to. */
export function canTransition(from: PlayerState, to: PlayerState, resume?: PlayerState): boolean {
  return NEXT[from].includes(to) || (from === 'reconnecting' && to === resume && to !== from)
}

if (import.meta.vitest) {
  const { test, expect } = import.meta.vitest

  test('canTransition accepts exactly the PLAN.md table', () => {
    const states = Object.keys(NEXT) as PlayerState[]
    const allowed = states.flatMap((from) =>
      states.filter((to) => canTransition(from, to)).map((to) => `${from}>${to}`),
    )
    expect(allowed).toEqual([
      ...['idle>connecting', 'idle>destroyed'],
      ...['connecting>ready', 'connecting>ended', 'connecting>error', 'connecting>destroyed'],
      ...['ready>playing', 'ready>paused', 'ready>stalled', 'ready>reconnecting', 'ready>ended'],
      ...['ready>error', 'ready>destroyed'],
      ...['playing>paused', 'playing>stalled', 'playing>reconnecting', 'playing>ended', 'playing>error'],
      'playing>destroyed',
      ...['paused>playing', 'paused>stalled', 'paused>reconnecting', 'paused>ended', 'paused>error'],
      'paused>destroyed',
      ...['stalled>playing', 'stalled>paused', 'stalled>reconnecting', 'stalled>ended', 'stalled>error'],
      'stalled>destroyed',
      ...['reconnecting>error', 'reconnecting>destroyed'],
      ...['ended>playing', 'ended>reconnecting', 'ended>error', 'ended>destroyed'],
      'error>destroyed',
    ])
    expect(canTransition('reconnecting', 'paused', 'paused')).toBe(true)
    expect(canTransition('reconnecting', 'paused', 'playing')).toBe(false)
  })
}
