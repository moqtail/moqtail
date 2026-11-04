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
}

/** @public `in` describes a sink pad, `out` a source pad. */
export interface Capability {
  direction: 'in' | 'out'
  code: CapabilityCode
  value: string
}

/** @public */
export type Capabilities = Capability[]
/** @public Each bundle describes one pad the element can create. */
export type CapabilityBundles = Capabilities[]

type UnionToIntersection<U> = (U extends any ? (x: U) => void : never) extends (x: infer I) => void ? I : never

/** @public The intersection of every interface listed in `provides`. */
export type Provided<P extends readonly InterfaceKey[]> = [P[number]] extends [never]
  ? unknown
  : UnionToIntersection<ElementInterfaces[P[number]]>

/**
 * @public
 * What identifies a track: `MsfSource` fields from the URI, plus catalog fields (MSF 5.2)
 * once a catalog exists.
 */
export type Identifiers = Partial<MsfSource> & Partial<CatalogTrack>

/** @public Probe ties are broken by priority, lower wins. */
export const DEFAULT_PRIORITY = 100

/** @public What the registry stores. */
export interface ElementDefinition<
  E extends BaseElement = BaseElement,
  P extends readonly InterfaceKey[] = readonly InterfaceKey[],
> {
  readonly type: 'source' | 'transform' | 'sink'
  readonly name: string
  readonly description?: string
  readonly capabilities: CapabilityBundles
  /** Breaks probe ties, a lower number wins. Default 100. */
  readonly priority?: number
  /** The optional interfaces this element implements. */
  readonly provides?: P
  /** Rejection might mean a browser API or a peer dependency is missing. */
  init(context: ElementContext, options?: Record<string, any>): Promise<E>
}

/** @public */
export interface SourceElementDefinition<
  E extends SourceElement = SourceElement,
  P extends readonly InterfaceKey[] = readonly InterfaceKey[],
> extends ElementDefinition<E, P> {
  readonly type: 'source'
  /** Confidence that this element handles the track: 0 for no match, higher for stronger matches. */
  probe(identifiers: Identifiers): number
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
  if (typeof definition.name !== 'string' || !/^\S+$/.test(definition.name)) {
    throw new TypeError(`${label}: name must be non-empty and without whitespace`)
  }
  if (!['source', 'transform', 'sink'].includes(definition.type)) {
    throw new TypeError(`${label}: type must be "source", "transform", or "sink"`)
  }
  if ((definition.type === 'source') !== (typeof definition.probe === 'function')) {
    throw new TypeError(`${label}: probe is required for sources and not allowed otherwise`)
  }
  if (!Array.isArray(definition.capabilities) || !definition.capabilities.every(Array.isArray)) {
    throw new TypeError(`${label}: capabilities must be an array of capability bundles`)
  }
  if (typeof definition.init !== 'function') throw new TypeError(`${label}: init must be a function`)
}

/** @public Narrows an element to an interface it lists in `provides`. */
export function hasInterface<E extends BaseElement, K extends InterfaceKey>(
  element: E,
  key: K,
): element is E & ElementInterfaces[K] {
  return element.definition.provides?.includes(key) ?? false
}

if (import.meta.vitest) {
  const { describe, test, expect, expectTypeOf } = import.meta.vitest
  const { TestSource, CollectingSink, detachedContext } = await import('@/player/testing/elements')
  const interfaces = await import('./interfaces')
  // Annotated, since copying a unique symbol into a new const widens it to symbol.
  const MEDIA_SINK: typeof interfaces.MEDIA_SINK = interfaces.MEDIA_SINK
  const source = { type: 'source', name: 'src', capabilities: [], probe: () => 1, init: async () => ({}) } as any

  describe('defineElement', () => {
    test.each([
      ['empty name', { ...source, name: '' }],
      ['whitespace in name', { ...source, name: 'a b' }],
      ['unknown type', { ...source, type: 'filter' }],
      ['source without probe', { ...source, probe: undefined }],
      ['sink with probe', { ...source, type: 'sink' }],
      ['capabilities not bundles', { ...source, capabilities: [{ direction: 'in', code: 0, value: 'x' }] }],
    ])('rejects %s', (_, definition) => {
      expect(() => defineElement(definition)).toThrow(TypeError)
    })

    test('init returns an instance with a read-only definition and its full type', async () => {
      const definition = defineElement({
        type: 'source',
        name: 'typed',
        capabilities: [],
        probe: () => 1,
        init: async (context) => new TestSource(context),
      })
      const element = await definition.init(detachedContext())
      expectTypeOf(element).toEqualTypeOf<InstanceType<typeof TestSource>>()
      expect(element.definition).toBe(definition)
      expect(() => ((element as any).definition = {})).toThrow(TypeError)
      expect(hasInterface(element, MEDIA_SINK)).toBe(false)
    })

    test('provides is checked against what init returns', () => {
      class FakeMediaSink extends CollectingSink {
        currentTime = 0
        buffered = {} as TimeRanges
        paused = true
        seeking = false
        ended = false
        playbackRate = 1
        setPlaybackRate() {}
        async play() {}
        async pause() {}
        async seek() {}
        addEventListener() {}
        removeEventListener() {}
        dispatchEvent() {
          return true
        }
      }
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
