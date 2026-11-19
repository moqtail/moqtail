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
import { TransformElement } from '@/player/element/base'
import type { ElementContext } from '@/player/element/context'
import { CapabilityCode, defineElement } from '@/player/element/define'
import type { Pad, PadEvent, PadItem, PadOutput } from '@/player/element/pad'
import type { Engine, EngineInit } from '@/player/worker'

/** A frame drawn: its media time, and the epoch time then, in seconds. */
export interface Draw {
  timestamp: number
  at: number
}

export class FrameFilterEngine implements Engine {
  readonly #output: (item: PadOutput) => Promise<void>
  readonly #report: (frames: number) => void
  #frames = 0

  constructor(init: EngineInit) {
    this.#output = init.outputs[0]!
    this.#report = init.report
  }

  async receive(item: PadItem): Promise<void> {
    if (item.type === 'frame') this.#report(++this.#frames)
    await this.#output(item)
  }

  async event(event: PadEvent): Promise<void> {
    await this.#output({ type: 'event', event })
  }
}

/** Records every frame drawn on an `OffscreenCanvas` in its thread from when it's created. */
export class DrawLog implements Engine {
  readonly #draws: Draw[] = []

  constructor() {
    const draws = this.#draws
    const draw = OffscreenCanvasRenderingContext2D.prototype.drawImage
    OffscreenCanvasRenderingContext2D.prototype.drawImage = function (
      this: OffscreenCanvasRenderingContext2D,
      ...args: unknown[]
    ) {
      const at = (performance.timeOrigin + performance.now()) / 1000
      draws.push({ timestamp: (args[0] as VideoFrame).timestamp / 1_000_000, at })
      return draw.apply(this, args as Parameters<typeof draw>)
    }
  }

  draws(): Draw[] {
    return this.#draws
  }

  receive(): void {}
  event(): void {}
}

const THREAD = { direction: 'out' as const, code: CapabilityCode.THREAD, value: 'worker' }

export class FrameFilter extends TransformElement {
  frames = 0
  /** Items that reached `receive()` on the main thread. */
  received = 0
  readonly #engines = new Map<Pad, FrameFilterEngine | Remote<FrameFilterEngine>>()

  constructor(context: ElementContext) {
    super(context)
    this.on('padadded', (pad) => {
      if (pad.direction === 'sink') this.#inputAdded(pad)
    })
  }

  negotiate(pad: Pad): boolean {
    return this.matchesCapabilities(pad)
  }

  async receive(pad: Pad, item: PadItem): Promise<void> {
    this.received++
    await this.#engines.get(pad)?.receive(item)
  }

  /** What the worker drew from now on. */
  drawLog(): Remote<DrawLog> {
    return this.context.worker!.create<DrawLog>('draw-log')
  }

  #inputAdded(input: Pad): void {
    const worker = this.context.worker
    const report = (frames: number) => {
      this.frames = Math.max(this.frames, frames)
    }
    const capabilities = input.capabilities.filter((c) => c.code !== CapabilityCode.THREAD)
    if (worker?.inWorker(input)) {
      const output = this.createPad('src', [...capabilities, THREAD])
      const engine = worker.create<FrameFilterEngine>('frame-filter', { outputs: [output], report })
      worker.link(input, engine)
      this.#engines.set(input, engine)
    } else {
      const output = this.createPad('src', capabilities)
      this.#engines.set(
        input,
        new FrameFilterEngine({ outputs: [(item) => output.write(item)], options: undefined, report }),
      )
    }
    input.onEvent((event) => {
      void this.#engines.get(input)?.event(event)
      return true
    })
  }
}

const raw = { direction: 'in' as const, code: CapabilityCode.PACKAGING, value: 'raw' }

export const frameFilter = defineElement({
  type: 'transform',
  name: 'frame-filter',
  capabilities: [[raw], [{ ...raw, direction: 'out' }]],
  threads: ({ worker }) => (worker?.has('element:frame-filter') ? 'both' : 'main'),
  init: async (context) => new FrameFilter(context),
})
