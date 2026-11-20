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
 * routing pads as they appear, loading the transforms a pad needs to reach a sink, the
 * `nomorepads` and `ready` handshake, and teardown.
 */

import type { MOQtailClient } from '@/client/client'
import type { PlayerWorker } from '@/player/element/worker/player_worker'
import { PlayerError } from '@/player/error'
import { canonicalJson } from '@/player/util/json'
import { BaseElement, SinkElement, SourceElement, TransformElement } from './base'
import { InstanceContext, type Loaded, type PlayerHandle } from './context'
import {
  CapabilityCode,
  definitionOf,
  hasInterface,
  type Capabilities,
  type ElementDefinition,
  type Identifiers,
  type SinkElementDefinition,
  type SourceElementDefinition,
  type TransformElementDefinition,
} from './define'
import { MEDIA_SINK } from './interfaces'
import { planTransforms, routeOf, selectSource, selectTargets, sourceCapabilities } from './negotiation'
import { Pad, type Consumer, type PadOutput } from './pad'

/** What the chain needs from the player that owns it. */
export interface ChainHost {
  player: PlayerHandle
  client(): MOQtailClient
  /** Player-registered source definitions, then the active plugins', then built-in ones, each in order. */
  definitions(): readonly SourceElementDefinition[]
  /** The active plugins' switch sources, in plugin order. */
  switchSources(): readonly SourceElementDefinition[]
  /** Player-registered transform definitions, then the active plugins', then built-in ones, each in order. */
  transformDefinitions(): readonly TransformElementDefinition[]
  worker?(): PlayerWorker | undefined
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

/** The track fields `supports` answers are kept by. */
const MEDIA_FIELDS = [
  'role',
  'packaging',
  'codec',
  'mimeType',
  'width',
  'height',
  'samplerate',
  'channelConfig',
] as const

/** Loaded transforms a pad may pass through in a row, in case one's output pad gets it loaded again. */
const MAX_LOADED_DEPTH = 8

export class Chain {
  /** Instances shared through `context.load()`, by definition, then by tracks. */
  readonly #shared = new Map<ElementDefinition, Map<string, Loaded>>()
  /** The context of every element, for its holders. */
  readonly #contexts = new Map<BaseElement, InstanceContext>()
  readonly #declared: Declaration[] = []
  readonly #sources: SourceElement[] = []
  /** The declared transforms, in chain order. */
  readonly #transforms: TransformElement[] = []
  readonly #sinks: SinkElement[] = []
  /** Each transform the chain loaded for a pad, and the element whose pad it took. */
  readonly #loaded = new Map<BaseElement, BaseElement>()
  /** Transforms being loaded for a pad, which `init()` waits for. */
  readonly #loading = new Set<Promise<void>>()
  /** `supports` answers by definition, then by the track's media fields. */
  readonly #supported = new Map<ElementDefinition, Map<string, Promise<boolean>>>()
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

  addTransform(definition: TransformElementDefinition, options: Record<string, any> = {}): void {
    this.#declare(definition, options)
  }

  addSink(definition: SinkElementDefinition, options: Record<string, any> = {}): void {
    this.#declare(definition, options)
  }

