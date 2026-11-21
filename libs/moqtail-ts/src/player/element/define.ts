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
 * Element definitions: the metadata the registry and negotiation read before an
 * instance exists, and `defineElement`, which checks and returns one.
 */

import type { MoqtObject } from '@/model/data/object'
import type { TrackProperty } from '@/model/property/track_property'
import type { ThreadEnvironment } from '@/player/element/worker/player_worker'
import type { MsfSource } from '@/player/msf_url'
import type { BaseElement, SinkElement, SourceElement, TransformElement } from './base'
import type { ElementContext } from './context'
import type { CatalogTrack, ElementInterfaces, InterfaceKey } from './interfaces'

/** @public What a capability describes. */
export enum CapabilityCode {
  PACKAGING,
  ROLE,
  CODEC,
  MIME_TYPE,
  EVENT_TYPE,
  /** `worker` on a worker pad: its items live in the player's worker. */
  THREAD,
}

/** @public `in` describes a sink pad, `out` a source pad. */
export interface Capability {
  direction: 'in' | 'out'
  code: CapabilityCode
  value: string
}

/** @public */
export type Capabilities = Capability[]
/**
 * @public
 * Each bundle describes, in one direction, a pad the element takes or creates. Capabilities of a
 * bundle with the same code are alternatives, different codes must all hold. A transform's output
 * pad has its input pad's capabilities, each value its `out` bundle doesn't list replaced by the
 * bundle's one value for that code.
 */
export type CapabilityBundles = Capabilities[]

type UnionToIntersection<U> = (U extends any ? (x: U) => void : never) extends (x: infer I) => void ? I : never

/** @public The intersection of every interface listed in `provides`. */
export type Provided<P extends readonly InterfaceKey[]> = [P[number]] extends [never]
  ? unknown
  : UnionToIntersection<ElementInterfaces[P[number]]>

/**
 * @public
 * What identifies a track: `MsfSource` fields from the URI, plus catalog fields (MSF 5.2)
 * once a catalog exists. For a URI no element accepts alone, the player requests the track and
 * probes again with `isLive` and `trackProperties`, then with each object it reads in `objects`.
 */
export interface Identifiers extends Partial<MsfSource>, Partial<CatalogTrack> {
  /** The track properties of the SUBSCRIBE_OK or FETCH_OK. */
  trackProperties?: TrackProperty[]
  /** The track's objects read so far, from the start of its first group. */
  objects?: readonly MoqtObject[]
}

/** @public Among the probes that accept a track, the lowest priority wins. */
export const DEFAULT_PRIORITY = 100

/** @public What the registry stores. */
export interface ElementDefinition<
  E extends BaseElement = BaseElement,
  P extends readonly InterfaceKey[] = readonly InterfaceKey[],
> {
  readonly type: 'source' | 'transform' | 'sink'
  readonly name: string
  readonly capabilities: CapabilityBundles
  /**
   * Ranks the probes that accept a track: lower wins, then the earlier definition. Default selection
   * also keeps only the lowest of the sets sharing an `altGroup` and `lang`. Default 100.
   */
  readonly priority?: number
  /** The optional interfaces this element implements. */
  readonly provides?: P
  /**
   * Whether this element can handle `track`'s media, asked during selection before any instance exists.
   * Leaving it out means yes. Called with the track as the pad it would take carries it, so behind a decoder
   * its packaging is `raw`. Yes answers are kept per role, packaging, codec, MIME type, size, sample rate and
   * channels for the chain's life.
   * It isn't part of `negotiate`, which is synchronous, since decoders answer asynchronously and per rendition.
   */
  supports?(track: Identifiers): boolean | Promise<boolean>
  /**
   * Where the element's engines may run, asked before each instance's `init`. `both` or `worker` give
   * it `context.worker` while the player has one, and `worker` without one rejects. Default `main`.
   */
  threads?(environment: ThreadEnvironment): 'main' | 'worker' | 'both'
  /** Rejection might mean a browser API or a peer dependency is missing. */
  init(context: ElementContext, options?: Record<string, any>): Promise<E>
}

/** @public */
export interface SourceElementDefinition<
  E extends SourceElement = SourceElement,
  P extends readonly InterfaceKey[] = readonly InterfaceKey[],
> extends ElementDefinition<E, P> {
  readonly type: 'source'
  /** Whether this element handles the track. */
  probe(identifiers: Identifiers): boolean
}

/** @public */
export interface TransformElementDefinition<
  E extends TransformElement = TransformElement,
  P extends readonly InterfaceKey[] = readonly InterfaceKey[],
> extends ElementDefinition<E, P> {
  readonly type: 'transform'
}

/** @public */
export interface SinkElementDefinition<
  E extends SinkElement = SinkElement,
  P extends readonly InterfaceKey[] = readonly InterfaceKey[],
> extends ElementDefinition<E, P> {
  readonly type: 'sink'
}

/**
 * @public
 * Checks an element definition and returns it with its types preserved. The instance
 * `init` returns gets a read-only `definition` property pointing at the returned object.
 * Invalid definitions throw `TypeError`, since they're programming errors.
 */
