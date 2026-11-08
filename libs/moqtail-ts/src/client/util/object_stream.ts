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

import type { MoqtObject } from '@/model/data/object'

/**
 * A request's consumer-facing object stream. The consumer may cancel it while objects are
 * still in flight, so writes after that, or after it closed or errored, are dropped.
 */
export class ObjectStream {
  readonly readable: ReadableStream<MoqtObject>
  #controller!: ReadableStreamDefaultController<MoqtObject>
  #open = true

  constructor() {
    this.readable = new ReadableStream<MoqtObject>({
      start: (controller) => {
        this.#controller = controller
      },
      cancel: () => {
        this.#open = false
      },
    })
  }

  /** False once closed, cancelled by the consumer, or errored: nothing written is read. */
  get open(): boolean {
    return this.#open && this.#controller.desiredSize !== null
  }

  enqueue(object: MoqtObject): void {
    if (this.open) this.#controller.enqueue(object)
  }

  close(): void {
    if (!this.open) return
    this.#open = false
    this.#controller.close()
  }
}

if (import.meta.vitest) {
  const { test, expect } = import.meta.vitest
  const object = {} as MoqtObject

  test('writes after the consumer cancels, or after close, are dropped', async () => {
    const cancelled = new ObjectStream()
    await cancelled.readable.cancel()
    expect(cancelled.open).toBe(false)
    cancelled.enqueue(object)
    cancelled.close()

    const closed = new ObjectStream()
    closed.enqueue(object)
    closed.close()
    closed.enqueue(object)
    closed.close()
    const reader = closed.readable.getReader()
    expect(await reader.read()).toEqual({ done: false, value: object })
    expect((await reader.read()).done).toBe(true)
  })
}
