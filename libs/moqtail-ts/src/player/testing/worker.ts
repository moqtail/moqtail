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

import type { Remote } from 'comlink'
import { vi } from 'vitest'
import { SinkElement, TransformElement } from '@/player/element/base'
import type { ElementContext } from '@/player/element/context'
import { CapabilityCode, defineElement } from '@/player/element/define'
import type { Pad, PadEvent, PadItem, PadOutput } from '@/player/element/pad'
import type { Engine, EngineClass, EngineInit } from '@/player/element/worker/host'
import { forTransfer } from '@/player/element/worker/transfer'
import { serveWorker } from '@/player/worker'

export class PassEngine implements Engine {
  readonly #output: (item: PadOutput) => Promise<void>

  constructor(init: EngineInit) {
    this.#output = init.outputs[0]!
  }

  receive(item: PadItem): Promise<void> {
    return this.#output(item)
  }

  event(event: PadEvent): Promise<void> {
    return this.#output({ type: 'event', event })
  }
}

/** Records what it receives. Each instance is in `created`, in this thread when served in process. */
export class RecordEngine implements Engine {
  static readonly created: RecordEngine[] = []
  /** Set to hold every `receive()` until it resolves. */
  static gate: Promise<void> | undefined
  readonly items: PadOutput[] = []

  constructor() {
    RecordEngine.created.push(this)
  }

  async receive(item: PadItem): Promise<void> {
    await RecordEngine.gate
    this.items.push(item)
  }

  event(event: PadEvent): void {
    this.items.push({ type: 'event', event })
  }
}

export const TEST_ENGINES = { 'worker-pass': PassEngine, 'worker-record': RecordEngine }

/**
 * `serveWorker(engines)` in this thread, over a `MessageChannel`. Node only: it stubs the
 * global scope's messaging, which a worker's is. Left stubbed, so replies after a test still have
 * somewhere to go: the next one's wrapper ignores them.
 */
export function inProcessWorker(engines: Record<string, EngineClass> = TEST_ENGINES): Worker {
  const { port1, port2 } = new MessageChannel()
  vi.stubGlobal('addEventListener', port2.addEventListener.bind(port2))
  vi.stubGlobal('removeEventListener', port2.removeEventListener.bind(port2))
  vi.stubGlobal('postMessage', port2.postMessage.bind(port2))
  serveWorker(engines)
  port2.start()
  return Object.assign(port1, { terminate: () => port1.close() }) as unknown as Worker
}

class WorkerTransform extends TransformElement {
  readonly #engines = new Map<Pad, Remote<PassEngine>>()

  constructor(context: ElementContext) {
    super(context)
    this.on('padadded', (pad) => {
      if (pad.direction !== 'sink') return
      const thread = { direction: 'out' as const, code: CapabilityCode.THREAD, value: 'worker' }
      const output = this.createPad('src', [...pad.capabilities, thread])
      this.#engines.set(pad, this.context.worker!.create<PassEngine>('worker-pass', { outputs: [output] }))
      pad.onEvent((event) => {
        void this.#engines.get(pad)?.event(event)
        return true
      })
    })
  }

  negotiate(pad: Pad): boolean {
    return this.matchesCapabilities(pad)
  }

  async receive(pad: Pad, item: PadItem): Promise<void> {
    await this.#engines.get(pad)?.receive(forTransfer(item, pad))
  }
}

export const workerTransform = defineElement({
  type: 'transform',
  name: 'worker-transform',
  capabilities: [[{ direction: 'in', code: CapabilityCode.PACKAGING, value: 'cmaf' }]],
  threads: () => 'worker',
  init: async (context) => new WorkerTransform(context),
})

/** Forwards what reaches `receive()` to a `worker-record` engine, which `link()` creates and links its worker pads to. */
export class WorkerSink extends SinkElement {
  readonly received: PadItem[] = []
  #engine: Remote<RecordEngine> | undefined

  constructor(context: ElementContext) {
    super(context)
    this.on('nomorepads', () => this.emit('ready'))
  }

  negotiate(pad: Pad): boolean {
    return this.matchesCapabilities(pad)
  }

  link(): void {
    const worker = this.context.worker!
    const engine = worker.create<RecordEngine>('worker-record')
    this.#engine = engine
    for (const pad of this.pads) if (worker.inWorker(pad)) worker.link(pad, engine)
  }

  async receive(pad: Pad, item: PadItem): Promise<void> {
    this.received.push(item)
    await this.#engine?.receive(forTransfer(item, pad))
  }
}

export const workerSink = defineElement({
  type: 'sink',
  name: 'worker-sink',
  capabilities: [[{ direction: 'in', code: CapabilityCode.PACKAGING, value: 'cmaf' }]],
  threads: () => 'worker',
  init: async (context) => new WorkerSink(context),
})