export function defineElement<E extends SourceElement & Provided<P>, const P extends readonly InterfaceKey[] = []>(
  definition: SourceElementDefinition<E, P>,
): SourceElementDefinition<E, P>
export function defineElement<E extends TransformElement & Provided<P>, const P extends readonly InterfaceKey[] = []>(
  definition: TransformElementDefinition<E, P>,
): TransformElementDefinition<E, P>
export function defineElement<E extends SinkElement & Provided<P>, const P extends readonly InterfaceKey[] = []>(
  definition: SinkElementDefinition<E, P>,
): SinkElementDefinition<E, P>
export function defineElement(definition: ElementDefinition & { probe?: unknown }): ElementDefinition {
  validate(definition)

  const init = definition.init
  const defined: ElementDefinition = {
    ...definition,
    async init(context, options) {
      const element = await init.call(definition, context, options)
      Object.defineProperty(element, 'definition', { value: defined, enumerable: true })
      return element
    },
  }
  return defined
}

function validate(definition: ElementDefinition & { probe?: unknown }): void {
  const label = `Element "${String(definition.name)}"`
  checkName(label, definition.name)

  if (!['source', 'transform', 'sink'].includes(definition.type)) {
    throw new TypeError(`${label}: type must be "source", "transform", or "sink"`)
  }

  if ((definition.type === 'source') !== (typeof definition.probe === 'function')) {
    throw new TypeError(`${label}: probe is required for sources and not allowed otherwise`)
  }

  if (!Array.isArray(definition.capabilities) || !definition.capabilities.every(Array.isArray)) {
    throw new TypeError(`${label}: capabilities must be an array of capability bundles`)
  }

  for (const bundle of definition.capabilities) {
    if (bundle.length === 0 || bundle.some((c) => c.direction !== bundle[0]!.direction)) {
      throw new TypeError(`${label}: each capability bundle needs capabilities, all in one direction`)
    }
  }

  if (typeof definition.init !== 'function') throw new TypeError(`${label}: init must be a function`)

  for (const key of ['supports', 'threads'] as const) {
    if (definition[key] !== undefined && typeof definition[key] !== 'function') {
      throw new TypeError(`${label}: ${key} must be a function`)
    }
  }
}

/** Throws a `TypeError` for a name that is empty or has whitespace. */
export function checkName(label: string, name: unknown): void {
  if (typeof name !== 'string' || !/^\S+$/.test(name)) {
    throw new TypeError(`${label}: name must be non-empty and without whitespace`)
  }
}

/** `element.definition`, which stays unset while the element's own `init` runs. */
export function definitionOf(element: BaseElement): ElementDefinition | undefined {
  return element.definition as ElementDefinition | undefined
}

/** @public Narrows an element to an interface it lists in `provides`. */
export function hasInterface<E extends BaseElement, K extends InterfaceKey>(
  element: E,
  key: K,
): element is E & ElementInterfaces[K] {
  return definitionOf(element)?.provides?.includes(key) ?? false
}

if (import.meta.vitest) {
  const { describe, test, expect, expectTypeOf } = import.meta.vitest
  const { TestSource, CollectingSink, FakeMediaSink, detachedContext } = await import('@/player/testing/elements')
  type FakeMediaSink = import('@/player/testing/elements').FakeMediaSink
  const interfaces = await import('./interfaces')
  // Annotated, since copying a unique symbol into a new const widens it to symbol
  const MEDIA_SINK: typeof interfaces.MEDIA_SINK = interfaces.MEDIA_SINK
  const source = { type: 'source', name: 'src', capabilities: [], probe: () => true, init: async () => ({}) } as any

  describe('defineElement', () => {
    test.each([
      ['empty name', { ...source, name: '' }],
      ['whitespace in name', { ...source, name: 'a b' }],
      ['unknown type', { ...source, type: 'filter' }],
      ['source without probe', { ...source, probe: undefined }],
      ['sink with probe', { ...source, type: 'sink' }],
      ['capabilities not bundles', { ...source, capabilities: [{ direction: 'in', code: 0, value: 'x' }] }],
      ['an empty bundle', { ...source, capabilities: [[]] }],
      ['supports not a function', { ...source, supports: true }],
      ['threads not a function', { ...source, threads: 'worker' }],
      [
        'a bundle mixing directions',
        {
          ...source,
          capabilities: [
            [
              { direction: 'in', code: 0, value: 'x' },
              { direction: 'out', code: 0, value: 'y' },
            ],
          ],
        },
      ],
    ])('rejects %s', (_, definition) => {
      expect(() => defineElement(definition)).toThrow(TypeError)
    })

    test('init returns an instance with a read-only definition and its full type', async () => {
      const definition = defineElement({
        type: 'source',
        name: 'typed',
        capabilities: [],
        probe: () => true,
        init: async (context) => new TestSource(context),
      })

      const element = await definition.init(detachedContext())
      expectTypeOf(element).toEqualTypeOf<InstanceType<typeof TestSource>>()
      expect(element.definition).toBe(definition)
      expect(() => ((element as any).definition = {})).toThrow(TypeError)
      expect(hasInterface(element, MEDIA_SINK)).toBe(false)
    })

    test('provides is checked against what init returns', () => {
      const definition = defineElement({
        type: 'sink',
        name: 'media',
        capabilities: [],
        provides: [MEDIA_SINK],
        init: async (context) => new FakeMediaSink(context),
      })

      expectTypeOf(definition).toEqualTypeOf<SinkElementDefinition<FakeMediaSink, readonly [typeof MEDIA_SINK]>>()

      defineElement({
        type: 'sink',
        name: 'media',
        capabilities: [],
        provides: [MEDIA_SINK],
        // @ts-expect-error CollectingSink has no play(), seek(), ...
        init: async (context) => new CollectingSink(context),
      })
    })
  })
}
