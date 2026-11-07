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

import type { MOQtailClient } from '@/client/client'
import { Tuple, TupleField } from '@/model/common/tuple'
import type { MoqtObject } from '@/model/data/object'
import { Emitter } from '@/util/emitter'
import { SinkElement, SourceElement, TransformElement } from '@/player/element/base'
import { Chain, type ChainHost } from '@/player/element/chain'
import type { ElementContext } from '@/player/element/context'
import { CapabilityCode, defineElement, type Capabilities, type Identifiers } from '@/player/element/define'
import { MEDIA_SINK, MEDIA_SOURCE, type CatalogTrack, type MediaSinkElement } from '@/player/element/interfaces'
import type { Pad, PadItem, PadOutput, PadSignal } from '@/player/element/pad'
import type { PlayerEvents } from '@/player/player'
import { timeRanges } from './media'

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
    /** Lists `MEDIA_SOURCE` in `provides`, without implementing it. */
    media?: boolean
    script?: (source: TestSource, options: Record<string, any> | undefined) => void | Promise<void>
  } = {},
) {
  return defineElement({
    type: 'source',
    name: options.name ?? 'test-source',
    capabilities: [],
    ...(options.priority !== undefined && { priority: options.priority }),
    ...(options.media && { provides: [MEDIA_SOURCE] as unknown as [] }),
    probe: options.probe ?? (() => 1),
    async init(context, initOptions) {
      const source = new TestSource(context)
      await options.script?.(source, initOptions)
      return source
    },
  })
}

/** Records every item and event per sink pad, every signal, and emits `ready` on `nomorepads`. */
export class CollectingSink extends SinkElement {
  readonly log = new Map<Pad, PadOutput[]>()
  readonly signals: PadSignal[] = []
  /** Replace to reject connections or delay `receive()`. */
  accept = (pad: Pad) => this.matchesCapabilities(pad)
  slow: ((pad: Pad) => Promise<void>) | undefined
  /** What the pad's signal listener returns. */
  consumeSignals = true

  constructor(context: ElementContext) {
    super(context)
    this.on('nomorepads', () => this.emit('ready'))
    this.on('padadded', (pad) => {
      this.log.set(pad, [])
      pad.onEvent((event) => {
        this.log.get(pad)?.push({ type: 'event', event })
        return true
      })
      pad.onSignal((signal) => {
        this.signals.push(signal)
        return this.consumeSignals
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
  items(pad = this.pads[0]): PadOutput[] {
    return (pad && this.log.get(pad)) ?? []
  }
}

/** A sink definition accepting CMAF; `media` makes it a media sink for `NO_MEDIA_SINK` checks. */
export function collectingSink(options: { media?: boolean; setup?: (sink: CollectingSink) => void } = {}) {
  return defineElement({
    type: 'sink',
    name: 'collecting-sink',
    capabilities: ACCEPT_CMAF,
    // Pretends to be a media sink: only the runtime's NO_MEDIA_SINK check reads `provides` here
    ...(options.media && { provides: [MEDIA_SINK] as unknown as [] }),
    async init(context) {
      const sink = new CollectingSink(context)
      options.setup?.(sink)
      return sink
    },
  })
}

/** A media sink driven by the test: `dispatch()` fires media events, control calls are logged. */
export class FakeMediaSink extends CollectingSink implements MediaSinkElement {
  readonly #target = new EventTarget()
  readonly calls: string[] = []
  currentTime = 0
  paused = true
  seeking = false
  ended = false
  playbackRate = 1
  liveEdgeStart = NaN
  /** Replace to make `play()` reject, e.g. with a `NotAllowedError`. */
  playResult: () => Promise<void> = () => Promise.resolve()
  readonly addEventListener = this.#target.addEventListener.bind(this.#target)
  readonly removeEventListener = this.#target.removeEventListener.bind(this.#target)
  readonly dispatchEvent = this.#target.dispatchEvent.bind(this.#target)

  get buffered(): TimeRanges {
    return timeRanges([])
  }

  dispatch(type: string): void {
    this.dispatchEvent(new Event(type))
  }

  setPlaybackRate(rate: number): void {
    this.calls.push(`rate ${rate}`)
  }

  play(): Promise<void> {
    this.calls.push('play')
    return this.playResult()
  }

  async pause(): Promise<void> {
    this.calls.push('pause')
  }

  async seek(mediaTime: number): Promise<void> {
    this.calls.push(`seek ${mediaTime}`)
  }
}

export const fakeMediaSink = defineElement({
  type: 'sink',
  name: 'fake-media-sink',
  capabilities: ACCEPT_CMAF,
  provides: [MEDIA_SINK],
  init: async (context) => new FakeMediaSink(context),
})

/** Pass-through: one source pad per sink pad. */
export class TestTransform extends TransformElement {
  readonly #outputs = new Map<Pad, Pad>()

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
    await this.#outputs.get(pad)?.write(item)
  }
}

const fakePlayer = () => Object.assign(new Emitter<PlayerEvents>(), { options: {}, bufferController: undefined })

/** A context for elements built outside a chain (transforms given to `addTransform`). */
export function detachedContext(): ElementContext {
  return {
    player: fakePlayer(),
    client: {} as MOQtailClient,
    signal: new AbortController().signal,
    tracks: [],
    attach() {},
    load: () => Promise.reject(new Error('detached')),
    release: async () => {},
    probe: () => undefined,
  }
}

/** A chain over a fake host that records warnings and failures. `client` is read on each access. */
export function testChain(definitions: ChainHost['definitions'] = () => [], client: () => unknown = () => ({})) {
  const warnings: Error[] = []
  const failures: Error[] = []
  const chain = new Chain({
    player: fakePlayer(),
    client: () => client() as MOQtailClient,
    definitions,
    warn: (e) => warnings.push(e),
    fail: (e) => failures.push(e),
  })
  return { chain, warnings, failures }
}

/** Lets queued microtasks and connection loops run. */
export const flush = () => new Promise<void>((resolve) => setTimeout(resolve))

/** A few `flush()`es: enough for a track reader to decompress and apply what was pushed. */
export async function settle(): Promise<void> {
  for (let i = 0; i < 5; i++) await flush()
}

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
