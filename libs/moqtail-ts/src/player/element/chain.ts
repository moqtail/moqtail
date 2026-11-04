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
 * The chain bookkeeping the player drives: initialising declared elements in order,
 * routing pads as they appear, the `nomorepads` and `ready` handshake, and teardown.
 */

import type { MOQtailClient } from '@/client/client'
import { PlayerError } from '@/player/error'
import { BaseElement, SinkElement, SourceElement, TransformElement } from './base'
import { InstanceContext, type Loaded, type PlayerHandle } from './context'
import {
  CapabilityCode,
  hasInterface,
  type ElementDefinition,
  type Identifiers,
  type SinkElementDefinition,
  type SourceElementDefinition,
} from './define'
import { MEDIA_SINK } from './interfaces'
import { selectTargets } from './negotiation'
import type { Consumer, Pad, PadOutput } from './pad'

/** What the chain needs from the player that owns it. */
export interface ChainHost {
  player: PlayerHandle
  client(): MOQtailClient
  /** Player-registered source definitions first, then built-in ones, each in registration order. */
  definitions(): readonly SourceElementDefinition[]
  warn(error: PlayerError): void
  fail(error: PlayerError): void
}

interface Declaration {
  definition: ElementDefinition
  options: Record<string, any>
  context: InstanceContext
  /** Resolved by `nomorepads` for sources, `ready` for sinks. */
  done: PromiseWithResolvers<void>
}

export class Chain {
  /** Instances shared through `context.load()`, by definition, then by tracks. */
  readonly #shared = new Map<ElementDefinition, Map<string, Loaded>>()
  /** The context of every element, for its holders. */
  readonly #contexts = new Map<BaseElement, InstanceContext>()
  readonly #declared: Declaration[] = []
  readonly #sources: SourceElement[] = []
  readonly #transforms: TransformElement[] = []
  readonly #sinks: SinkElement[] = []
  readonly #unwatch = new Map<BaseElement, () => void>()
  /** Source pads added before every declared element exists, routed once they do. Writes wait until then. */
  #pending: { pad: Pad; release: () => void }[] | undefined = []
  #negotiated = false
  #destroyed = false

  constructor(readonly host: ChainHost) {}

  /** Top-level and loaded sources, in load order. */
  get sources(): readonly SourceElement[] {
    return this.#sources
  }

  get sinks(): readonly SinkElement[] {
    return this.#sinks
  }

  /** Declares a source. Its `init` gets `{ ...options, source: identifiers }`. */
  addSource(definition: SourceElementDefinition, identifiers: Identifiers, options: object = {}): void {
    this.#declare(definition, { ...options, source: identifiers })
  }

  addTransform(element: TransformElement): void {
    this.#transforms.push(element)
    this.watch(element)
  }

  addSink(definition: SinkElementDefinition, options: object = {}): void {
    this.#declare(definition, { ...options })
  }

