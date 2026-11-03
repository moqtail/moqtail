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

/** Minimal elements for runtime tests. Never exported from the player entry. */

import type { MOQtailClient } from '../../client/client'
import { Tuple, TupleField } from '../../model/common/tuple'
import type { MoqtObject } from '../../model/data/object'
import { Emitter } from '../../util/emitter'
import { SinkElement, SourceElement, TransformElement } from '../element/base'
import { Chain, type ChainHost } from '../element/chain'
import type { ElementContext } from '../element/context'
import { CapabilityCode, defineElement, type Capabilities, type Identifiers } from '../element/define'
import { MEDIA_SINK, type CatalogTrack } from '../element/interfaces'
import type { Pad, PadEvent, PadItem, PadOutput } from '../element/pad'

export const VIDEO_CMAF: Capabilities = [
  { direction: 'out', code: CapabilityCode.ROLE, value: 'video' },
  { direction: 'out', code: CapabilityCode.PACKAGING, value: 'cmaf' },
]
const ACCEPT_CMAF = [[{ direction: 'in' as const, code: CapabilityCode.PACKAGING, value: 'cmaf' }]]

/** A source driven by the test: it adds pads, writes items, and reports `nomorepads`. */
export class TestSource extends SourceElement {
  addPad(capabilities: Capabilities = VIDEO_CMAF): Pad {
    return this.createPad('src', capabilities)
  }

  dropPad(pad: Pad, failed = false): void {
    this.removePad(pad, failed)
  }

  /** Signal the element was created with; aborted on destroy. */
  get signal(): AbortSignal {
    return this.context.signal
  }

  /** Exposes `context.load()` to tests. */
  get ctx(): ElementContext {
    return this.context
  }
}

/** A source definition. `script` runs inside `init`, e.g. to add pads and emit `nomorepads`. */
export function testSource(
  options: {
    name?: string
    probe?: (identifiers: Identifiers) => number
    priority?: number
    script?: (source: TestSource, options: Record<string, any> | undefined) => void | Promise<void>
  } = {},
) {
  return defineElement({
    type: 'source',
    name: options.name ?? 'test-source',
    capabilities: [],
    ...(options.priority !== undefined && { priority: options.priority }),
    probe: options.probe ?? (() => 1),
    async init(context, initOptions) {
      const source = new TestSource(context)
      await options.script?.(source, initOptions)
      return source
    },
  })
}

type Logged = PadItem | { type: 'event'; event: PadEvent }

/** Records every item and event per sink pad, and emits `ready` on `nomorepads`. */
export class CollectingSink extends SinkElement {
  readonly log = new Map<Pad, Logged[]>()
  /** Replace to reject connections or delay `receive()`. */
  accept = (pad: Pad) => this.matchesCapabilities(pad)
  slow: ((pad: Pad) => Promise<void>) | undefined
  /** What the pad's event listener returns. */
  consumeEvents = true

  constructor(context: ElementContext) {
    super(context)
    this.on('nomorepads', () => this.emit('ready'))
    this.on('padadded', (pad) => {
      this.log.set(pad, [])
      pad.onEvent((event) => {
        this.log.get(pad)?.push({ type: 'event', event })
        return this.consumeEvents
      })
    })
  }

  negotiate(pad: Pad): boolean {
    return this.accept(pad)
  }

  async receive(pad: Pad, item: PadItem): Promise<void> {
    await this.slow?.(pad)
    this.log.get(pad)?.push(item)
  }

  /** Everything logged on the only (or given) sink pad. */
  items(pad = this.pads[0]): Logged[] {
    return (pad && this.log.get(pad)) ?? []
  }
}

/** A sink definition accepting CMAF; `media` makes it a media sink for `NO_MEDIA_SINK` checks. */
export function collectingSink(
  options: { name?: string; media?: boolean; setup?: (sink: CollectingSink) => void } = {},
) {
  return defineElement({
    type: 'sink',
    name: options.name ?? 'collecting-sink',
    capabilities: ACCEPT_CMAF,
    // Pretends to be a media sink: only the runtime's NO_MEDIA_SINK check reads `provides` here.
    ...(options.media && { provides: [MEDIA_SINK] as unknown as [] }),
    async init(context) {
      const sink = new CollectingSink(context)
      options.setup?.(sink)
      return sink
    },
  })
}

/** Pass-through: one source pad per sink pad. `hook` sees each item before it's forwarded. */
export class TestTransform extends TransformElement {
  readonly #outputs = new Map<Pad, Pad>()
  hook: ((pad: Pad, item: PadItem) => void | Promise<void>) | undefined

  constructor(context: ElementContext) {
    super(context)
    this.on('padadded', (pad) => {
      if (pad.direction === 'sink') this.#outputs.set(pad, this.createPad('src', pad.capabilities))
    })
  }

  negotiate(pad: Pad): boolean {
    return pad.capabilities.some((c) => c.code === CapabilityCode.PACKAGING)
  }

  async receive(pad: Pad, item: PadItem): Promise<void> {
    await this.hook?.(pad, item)
    await this.#outputs.get(pad)?.write(item as PadOutput)
  }
}

/** A context for elements built outside a chain (transforms given to `addTransform`). */
export function detachedContext(): ElementContext {
  return {
    player: new Emitter(),
    client: {} as MOQtailClient,
    signal: new AbortController().signal,
    tracks: [],
    attach() {},
    load: () => Promise.reject(new Error('detached')),
    probe: () => undefined,
  }
}

/** A chain over a fake host that records warnings and failures. */
export function testChain(definitions: ChainHost['definitions'] = () => []) {
  const warnings: Error[] = []
  const failures: Error[] = []
  const chain = new Chain({
    player: new Emitter(),
    client: () => ({}) as MOQtailClient,
    definitions,
    warn: (e) => warnings.push(e),
    fail: (e) => failures.push(e),
  })
  return { chain, warnings, failures }
}

/** Lets queued microtasks and connection loops run. */
export const flush = () => new Promise<void>((resolve) => setTimeout(resolve))

/** A packet whose object is only a marker; the runtime never looks inside. */
export const packet = (n: number): PadItem => ({ type: 'packet', object: { n } as unknown as MoqtObject })

/** A catalog track in namespace `ns`. */
export function track(name: string, ns = 'ns'): CatalogTrack {
  const nameBytes = new TextEncoder().encode(name)
  return { name, nameBytes, namespace: new Tuple([TupleField.fromUtf8(ns)]), packaging: 'cmaf', isLive: true }
}

/** An initialised chain with one test source pad connected to one collecting sink. */
export async function connected(setup?: (sink: CollectingSink) => void) {
  let source: TestSource | undefined
  let sink: CollectingSink | undefined
  const { chain, warnings, failures } = testChain()
  chain.addSource(
    testSource({
      script: (s) => {
        source = s
        s.addPad()
        s.emit('nomorepads')
      },
    }),
    {},
  )
  chain.addSink(
    collectingSink({
      setup: (k) => {
        sink = k
        setup?.(k)
      },
    }),
  )
  await chain.init(new AbortController().signal)
  return { chain, warnings, failures, source: source!, sink: sink!, pad: source!.pads[0]! }
}
