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
 * @public
 * A typed event emitter over a private `EventTarget`. `Events` maps each event name to
 * its listener signature, so the name decides what a listener receives and what
 * `emit()` must be given. A throwing listener doesn't affect the others: `EventTarget`
 * reports it to the global error handler.
 */
export class Emitter<Events extends { [K in keyof Events]: (...args: any[]) => void }> {
  readonly #target = new EventTarget()
  /** Maps each listener to the function given to `addEventListener`, which unpacks `detail`. */
  readonly #wrappers = new Map<keyof Events, Map<Function, EventListener>>()

  /** Adds `listener` for `event`. Adding the same function twice has no effect. */
  on<K extends keyof Events & string>(event: K, listener: Events[K]): void {
    let wrappers = this.#wrappers.get(event)
    if (!wrappers) {
      wrappers = new Map()
      this.#wrappers.set(event, wrappers)
    }
    if (wrappers.has(listener)) return
    const wrapper = (e: Event) => listener(...(e as CustomEvent<Parameters<Events[K]>>).detail)
    wrappers.set(listener, wrapper)
    this.#target.addEventListener(event, wrapper)
  }

  /** Removes `listener`, the same function given to {@link Emitter.on}, from `event`. */
  off<K extends keyof Events & string>(event: K, listener: Events[K]): void {
    const wrappers = this.#wrappers.get(event)
    const wrapper = wrappers?.get(listener)
    if (!wrappers || !wrapper) return
    wrappers.delete(listener)
    if (wrappers.size === 0) this.#wrappers.delete(event)
    this.#target.removeEventListener(event, wrapper)
  }

  /** Calls every listener of `event` with `args`. Allocates nothing when there are none. */
  emit<K extends keyof Events & string>(event: K, ...args: Parameters<Events[K]>): void {
    if (!this.#wrappers.has(event)) return
    this.#target.dispatchEvent(new CustomEvent(event, { detail: args }))
  }
}

if (import.meta.vitest) {
  const { describe, test, expect, vi } = import.meta.vitest

  type TestEvents = { ping: (n: number, s: string) => void; pong: () => void }

  describe('Emitter', () => {
    test('every listener receives the arguments, and off removes exactly one', () => {
      const emitter = new Emitter<TestEvents>()
      const a = vi.fn()
      const b = vi.fn()
      emitter.on('ping', a)
      emitter.on('ping', b)
      emitter.emit('ping', 1, 'x')
      emitter.off('ping', a)
      emitter.emit('ping', 2, 'y')
      expect(a.mock.calls).toEqual([[1, 'x']])
      expect(b.mock.calls).toEqual([
        [1, 'x'],
        [2, 'y'],
      ])
    })

    test('attaching the same function twice calls it once', () => {
      const emitter = new Emitter<TestEvents>()
      const a = vi.fn()
      emitter.on('pong', a)
      emitter.on('pong', a)
      emitter.emit('pong')
      expect(a).toHaveBeenCalledTimes(1)
    })

    test('a throwing listener does not stop the others', async () => {
      const emitter = new Emitter<TestEvents>()
      const b = vi.fn()
      // Node reports an EventTarget listener's error as an uncaught exception on the next tick
      const reported: unknown[] = []
      const saved = process.listeners('uncaughtException')
      process.removeAllListeners('uncaughtException')
      process.on('uncaughtException', (error) => reported.push(error))

      emitter.on('pong', () => {
        throw new Error('boom')
      })
      emitter.on('pong', b)
      emitter.emit('pong')
      await new Promise((resolve) => setTimeout(resolve))

      process.removeAllListeners('uncaughtException')
      for (const listener of saved) process.on('uncaughtException', listener)
      expect(b).toHaveBeenCalledTimes(1)
      expect(reported).toHaveLength(1)
    })

    test('emit with no listener creates no event', () => {
      const emitter = new Emitter<TestEvents>()
      const spy = vi.spyOn(globalThis, 'CustomEvent')
      const a = vi.fn()
      emitter.emit('pong')
      emitter.on('pong', a)
      emitter.off('pong', a)
      emitter.emit('pong')
      expect(spy).not.toHaveBeenCalled()
      spy.mockRestore()
    })
  })
}
