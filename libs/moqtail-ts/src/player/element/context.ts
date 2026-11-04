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
 * The context each element instance gets, and element sharing: loading a definition for
 * tracks that are already loaded returns the same instance and adds a reference. An
 * instance is destroyed once its last reference is released.
 */

import type { MOQtailClient } from '../../client/client'
import type { Emitter } from '../../util/emitter'
import { PlayerError } from '../error'
import type { MOQtailPlayerOptions, PlayerEvents } from '../player'
import type { BaseElement } from './base'
import type { Chain } from './chain'
import type { ElementDefinition, Identifiers, SourceElementDefinition } from './define'
import type { CatalogReader, CatalogTrack } from './interfaces'
import { selectSource } from './negotiation'

/** @public What elements see of the player: its events and options. */
export type PlayerHandle = Emitter<PlayerEvents> & { readonly options: Readonly<MOQtailPlayerOptions> }

/** @public Given to `init`, one per element instance. */
export interface ElementContext {
  readonly player: PlayerHandle
  /** Always the current client, it changes after a reconnect. */
  readonly client: MOQtailClient
  /** Aborted when the instance is destroyed. */
  readonly signal: AbortSignal
  /** The tracks this instance was loaded for, empty without a catalog. */
  readonly tracks: readonly CatalogTrack[]
  /** Set for elements the catalog loads, and inherited by what they load. */
  readonly catalog?: CatalogReader
  /** Called by the `BaseElement` constructor, so the player hears what the instance emits. */
  attach(element: BaseElement): void
  /** Returns the existing instance when the same definition and tracks are already loaded. */
  load<E extends BaseElement>(definition: ElementDefinition<E>, request?: ElementLoadRequest): Promise<E>
  /** Drops this element's reference to one it loaded; it's destroyed once no holder is left. */
  release(element: BaseElement): Promise<void>
  /** The source definition the player would pick for `identifiers`, as `source()` does. */
  probe(identifiers: Identifiers): SourceElementDefinition | undefined
}

/** @public */
export interface ElementLoadRequest {
  tracks?: readonly CatalogTrack[]
  /** The first load's options apply; a later load with different options is an error. */
  options?: Record<string, any>
  /** Defaults to the caller's catalog. */
  catalog?: CatalogReader
  /** `false` always creates a new instance that later loads never find, so its options aren't compared. */
  shared?: false
}

/** An instance loaded through `context.load()`. */
export interface Loaded {
  readonly context: InstanceContext
  readonly element: Promise<BaseElement>
  readonly options: Record<string, any>
  /** Removes it from the chain's sharing table, if it's in it. */
  readonly unshare: () => void
}

/** The runtime's `ElementContext`. */
export class InstanceContext implements ElementContext {
  readonly tracks: readonly CatalogTrack[]
  readonly catalog?: CatalogReader
  /** The contexts holding a reference to this instance; empty for top-level ones. */
  readonly holders = new Set<InstanceContext>()
  readonly #chain: Chain
  readonly #abort = new AbortController()
  /** Loaded instances this one holds a reference to. */
  readonly #holding = new Set<Loaded>()
  #element: BaseElement | undefined

  constructor(chain: Chain, tracks: readonly CatalogTrack[] = [], catalog?: CatalogReader) {
    this.#chain = chain
    this.tracks = tracks
    if (catalog) this.catalog = catalog
  }

  get player(): PlayerHandle {
    return this.#chain.host.player
  }

  get client(): MOQtailClient {
    return this.#chain.host.client()
  }

  get signal(): AbortSignal {
    return this.#abort.signal
  }

  /** The instance, once its constructor ran. */
  get element(): BaseElement | undefined {
    return this.#element
  }

  attach(element: BaseElement): void {
    this.#element = element
    this.#chain.watch(element, this)
  }