  /**
   * Runs `init` in chain order, routes the pads created meanwhile, waits for `nomorepads`
   * from every top-level source and for the transforms being loaded, emits `nomorepads` on
   * the sinks, and resolves once every sink emitted `ready`. A media sink left without pads
   * rejects with `NO_MEDIA_SINK`; the player reports `init()` failures, so this one isn't
   * also sent to `host.fail`.
   * Resolves `ended` without negotiating the sinks when no source has a pad: every source
   * reported and has nothing to play, as with an empty `isComplete` catalog.
   */
  async init(signal: AbortSignal): Promise<'ready' | 'ended'> {
    const sources = this.#declarationsOf('source')
    const sinks = this.#declarationsOf('sink')
    for (const declaration of [...sources, ...this.#declarationsOf('transform'), ...sinks]) {
      signal.throwIfAborted()
      await declaration.context.create(declaration.definition, declaration.options)
    }
    const pending = this.#pending ?? []
    this.#pending = undefined
    try {
      for (const { pad, release } of pending) {
        const loading = pad.element.pads.includes(pad) ? this.#route(pad) : undefined
        if (loading) await abortable(loading, signal)
        release()
      }
    } finally {
      for (const { release } of pending) release()
    }

    await abortable(Promise.all(sources.map((d) => d.done.promise)), signal)
    // Catalog children create their pads after the loop above, so their transforms may still be loading
    while (this.#loading.size > 0) await abortable(Promise.all(this.#loading), signal)
    if (!this.#sources.some((s) => s.pads.some((p) => p.direction === 'src'))) return 'ended'
    const unlinked = this.#mediaSinkWithoutPads()
    if (unlinked) throw noMediaSink(unlinked)
    this.#negotiated = true
    for (const sink of this.#sinks) sink.emit('nomorepads')
    await abortable(Promise.all(sinks.map((d) => d.done.promise)), signal)
    return 'ready'
  }

  /**
   * Destroys sinks first, then declared transforms, then the ones loaded for pads, then sources.
   * Elements loaded through `context.load()` go with their last holder.
   */
  async destroy(): Promise<void> {
    this.#destroyed = true
    for (const { release } of this.#pending ?? []) release()
    for (const declaration of this.#declarationsOf('sink')) await declaration.context.destroy()
    for (const declaration of this.#declarationsOf('transform')) await declaration.context.destroy()
    for (const transform of [...this.#loaded.keys()]) await this.#contexts.get(transform)?.destroy()
    for (const declaration of this.#declarationsOf('source')) await declaration.context.destroy()
  }

  fail(error: PlayerError): void {
    if (!this.#destroyed) this.host.fail(error)
  }

  /** See `ElementContext.cost`. */
  async cost(track: Identifiers): Promise<number | undefined> {
    const sinks = this.#definitionsOf('sink')
    if (sinks.length === 0) return 0
    const source = selectSource(this.host.definitions(), track)
    if (!source) return undefined
    const capabilities = sourceCapabilities(source, track.role)
    if (!capabilities) return 0
    const declared = { transforms: this.#definitionsOf('transform'), sinks }
    const route = routeOf(capabilities, declared, this.host.transformDefinitions())
    if (!route) return undefined
    const supported = await Promise.all(
      route.path.map((step) => this.#supports(step.definition, receivedTrack(track, step.capabilities))),
    )
    return supported.every(Boolean) ? route.loaded : undefined
  }

  /** Listens to an element's emitted events. Called by `context.attach()`. */
  watch(element: BaseElement, context?: InstanceContext): void {
    if (context) this.#contexts.set(element, context)
    const onPadAdded = (pad: Pad) => this.#padAdded(pad)
    const onPadRemoved = (pad: Pad, failed: boolean) => this.#padRemoved(pad, failed)
    element.on('padadded', onPadAdded)
    element.on('padremoved', onPadRemoved)
    // Only top-level elements count; a loaded child reports to the element that loaded it
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

  /** Declared transforms are created one at a time, so `#transforms` stays in chain order. */
  register(element: BaseElement): void {
    if (element instanceof SourceElement) this.#sources.push(element)
    else if (element instanceof SinkElement) this.#sinks.push(element)
    else if (element instanceof TransformElement && this.#isDeclared(element)) this.#transforms.push(element)
  }

  /** Removes a destroyed instance from the chain's lists and stops listening to it. */
  unregister(element: BaseElement): void {
    this.#unwatch.get(element)?.()
    this.#unwatch.delete(element)
    this.#contexts.delete(element)
    this.#loaded.delete(element)
    for (const list of [this.#sources, this.#transforms, this.#sinks] as BaseElement[][]) {
      const index = list.indexOf(element)
      if (index !== -1) list.splice(index, 1)
    }
  }

  #declare(definition: ElementDefinition, options: Record<string, any>): void {
    this.#declared.push({ definition, options, context: new InstanceContext(this), done: Promise.withResolvers() })
  }

  #declarationsOf(type: ElementDefinition['type']): Declaration[] {
    return this.#declared.filter((d) => d.definition.type === type)
  }

  #definitionsOf(type: ElementDefinition['type']): ElementDefinition[] {
    return this.#declarationsOf(type).map((d) => d.definition)
  }

  #isDeclared(element: BaseElement): boolean {
    const context = this.#contexts.get(element)
    return this.#declared.some((d) => d.context === context)
  }

  #padAdded(pad: Pad): void {
    if (pad.direction !== 'src' || this.#destroyed) return
    if (this.#pending) {
      this.#pending.push({ pad, release: pad.holdWrites() })
      return
    }
    const loading = this.#route(pad)
    if (!loading) return
    // Writes made while the transform loads wait for it, in order
    const release = pad.holdWrites()
    void loading.then(release)
  }

  /**
   * Connects `pad` to the holder, declared transform or sinks that take it. Otherwise loads the first of
   * the fewest transforms that reach one, and returns that loading; the transform's own pad routes the same way.
   */
  #route(pad: Pad): Promise<void> | undefined {
    const position = this.#positionOf(pad.element)
    const targets = this.#takenByHolder(pad) ?? selectTargets(pad, this.#transforms, this.#sinks, position)
    for (const target of targets) this.#connect(pad, target)
    if (targets.length > 0) return undefined
    if (this.#loadedDepth(pad.element) >= MAX_LOADED_DEPTH) {
      this.#unlinked(pad, new Error(`${MAX_LOADED_DEPTH} transforms were loaded in a row`))
      return undefined
    }
    const plan = planTransforms(pad.capabilities, this.host.transformDefinitions(), (capabilities) => {
      // Never added to an element: negotiate only reads its capabilities
      const probe = new Pad('src', pad.element, capabilities)
      return selectTargets(probe, this.#transforms, this.#sinks, position).length > 0
    })
    if (!plan?.[0]) {
      this.#unlinked(pad)
      return undefined
    }
    const loading = this.#load(pad, plan[0])
    this.#loading.add(loading)
    void loading.then(() => this.#loading.delete(loading))
    return loading
  }

  /**
   * A new instance for this pad only, so it has one upstream and sits where the pad was.
   * Never rejects: the pad's held writes and `init()` wait on it.
   */
  async #load(pad: Pad, definition: TransformElementDefinition): Promise<void> {
    const context = new InstanceContext(this)
    try {
      const transform = await context.create(definition)
      if (this.#destroyed || !pad.element.pads.includes(pad)) {
        await context.destroy()
        return
      }
      if (!transform.negotiate(pad)) {
        await context.destroy()
        this.#unlinked(pad)
        return
      }
      this.#loaded.set(transform, pad.element)
      this.#connect(pad, transform)
    } catch (cause) {
      this.#unlinked(pad, cause)
      await context.destroy().catch(() => {}) // the pad's warning already carries the failure
    }
  }

  /** A loaded transform sits where the pad it took was, followed back to a declared element or a source. */
  #positionOf(element: BaseElement): BaseElement {
    let position = element
    while (this.#loaded.has(position)) position = this.#loaded.get(position)!
    return position
  }

  #loadedDepth(element: BaseElement): number {
    let depth = 0
    for (let current = element; this.#loaded.has(current); current = this.#loaded.get(current)!) depth++
    return depth
  }

  #connect(pad: Pad, target: Consumer): void {
    pad.connect(target, (error, destination) => this.#receiveFailed(error, destination))
    this.host.worker?.()?.connected(pad)
  }

  #unlinked(pad: Pad, cause?: unknown): void {
    // Event timelines are optional, so nobody taking their pad isn't worth a warning
    if (isEventTimeline(pad)) return
    const name = nameOf(pad.element)
    const message = `No element accepts a pad of "${name}"`
    this.host.warn(new PlayerError('PAD_UNLINKED', message, { element: name, ...(cause !== undefined && { cause }) }))
  }

  /** Yes answers are kept for the chain's life, so a catalog update doesn't ask the browser again. */
  #supports(definition: ElementDefinition, track: Identifiers): Promise<boolean> {
    if (!definition.supports) return Promise.resolve(true)
    const answers = this.#supported.get(definition) ?? new Map<string, Promise<boolean>>()
    this.#supported.set(definition, answers)
    const key = canonicalJson(MEDIA_FIELDS.map((field) => track[field]))
    const kept = answers.get(key)
    if (kept) return kept
    const answer = Promise.resolve(definition.supports(track))
    answers.set(key, answer)
    // A no or a failure may not hold later, as when a decoder frees up
    const forget = () => {
      if (answers.get(key) === answer) answers.delete(key)
    }
    answer.then((yes) => {
      if (!yes) forget()
    }, forget)
    return answer
  }

  /** A loaded child's pad goes to the first element holding it that negotiates it, if any. */
  #takenByHolder(pad: Pad): Consumer[] | undefined {
    const parent = this.holdersOf(pad.element).find(
      (h): h is SourceElement & Consumer => h instanceof SourceElement && !!h.receive && !!h.negotiate?.(pad),
    )
    return parent && [parent]
  }

  /**
   * A loaded transform left without sink pads goes. Pads lost to failures after negotiation are
   * fatal once a media sink has none left.
   */
  #padRemoved(pad: Pad, failed: boolean): void {
    const { element } = pad
    if (this.#loaded.has(element) && !this.#destroyed && !element.pads.some((p) => p.direction === 'sink')) {
      // After the transform's own listeners, which remove its outputs with `failed`
      queueMicrotask(() => void this.#contexts.get(element)?.destroy())
    }
    // A source pad's removal is reported after the sink pads it fed are gone
    if (!failed || pad.direction !== 'src' || !this.#negotiated || this.#destroyed) return
    const unlinked = this.#mediaSinkWithoutPads()
    if (unlinked) this.host.fail(noMediaSink(unlinked))
  }

  #receiveFailed(error: unknown, destination: Pad): void {
    const name = nameOf(destination.element)
    if (error instanceof PlayerError) {
      this.fail(error)
    } else {
      this.fail(new PlayerError('MEDIA_FAILURE', `"${name}" failed on an item`, { element: name, cause: error }))
    }
  }

  #mediaSinkWithoutPads(): SinkElement | undefined {
    return this.#sinks.find((s) => hasInterface(s, MEDIA_SINK) && !s.pads.some((p) => p.direction === 'sink'))
  }
}

function isEventTimeline(pad: Pad): boolean {
  return pad.capabilities.some((c) => c.code === CapabilityCode.PACKAGING && c.value === 'eventtimeline')
}

/** `track` as a pad with `capabilities` carries it, as a decoder's output carries it `raw`. */
function receivedTrack(track: Identifiers, capabilities: Capabilities): Identifiers {
  const packaging = capabilities.find((c) => c.code === CapabilityCode.PACKAGING)?.value
  return packaging === undefined ? track : { ...track, packaging }
}

function nameOf(element: BaseElement): string {
  return definitionOf(element)?.name ?? element.constructor.name
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
  const { testChain, testSource, testTransform, collectingSink, connected, flush, packet, track } =
    await import('@/player/testing/elements')
  type Sink = import('@/player/testing/elements').CollectingSink
  type Transform = import('@/player/testing/elements').TestTransform
  type Source = import('@/player/testing/elements').TestSource
  const { locCapabilities } = await import('@/player/elements/loc/track')
  const signal = () => new AbortController().signal
  const packaging = (value: string) => [{ direction: 'out' as const, code: CapabilityCode.PACKAGING, value }]
  /** A source adding a `loc` pad per role during init. */
  const locSource = (setup: (source: Source) => void = () => {}, ...roles: string[]) =>
    testSource({
      script: (s) => {
        setup(s)
        for (const role of roles) s.addPad(locCapabilities('out', role))
        s.emit('nomorepads')
      },
    })
  /** A chain whose host offers `definitions` to load. */
  const loading = (...definitions: TransformElementDefinition[]) =>
    testChain(undefined, undefined, undefined, () => definitions)

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
      // Children finish init, and register, before the parent that awaits them
      const [taken, declined] = chain.sources as import('@/player/testing/elements').TestSource[]
      // The first child's pad feeds a sink pad on the parent; the second went to the sink
      expect(parent!.pads.map((p) => p.peers[0]!.element)).toEqual([taken])
      expect(sink!.pads.map((p) => p.peers[0]!.element)).toEqual([declined])

      const upstream: unknown[] = []
      taken!.pads[0]!.onSignal((signal) => (upstream.push(signal), true))
      await taken!.pads[0]!.write(packet(1))
      await flush()
      parent!.pads[0]!.signal({ type: 'seek', mediaTime: 3 })
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
      let transform: Transform | undefined
      let sink: Sink | undefined
      chain.addSource(testSource({ script: (s) => (s.addPad(), s.emit('nomorepads')) }), {})
      chain.addTransform(testTransform({ setup: (t) => (transform = t) }))
      chain.addSink(collectingSink({ setup: (s) => (sink = s) }))
      await chain.init(signal())
      expect(sink!.pads).toHaveLength(1)
      expect(sink!.pads[0]!.peers[0]!.element).toBe(transform)
    })

    test('a pad no declared element takes goes through the fewest registered transforms that reach a sink, one instance per pad', async () => {
      const loaded: Transform[] = []
      const { chain, warnings } = loading(testTransform({ takes: 'loc', makes: 'raw', setup: (t) => loaded.push(t) }))
      let source: Source | undefined
      let sink: Sink | undefined
      chain.addSource(
        locSource((s) => (source = s), 'video', 'audio'),
        {},
      )
      chain.addSink(collectingSink({ packaging: 'raw', setup: (k) => (sink = k) }))
      await chain.init(signal())
      expect(loaded).toHaveLength(2)
      expect(sink!.pads.map((p) => p.peers[0]!.element)).toEqual(loaded)
      expect(sink!.pads.map((p) => p.capabilities.map((c) => c.value))).toEqual([
        ['raw', 'video'],
        ['raw', 'audio'],
      ])
      await source!.pads[1]!.write(packet(1))
      await flush()
      expect([sink!.items(sink!.pads[1]), warnings]).toEqual([[packet(1)], []])
    })

    test('nothing is loaded when a declared transform already makes what the sink takes', async () => {
      const loaded: Transform[] = []
      const { chain } = loading(testTransform({ takes: 'loc', makes: 'raw', setup: (t) => loaded.push(t) }))
      let declared: Transform | undefined
      let sink: Sink | undefined
      chain.addSource(locSource(undefined, 'video'), {})
      chain.addTransform(testTransform({ takes: 'loc', makes: 'raw', setup: (t) => (declared = t) }))
      chain.addSink(collectingSink({ packaging: 'raw', setup: (k) => (sink = k) }))
      await chain.init(signal())
      expect([loaded, sink!.pads[0]!.peers[0]!.element]).toEqual([[], declared])
    })

    test('a loaded transform sits where the pad was, so a declared transform after it still takes its output', async () => {
      let decoder: Transform | undefined
      let filter: Transform | undefined
      let source: Source | undefined
      let sink: Sink | undefined
      const { chain } = loading(testTransform({ takes: 'loc', makes: 'raw', setup: (t) => (decoder = t) }))
      chain.addSource(
        locSource((s) => (source = s), 'video'),
        {},
      )
      chain.addTransform(testTransform({ takes: 'raw', setup: (t) => (filter = t) }))
      chain.addSink(collectingSink({ packaging: 'raw', setup: (k) => (sink = k) }))
      await chain.init(signal())
      const upstream = (element: BaseElement) => element.pads.find((p) => p.direction === 'sink')!.peers[0]!.element
      expect([upstream(sink!), upstream(filter!), upstream(decoder!)]).toEqual([filter, decoder, source])
    })

    test('two hops: a → b → raw loads both, and a longer path is not chosen', async () => {
      const loaded: string[] = []
      const hop = (name: string, takes: string, makes: string) =>
        testTransform({ name, takes, makes, setup: () => loaded.push(name) })
      const { chain } = loading(
        hop('long-1', 'loc', 'x'),
        hop('long-2', 'x', 'y'),
        hop('long-3', 'y', 'raw'),
        hop('a', 'loc', 'b'),
        hop('b', 'b', 'raw'),
      )
      let sink: Sink | undefined
      chain.addSource(locSource(undefined, 'video'), {})
      chain.addSink(collectingSink({ packaging: 'raw', setup: (k) => (sink = k) }))
      await chain.init(signal())
      expect([loaded, sink!.pads.length]).toEqual([['a', 'b'], 1])
    })

    test('no chain reaches a sink: PAD_UNLINKED, then NO_MEDIA_SINK for a media sink left without pads', async () => {
      const { chain, warnings } = loading(testTransform({ takes: 'loc', makes: 'raw' }))
      chain.addSource(locSource(undefined, 'video'), {})
      chain.addSink(collectingSink({ media: true }))
      await expect(chain.init(signal())).rejects.toMatchObject({ code: 'NO_MEDIA_SINK', element: 'collecting-sink' })
      expect(warnings).toMatchObject([{ code: 'PAD_UNLINKED', element: 'test-source' }])
    })

    test('writes made while the transform loads arrive, in order', async () => {
      const gate = Promise.withResolvers<void>()
      const decoder = testTransform({ takes: 'loc', makes: 'raw' })
      const { chain } = loading({
        ...decoder,
        init: (context, options) => gate.promise.then(() => decoder.init(context, options)),
      })
      let source: Source | undefined
      let sink: Sink | undefined
      chain.addSource(
        locSource((s) => ((source = s), s.addPad())),
        {},
      )
      chain.addSink(collectingSink())
      chain.addSink(collectingSink({ packaging: 'raw', setup: (k) => (sink = k) }))
      await chain.init(signal())
      const pad = source!.addPad(locCapabilities('out', 'video'))
      const writes = [1, 2, 3].map((n) => pad.write(packet(n)))
      gate.resolve()
      await Promise.all(writes)
      await flush()
      expect(sink!.items()).toEqual([packet(1), packet(2), packet(3)])
    })

    test('a transform that fails to load, throws or declines the pad leaves it unlinked, and one loaded for a removed pad is released', async () => {
      const failed = new Error('no decoder')
      const throwing = (t: Transform) => {
        t.negotiate = () => {
          throw failed
        }
      }
      for (const definition of [
        { ...testTransform({ takes: 'loc', makes: 'raw' }), init: () => Promise.reject(failed) },
        testTransform({ takes: 'loc', makes: 'raw', setup: throwing }),
      ]) {
        const failing = loading(definition)
        failing.chain.addSource(locSource(undefined, 'video'), {})
        failing.chain.addSink(collectingSink({ packaging: 'raw' }))
        await failing.chain.init(signal())
        expect(failing.warnings).toMatchObject([{ code: 'PAD_UNLINKED', cause: failed }])
      }

      const declined: Transform[] = []
      const decline = (t: Transform) => {
        t.negotiate = () => false
        declined.push(t)
      }
      const declining = loading(testTransform({ takes: 'loc', makes: 'raw', setup: decline }))
      declining.chain.addSource(locSource(undefined, 'video'), {})
      declining.chain.addSink(collectingSink({ packaging: 'raw' }))
      await declining.chain.init(signal())
      expect([declining.warnings, declined[0]!.signal.aborted]).toMatchObject([[{ code: 'PAD_UNLINKED' }], true])

      const gate = Promise.withResolvers<void>()
      const loaded: Transform[] = []
      const decoder = testTransform({ takes: 'loc', makes: 'raw', setup: (t) => loaded.push(t) })
      const { chain, warnings } = loading({ ...decoder, init: (c, o) => gate.promise.then(() => decoder.init(c, o)) })
      let source: Source | undefined
      chain.addSource(
        locSource((s) => ((source = s), s.addPad())),
        {},
      )
      chain.addSink(collectingSink())
      chain.addSink(collectingSink({ packaging: 'raw' }))
      await chain.init(signal())
      source!.dropPad(source!.addPad(locCapabilities('out', 'video')))
      gate.resolve()
      await flush()
      expect([loaded[0]!.signal.aborted, warnings]).toEqual([true, []])
    })

    test('a transform whose pads keep what it takes is loaded only so many times in a row', async () => {
      const loaded: Transform[] = []
      // Its definition says it makes raw, but its instances' pads stay loc
      const echo = testTransform({ takes: 'loc', setup: (t) => loaded.push(t) })
      const { chain, warnings } = loading({ ...testTransform({ takes: 'loc', makes: 'raw' }), init: echo.init })
      chain.addSource(locSource(undefined, 'video'), {})
      chain.addSink(collectingSink({ packaging: 'raw' }))
      await chain.init(signal())
      expect([loaded.length, warnings]).toMatchObject([MAX_LOADED_DEPTH, [{ code: 'PAD_UNLINKED' }]])
    })

    test('an aborted init stops waiting for a transform that never loads', async () => {
      const { chain } = loading({ ...testTransform({ takes: 'loc', makes: 'raw' }), init: () => new Promise(() => {}) })
      chain.addSource(locSource(undefined, 'video'), {})
      chain.addSink(collectingSink({ packaging: 'raw' }))
      const abort = new AbortController()
      const init = chain.init(abort.signal)
      await flush()
      abort.abort(new Error('stopped'))
      await expect(init).rejects.toThrow('stopped')
    })

    test('a loaded transform goes with its input pad, and with the chain', async () => {
      const loaded: Transform[] = []
      const { chain } = loading(testTransform({ takes: 'loc', makes: 'raw', setup: (t) => loaded.push(t) }))
      let source: Source | undefined
      let sink: Sink | undefined
      chain.addSource(
        locSource((s) => (source = s), 'video', 'audio'),
        {},
      )
      chain.addSink(collectingSink({ packaging: 'raw', setup: (k) => (sink = k) }))
      await chain.init(signal())
      source!.dropPad(source!.pads[0]!)
      await flush()
      expect([loaded.map((t) => t.signal.aborted), sink!.pads.length]).toEqual([[true, false], 1])
      await chain.destroy()
      expect(loaded[1]!.signal.aborted).toBe(true)
    })

    test('cost: the transforms to load, undefined once an element on the way doesn’t support the track it takes, 0 without sinks', async () => {
      const asked: string[] = []
      const supports = (name: string) => (track: Identifiers) => {
        asked.push(`${name} ${track.packaging} ${track.codec}`)
        return track.codec !== 'hvc1'
      }
      const loc = testSource({ name: 'loc', packaging: 'loc', probe: (t) => t.packaging === 'loc' })
      const decoder = { ...testTransform({ takes: 'loc', makes: 'raw' }), supports: supports('decoder') }
      const video = (codec: string, extra = {}) => ({ ...track('v'), packaging: 'loc', role: 'video', codec, ...extra })

      expect(await testChain(() => [loc]).chain.cost(video('hvc1'))).toBe(0)
      const { chain } = testChain(
        () => [loc],
        undefined,
        undefined,
        () => [decoder],
      )
      chain.addSink(collectingSink({ packaging: 'raw' }))
      expect([await chain.cost(video('avc1')), await chain.cost(video('hvc1'))]).toEqual([1, undefined])
      // A catalog update brings new track objects; a yes is kept by media fields, a no is asked again
      expect([await chain.cost(video('avc1', { bitrate: 2 })), await chain.cost(video('hvc1'))]).toEqual([1, undefined])
      expect(asked.splice(0)).toEqual(['decoder loc avc1', 'decoder loc hvc1', 'decoder loc hvc1'])

      // A declared transform's answer counts like a sink's, and costs nothing; the sink is asked about its raw pad
      const declared = testChain(() => [loc])
      declared.chain.addTransform(decoder)
      declared.chain.addSink({ ...collectingSink({ packaging: 'raw' }), supports: supports('sink') })
      expect([await declared.chain.cost(video('avc1')), await declared.chain.cost(video('hvc1'))]).toEqual([
        0,
        undefined,
      ])
      expect(asked).toEqual(['decoder loc avc1', 'sink raw avc1', 'decoder loc hvc1', 'sink raw hvc1'])
      // Unreachable from the source's pad
      const cmaf = testChain(() => [loc])
      cmaf.chain.addSink(collectingSink())
      expect(await cmaf.chain.cost(video('avc1'))).toBeUndefined()
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
      // The pad was created and written before the sink existed; it's routed once it does, nothing lost
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
