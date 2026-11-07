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

/** 250 ms × 2ⁿ, capped at 5 s, with ±20 % jitter so players don't retry in lockstep. */
export function backoffDelay(attempt: number): number {
  return Math.min(250 * 2 ** attempt, 5000) * (0.8 + Math.random() * 0.4)
}

/** Resolves after `ms`, or rejects with the signal's reason once it aborts. */
export function sleep(ms: number, signal: AbortSignal): Promise<void> {
  return new Promise((resolve, reject) => {
    if (signal.aborted) return reject(signal.reason)
    const onAbort = () => {
      clearTimeout(timer)
      reject(signal.reason)
    }
    const timer = setTimeout(() => {
      signal.removeEventListener('abort', onAbort)
      resolve()
    }, ms)
    signal.addEventListener('abort', onAbort, { once: true })
  })
}

/** `promise`, or `timedOut()` after `ms`, or the signal's reason. */
export async function within<T>(
  promise: Promise<T>,
  ms: number,
  signal: AbortSignal,
  timedOut: () => Error,
): Promise<T> {
  const done = new AbortController()
  const timer = sleep(Math.max(0, ms), AbortSignal.any([signal, done.signal])).then(() => {
    throw timedOut()
  })
  try {
    return await Promise.race([promise, timer])
  } finally {
    done.abort()
    timer.catch(() => {})
  }
}

/** A deferred whose rejection is handled, for a promise that may reject before anyone awaits it. */
export function handledResolvers(): PromiseWithResolvers<void> {
  const resolvers = Promise.withResolvers<void>()
  resolvers.promise.catch(() => {})
  return resolvers
}

if (import.meta.vitest) {
  const { test, expect } = import.meta.vitest

  test('backoffDelay doubles from 250 ms, caps at 5 s, and stays within ±20 %', () => {
    for (const [attempt, base] of [
      [0, 250],
      [3, 2000],
      [10, 5000],
    ] as const) {
      const delay = backoffDelay(attempt)
      expect(delay).toBeGreaterThanOrEqual(base * 0.8)
      expect(delay).toBeLessThanOrEqual(base * 1.2)
    }
  })
}