  probe(identifiers: Identifiers): SourceElementDefinition | undefined {
    return selectSource(this.#chain.host.definitions(), identifiers)
  }

  /** Runs `init` with this context and adds the instance to the chain. */
  async create<E extends BaseElement>(definition: ElementDefinition<E>, options?: Record<string, any>): Promise<E> {
    const element = await definition.init(this, options)
    this.#chain.register(element)
    // Destroyed while init was running: nothing will release it later.
    if (this.signal.aborted) await this.destroy()
    return element
  }

  async load<E extends BaseElement>(definition: ElementDefinition<E>, request: ElementLoadRequest = {}): Promise<E> {
    const tracks = request.tracks ?? []
    const options = request.options ?? {}
    const table = request.shared === false ? undefined : this.#chain.sharingTable(definition)
    const key = sharingKey(tracks)
    let loaded = table?.get(key)
    if (loaded && !deepEqual(loaded.options, options)) {
      throw new PlayerError('UNSUPPORTED_SOURCE', `Element "${definition.name}" is already loaded with other options`, {
        element: definition.name,
      })
    }
    if (!loaded) {
      const context = new InstanceContext(this.#chain, tracks, request.catalog ?? this.catalog)
      // Before init runs, so pads it creates already know their holder.
      context.holders.add(this)
      const created: Loaded = {
        context,
        element: context.create(definition, options),
        options,
        unshare: () => table?.get(key) === created && table.delete(key),
      }
      created.element.catch(created.unshare) // a failed init isn't shared
      table?.set(key, created)
      loaded = created
    }
    loaded.context.holders.add(this)
    this.#holding.add(loaded)
    return loaded.element as Promise<E>
  }

  /** Aborts the signal, destroys the element, then releases what it loaded. */
  async destroy(): Promise<void> {
    this.#abort.abort()
    const element = this.#element
    try {
      if (element) await element.destroy()
    } finally {
      if (element) this.#chain.unregister(element)
      for (const loaded of [...this.#holding]) await this.#drop(loaded)
    }
  }

  async release(element: BaseElement): Promise<void> {
    const loaded = [...this.#holding].find((l) => l.context.element === element)
    if (loaded) await this.#drop(loaded)
  }

  async #drop(loaded: Loaded): Promise<void> {
    this.#holding.delete(loaded)
    loaded.context.holders.delete(this)
    if (loaded.context.holders.size > 0) return
    loaded.unshare()
    await loaded.context.destroy()
  }
}

/** Full track names (namespace and name bytes), sorted, so the order of `tracks` doesn't matter. */
function sharingKey(tracks: readonly CatalogTrack[]): string {
  return tracks
    .map((t) => JSON.stringify([t.namespace.fields.map((f) => Array.from(f.value)), Array.from(t.nameBytes)]))
    .sort()
    .join('\n')
}

/** Structural equality for load options: plain objects, arrays, typed arrays, primitives. */
function deepEqual(a: unknown, b: unknown): boolean {
  if (Object.is(a, b)) return true
  if (typeof a !== 'object' || typeof b !== 'object' || a === null || b === null) return false
  if (Object.getPrototypeOf(a) !== Object.getPrototypeOf(b)) return false
  const keys = Object.keys(a)
  return (
    keys.length === Object.keys(b).length &&
    keys.every((k) => deepEqual((a as Record<string, unknown>)[k], (b as Record<string, unknown>)[k]))
  )
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest
  const { testChain, testSource, track } = await import('../testing/elements')
  type Source = import('../testing/elements').TestSource

  /** A chain with `count` top-level sources, returned once initialised. */
  async function parents(count: number) {
    const { chain } = testChain()
    const sources: Source[] = []
    for (let i = 0; i < count; i++) {
      chain.addSource(testSource({ script: (s) => (sources.push(s), s.emit('nomorepads')) }), {})
    }
    await chain.init(new AbortController().signal)
    return { chain, sources }
  }
  const child = testSource({ name: 'child' })
  const teardown = (source: Source) => (source.ctx as InstanceContext).destroy()

  describe('ElementContext.load', () => {
    test('one instance for the same definition and tracks, destroyed after its last holder', async () => {
      const { chain, sources } = await parents(2)
      const [a, b] = sources as [Source, Source]
      const first = await a.ctx.load(child, { tracks: [track('v1'), track('a1')] })
      const second = await b.ctx.load(child, { tracks: [track('a1'), track('v1')] })
      const other = await b.ctx.load(child, { tracks: [track('v2')] })
      expect(second).toBe(first)
      expect(other).not.toBe(first)
      expect(chain.sources).toEqual([a, b, first, other])

      await teardown(a)
      expect(first.signal.aborted).toBe(false)
      await teardown(b)
      expect([first.signal.aborted, other.signal.aborted]).toEqual([true, true])
      expect(chain.sources).toEqual([])
    })

    test('a later load with different options is a PlayerError', async () => {
      const { sources } = await parents(2)
      await sources[0]!.ctx.load(child, { tracks: [track('v1')], options: { switchFrom: true } })
      await expect(
        sources[1]!.ctx.load(child, { tracks: [track('v1')], options: { switchFrom: true } }),
      ).resolves.toBeDefined()
      await expect(sources[1]!.ctx.load(child, { tracks: [track('v1')] })).rejects.toMatchObject({
        name: 'PlayerError',
        code: 'UNSUPPORTED_SOURCE',
        element: 'child',
      })
    })

    test('release drops one child, unless another holder keeps it', async () => {
      const { sources } = await parents(2)
      const [a, b] = sources as [Source, Source]
      const v1 = await a.ctx.load(child, { tracks: [track('v1')] })
      const v2 = await a.ctx.load(child, { tracks: [track('v2')] })
      expect(await b.ctx.load(child, { tracks: [track('v2')] })).toBe(v2)

      await a.ctx.release(v1)
      await a.ctx.release(v2)
      expect([v1.signal.aborted, v2.signal.aborted]).toEqual([true, false])
      await b.ctx.release(v2)
      expect(v2.signal.aborted).toBe(true)
    })

    test('unshared loads of one track coexist, with their own options', async () => {
      const { sources } = await parents(1)
      const parent = sources[0]!
      const shared = await parent.ctx.load(child, { tracks: [track('v1')] })
      const first = await parent.ctx.load(child, { tracks: [track('v1')], shared: false })
      const second = await parent.ctx.load(child, { tracks: [track('v1')], options: { other: 1 }, shared: false })
      expect(new Set([shared, first, second]).size).toBe(3)
      expect(await parent.ctx.load(child, { tracks: [track('v1')] })).toBe(shared)
      await parent.ctx.release(first)
      expect([first.signal.aborted, second.signal.aborted]).toEqual([true, false])
    })

    test('releasing a parent releases its children recursively, except what another holder keeps', async () => {
      const { sources } = await parents(2)
      const [a, b] = sources as [Source, Source]
      const middle = await a.ctx.load(child, { tracks: [track('v1')] })
      const shared = await middle.ctx.load(child, { tracks: [track('timeline')] })
      expect(await b.ctx.load(child, { tracks: [track('timeline')] })).toBe(shared)

      await teardown(a)
      expect([middle.signal.aborted, shared.signal.aborted]).toEqual([true, false])
      await teardown(b)
      expect(shared.signal.aborted).toBe(true)
    })
  })
}