  /**
   * Runs `init` in chain order, routes the pads created meanwhile, waits for `nomorepads`
   * from every top-level source, emits `nomorepads` on the sinks, and resolves once every
   * sink emitted `ready`. A media sink left without pads rejects with `NO_MEDIA_SINK`;
   * the player reports `init()` failures, so this one isn't also sent to `host.fail`.
   * Resolves `ended` without negotiating the sinks when no source has a pad: every source
   * reported and has nothing to play, as with an empty `isComplete` catalog.
   */
  async init(signal: AbortSignal): Promise<'ready' | 'ended'> {
    const sources = this.#declared.filter((d) => d.definition.type === 'source')
    const sinks = this.#declared.filter((d) => d.definition.type === 'sink')
    for (const declaration of [...sources, ...sinks]) {
      signal.throwIfAborted()
      await declaration.context.create(declaration.definition, declaration.options)
    }
    const pending = this.#pending ?? []
    this.#pending = undefined
    for (const { pad, release } of pending) {
      if (pad.element.pads.includes(pad)) this.#route(pad)
      release()
    }

    await abortable(Promise.all(sources.map((d) => d.done.promise)), signal)
    if (!this.#sources.some((s) => s.pads.some((p) => p.direction === 'src'))) return 'ended'
    const unlinked = this.#mediaSinkWithoutPads()
    if (unlinked) throw noMediaSink(unlinked)
    this.#negotiated = true
    for (const sink of this.#sinks) sink.emit('nomorepads')
    await abortable(Promise.all(sinks.map((d) => d.done.promise)), signal)
    return 'ready'
  }

  /** Destroys sinks first, then transforms, then sources. Loaded elements go with their last holder. */
  async destroy(): Promise<void> {
    this.#destroyed = true
    for (const { release } of this.#pending ?? []) release()
    const declared = (type: string) => this.#declared.filter((d) => d.definition.type === type)
    for (const declaration of declared('sink')) await declaration.context.destroy()
    for (const transform of [...this.#transforms]) {
      await transform.destroy()
      this.unregister(transform)
    }
    for (const declaration of declared('source')) await declaration.context.destroy()
  }

  /** Listens to an element's signals. Called by `context.attach()`. */
  watch(element: BaseElement, context?: InstanceContext): void {
    if (context) this.#contexts.set(element, context)
    const onPadAdded = (pad: Pad) => this.#padAdded(pad)
    const onPadRemoved = (pad: Pad, failed: boolean) => this.#padRemoved(pad, failed)
    element.on('padadded', onPadAdded)
    element.on('padremoved', onPadRemoved)
    // Only top-level elements count; a loaded child reports to the element that loaded it.
    const declaration = this.#declared.find((d) => d.context === context)
    const event = declaration?.definition.type === 'source' ? 'nomorepads' : 'ready'
    const done = () => declaration?.done.resolve()
    if (declaration) element.on(event, done)
    this.#unwatch.set(element, () => {
      element.off('padadded', onPadAdded)
      element.off('padremoved', onPadRemoved)
      element.off(event, done)
    })
  }

  /** The elements holding a reference to a loaded instance; empty for top-level ones. */
  holdersOf(element: BaseElement): BaseElement[] {
    return [...(this.#contexts.get(element)?.holders ?? [])].flatMap((h) => h.element ?? [])
  }

  /** The loaded instances of `definition` that later loads share, by tracks. */
  sharingTable(definition: ElementDefinition): Map<string, Loaded> {
    let table = this.#shared.get(definition)
    if (!table) this.#shared.set(definition, (table = new Map()))
    return table
  }

  /** Adds an initialised instance to the chain's lists. */
  register(element: BaseElement): void {
    if (element instanceof SourceElement) this.#sources.push(element)
    else if (element instanceof SinkElement) this.#sinks.push(element)
    else if (element instanceof TransformElement) this.#transforms.push(element)
  }

  /** Removes a destroyed instance from the chain's lists and stops listening to it. */
  unregister(element: BaseElement): void {
    this.#unwatch.get(element)?.()
    this.#unwatch.delete(element)
    this.#contexts.delete(element)
    for (const list of [this.#sources, this.#transforms, this.#sinks] as BaseElement[][]) {
      const index = list.indexOf(element)
      if (index !== -1) list.splice(index, 1)
    }
  }

  #declare(definition: ElementDefinition, options: Record<string, any>): void {
    this.#declared.push({ definition, options, context: new InstanceContext(this), done: Promise.withResolvers() })
  }

  #padAdded(pad: Pad): void {
    if (pad.direction !== 'src' || this.#destroyed) return
    if (this.#pending) this.#pending.push({ pad, release: pad.holdWrites() })
    else this.#route(pad)
  }

  #route(pad: Pad): void {
    const targets = this.#takenByHolder(pad) ?? selectTargets(pad, this.#transforms, this.#sinks)
    for (const target of targets) pad.connect(target, (error, destination) => this.#receiveFailed(error, destination))
    if (targets.length > 0 || isEventTimeline(pad)) return
    const name = nameOf(pad.element)
    this.host.warn(new PlayerError('PAD_UNLINKED', `No element accepts a pad of "${name}"`, { element: name }))
  }

  /** A loaded child's pad goes to the first element holding it that negotiates it, if any. */
  #takenByHolder(pad: Pad): Consumer[] | undefined {
    const parent = this.holdersOf(pad.element).find(
      (h): h is SourceElement & Consumer => h instanceof SourceElement && !!h.receive && !!h.negotiate?.(pad),
    )
    return parent && [parent]
  }

  /** Pads lost to failures after negotiation are fatal once a media sink has none left. */
  #padRemoved(pad: Pad, failed: boolean): void {
    // A source pad's removal is reported after the sink pads it fed are gone.
    if (!failed || pad.direction !== 'src' || !this.#negotiated || this.#destroyed) return
    const unlinked = this.#mediaSinkWithoutPads()
    if (unlinked) this.host.fail(noMediaSink(unlinked))
  }

  #receiveFailed(error: unknown, destination: Pad): void {
    if (this.#destroyed) return
    const name = nameOf(destination.element)
    if (error instanceof PlayerError) this.host.fail(error)
    else
      this.host.fail(new PlayerError('MEDIA_FAILURE', `"${name}" failed on an item`, { element: name, cause: error }))
  }

  #mediaSinkWithoutPads(): SinkElement | undefined {
    return this.#sinks.find((s) => hasInterface(s, MEDIA_SINK) && !s.pads.some((p) => p.direction === 'sink'))
  }
}

/** Event timelines are optional, so nobody taking their pad isn't worth a warning. */
function isEventTimeline(pad: Pad): boolean {
  return pad.capabilities.some((c) => c.code === CapabilityCode.PACKAGING && c.value === 'eventtimeline')
}

// definition is unset while an element's init is still running.
function nameOf(element: BaseElement): string {
  return (element.definition as ElementDefinition | undefined)?.name ?? element.constructor.name
}

function noMediaSink(sink: SinkElement): PlayerError {
  const name = nameOf(sink)
  return new PlayerError('NO_MEDIA_SINK', `Media sink "${name}" has no connected pads`, { element: name })
}

/** Rejects with the signal's reason once it aborts. */
function abortable<T>(promise: Promise<T>, signal: AbortSignal): Promise<T> {
  return new Promise((resolve, reject) => {
    const onAbort = () => reject(signal.reason)
    if (signal.aborted) return onAbort()
    signal.addEventListener('abort', onAbort, { once: true })
    promise.then(resolve, reject).finally(() => signal.removeEventListener('abort', onAbort))
  })
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest
  const { testChain, testSource, collectingSink, connected, flush, packet, track, TestTransform, detachedContext } =
    await import('@/player/testing/elements')
  type Sink = import('@/player/testing/elements').CollectingSink
  const signal = () => new AbortController().signal
  const packaging = (value: string) => [{ direction: 'out' as const, code: CapabilityCode.PACKAGING, value }]

  describe('Chain', () => {
    test('routes each pad to every accepting sink, and warns once for a pad nobody takes', async () => {
      const { chain, warnings } = testChain()
      const sinks: Sink[] = []
      chain.addSource(
        testSource({
          script: (s) => {
            s.addPad()
            s.addPad(packaging('loc'))
            s.addPad(packaging('eventtimeline'))
            s.emit('nomorepads')
          },
        }),
        {},
      )
      chain.addSink(collectingSink({ setup: (s) => sinks.push(s) }))
      chain.addSink(collectingSink({ setup: (s) => sinks.push(s) }))
      await chain.init(signal())
      expect(sinks.map((s) => s.pads.length)).toEqual([1, 1])
      expect(warnings).toMatchObject([{ code: 'PAD_UNLINKED', element: 'test-source' }])
    })

    test('a source can take the pads of a child it loaded, or leave them to normal routing', async () => {
      const { chain } = testChain()
      const child = testSource({ name: 'child', script: (c) => void c.addPad() })
      let parent: import('@/player/testing/elements').TestSource | undefined
      let sink: Sink | undefined
      const received: unknown[] = []
      chain.addSource(
        testSource({
          script: async (s) => {
            parent = s
            s.negotiate = (pad) => (pad.element as typeof s).ctx.tracks[0]?.name === 'taken'
            s.receive = (_, item) => void received.push(item)
            await s.ctx.load(child, { tracks: [track('taken')] })
            await s.ctx.load(child, { tracks: [track('declined')] })
            s.emit('nomorepads')
          },
        }),
        {},
      )
      chain.addSink(collectingSink({ setup: (k) => (sink = k) }))
      await chain.init(signal())
      // Children finish init, and register, before the parent that awaits them.
      const [taken, declined] = chain.sources as import('@/player/testing/elements').TestSource[]
      // The first child's pad feeds a sink pad on the parent; the second went to the sink.
      expect(parent!.pads.map((p) => p.peers[0]!.element)).toEqual([taken])
      expect(sink!.pads.map((p) => p.peers[0]!.element)).toEqual([declined])

      const upstream: unknown[] = []
      taken!.pads[0]!.onEvent((event) => (upstream.push(event), true))
      await taken!.pads[0]!.write(packet(1))
      await flush()
      parent!.pads[0]!.sendEvent({ type: 'seek', mediaTime: 3 })
      expect([received, upstream]).toEqual([[packet(1)], [{ type: 'seek', mediaTime: 3 }]])

      taken!.dropPad(taken!.pads[0]!)
      expect(parent!.pads).toEqual([])
    })

    test('a parent takes the pad of an unshared child it loads after negotiation', async () => {
      const { chain } = testChain()
      const child = testSource({ name: 'child', script: (c) => void c.addPad() })
      let parent: import('@/player/testing/elements').TestSource | undefined
      chain.addSource(
        testSource({
          script: (s) => {
            parent = s
            s.negotiate = () => true
            s.receive = () => {}
            s.emit('nomorepads')
          },
        }),
        {},
      )
      chain.addSink(collectingSink())
      await chain.init(signal())
      const loaded = await parent!.ctx.load(child, { tracks: [track('v1')], shared: false })
      expect(parent!.pads.map((p) => p.peers[0]!.element)).toEqual([loaded])
    })

    test('a transform that accepts the pad takes it before the sinks', async () => {
      const { chain } = testChain()
      const transform = new TestTransform(detachedContext())
      let sink: Sink | undefined
      chain.addSource(testSource({ script: (s) => (s.addPad(), s.emit('nomorepads')) }), {})
      chain.addTransform(transform)
      chain.addSink(collectingSink({ setup: (s) => (sink = s) }))
      await chain.init(signal())
      expect(sink!.pads).toHaveLength(1)
      expect(sink!.pads[0]!.peers[0]!.element).toBe(transform)
    })

    test('sinks hear nomorepads only after every top-level source, and init waits for ready', async () => {
      const { chain } = testChain()
      const late = Promise.withResolvers<void>()
      let sink: Sink | undefined
      let heard = 0
      chain.addSource(testSource({ script: (s) => s.emit('nomorepads') }), {})
      const first: PadOutput = { type: 'properties', properties: { track: 'v1' } }
      chain.addSource(
        testSource({
          script: (s) => {
            void s.addPad().write(first) // before any sink exists
            void late.promise.then(() => s.emit('nomorepads'))
          },
        }),
        {},
      )
      chain.addSink(collectingSink({ setup: (s) => ((sink = s), s.on('nomorepads', () => heard++)) }))
      let initialised = false
      const init = chain.init(signal()).then(() => (initialised = true))
      await flush()
      // The pad was created and written before the sink existed; it's routed once it does, nothing lost.
      expect([heard, initialised, sink!.items()]).toEqual([0, false, [first]])
      late.resolve()
      await init
      expect(heard).toBe(1)
    })

    test('no source pads at all is ended; NO_MEDIA_SINK at negotiation, and later only for pads lost to failures', async () => {
      const empty = testChain()
      empty.chain.addSource(testSource({ script: (s) => s.emit('nomorepads') }), {})
      empty.chain.addSink(collectingSink({ media: true }))
      await expect(empty.chain.init(signal())).resolves.toBe('ended')

      const unlinked = testChain()
      unlinked.chain.addSource(testSource({ script: (s) => (s.addPad(packaging('loc')), s.emit('nomorepads')) }), {})
      unlinked.chain.addSink(collectingSink({ media: true }))
      await expect(unlinked.chain.init(signal())).rejects.toMatchObject({ code: 'NO_MEDIA_SINK' })

      const { chain, failures } = testChain()
      let source: import('@/player/testing/elements').TestSource | undefined
      chain.addSource(testSource({ script: (s) => ((source = s), s.addPad(), s.addPad(), s.emit('nomorepads')) }), {})
      chain.addSink(collectingSink({ media: true }))
      await chain.init(signal())
      source!.dropPad(source!.pads[0]!) // e.g. a catalog update
      expect(failures).toEqual([])
      source!.dropPad(source!.pads[0]!, true)
      expect(failures).toMatchObject([{ code: 'NO_MEDIA_SINK', element: 'collecting-sink' }])
    })

    test('destroy tears down sinks, then sources, and stops listening to them', async () => {
      const { chain, source, sink, warnings } = await connected()
      const order: string[] = []
      sink.on('padremoved', () => order.push('sink'))
      source.on('padremoved', () => order.push('source'))
      await chain.destroy()
      expect(order).toEqual(['sink', 'source'])
      expect(source.signal.aborted).toBe(true)
      expect([chain.sources.length, chain.sinks.length]).toEqual([0, 0])
      source.addPad(packaging('loc'))
      expect(warnings).toEqual([])
    })
  })
}
