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
 * One operation queue per `SourceBuffer`. A source buffer runs one operation at a time,
 * so every append, remove, and `changeType` goes through `run()`, which waits for the
 * previous operation's `updateend` before starting the next.
 */
export class BufferQueue {
  #tail: Promise<void> = Promise.resolve()

  constructor(readonly buffer: SourceBuffer) {}

  /**
   * Runs `operation` once every earlier one has finished, and resolves on its `updateend`,
   * or right away when it didn't start an update (`changeType`, an empty remove). Rejects
   * with what `operation` throws, or on the buffer's `error` or `abort`. A rejection doesn't
   * stop later operations.
   */
  run(operation: (buffer: SourceBuffer) => void): Promise<void> {
    const result = this.#tail.then(() => this.#step(operation))
    this.#tail = result.catch(() => {})
    return result
  }

  #step(operation: (buffer: SourceBuffer) => void): Promise<void> {
    return new Promise((resolve, reject) => {
      operation(this.buffer) // a synchronous throw, e.g. QuotaExceededError, rejects
      if (!this.buffer.updating) return resolve()
      const listening = new AbortController()
      const done = (event: Event) => {
        listening.abort()
        if (event.type === 'updateend') resolve()
        else reject(new DOMException(`SourceBuffer ${event.type}`, 'InvalidStateError'))
      }
      // `error` and `abort` come before their `updateend`, so they settle first.
      for (const type of ['updateend', 'error', 'abort']) {
        this.buffer.addEventListener(type, done, { signal: listening.signal })
      }
    })
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest
  const { FakeSourceBuffer } = await import('../../testing/media')

  describe('BufferQueue', () => {
    test('runs one operation at a time, each after the previous updateend', async () => {
      const buffer = new FakeSourceBuffer()
      const queue = new BufferQueue(buffer as unknown as SourceBuffer)
      const order: string[] = []
      await Promise.all([
        queue.run((b) => b.appendBuffer(new Uint8Array(1))).then(() => order.push('a')),
        queue.run((b) => b.changeType('audio/mp4')).then(() => order.push('b')),
        queue.run((b) => b.remove(0, 0.5)).then(() => order.push('c')),
      ])
      expect(order).toEqual(['a', 'b', 'c'])
      expect(buffer.log).toEqual(['append 1', 'changeType audio/mp4', 'remove 0-0.5'])
      expect(buffer.ranges).toEqual([[0.5, 1]])
    })

    test('a throw or an error event rejects that operation only', async () => {
      const buffer = new FakeSourceBuffer()
      const queue = new BufferQueue(buffer as unknown as SourceBuffer)
      buffer.fail = 'quota'
      const quota = queue.run((b) => b.appendBuffer(new Uint8Array(1)))
      await expect(quota).rejects.toMatchObject({ name: 'QuotaExceededError' })
      buffer.fail = 'error'
      const failed = queue.run((b) => b.appendBuffer(new Uint8Array(2)))
      const next = queue.run((b) => b.appendBuffer(new Uint8Array(3)))
      await expect(failed).rejects.toMatchObject({ name: 'InvalidStateError' })
      await next
      expect(buffer.ranges).toEqual([[0, 1]])
    })
  })
}
