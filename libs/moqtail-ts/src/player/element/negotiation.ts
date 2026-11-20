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

/** Capability matching, probe selection, and pad routing. Pure functions over elements. */

import type { SinkElement, TransformElement } from './base'
import {
  DEFAULT_PRIORITY,
  type Capabilities,
  type Capability,
  type CapabilityBundles,
  type Identifiers,
  type SourceElementDefinition,
} from './define'
import type { Pad } from './pad'

/**
 * A pad matches an `in` bundle when, for each code the bundle lists, the pad has one of the
 * bundle's values for it; extra pad capabilities are fine. `out` bundles never match.
 */
export function matchesCapabilities(bundles: CapabilityBundles, pad: Pad): boolean {
  return bundles.some((bundle) => accepts(bundle, pad.capabilities))
}

/**
 * The capabilities of the source pad a transform makes for a pad with `capabilities`, using the
 * first `out` bundle that converts them: for each code the bundle lists, the pad's value stays
 * when the bundle lists it, else it becomes the bundle's value when the bundle lists one. Codes
 * the bundle doesn't list carry over. Undefined when no `in` bundle accepts the pad or no `out`
 * bundle converts it.
 */
export function convertedCapabilities(
  bundles: CapabilityBundles,
  capabilities: Capabilities,
): Capabilities | undefined {
  if (!bundles.some((bundle) => accepts(bundle, capabilities))) return undefined
  for (const bundle of bundles) {
    if (bundle[0]?.direction !== 'out') continue
    const converted = convert(bundle, capabilities)
    if (converted) return converted
  }
  return undefined
}

/**
 * The source definition for `identifiers`: among those whose probe accepts, the lower
 * priority, then the earlier in `definitions`. The caller lists player-registered definitions,
 * then plugins' in plugin order, then built-in ones. Undefined when none accepts.
 */
export function selectSource(
  definitions: readonly SourceElementDefinition[],
  identifiers: Identifiers,
): SourceElementDefinition | undefined {
  let best: SourceElementDefinition | undefined
  for (const definition of definitions) {
    if (!definition.probe(identifiers)) continue
    if (!best || (definition.priority ?? DEFAULT_PRIORITY) < (best.priority ?? DEFAULT_PRIORITY)) best = definition
  }
  return best
}

/**
 * Where a new source pad connects: the first transform after the pad's element (in chain
 * order) that negotiates it, otherwise every sink that does. Empty means unlinked.
 */
export function selectTargets(
  pad: Pad,
  transforms: readonly TransformElement[],
  sinks: readonly SinkElement[],
): (TransformElement | SinkElement)[] {
  // indexOf is -1 for a source, so the walk starts at the first transform
  const start = transforms.indexOf(pad.element as TransformElement) + 1
  const transform = transforms.slice(start).find((t) => t.negotiate(pad))
  return transform ? [transform] : sinks.filter((s) => s.negotiate(pad))
}

function accepts(bundle: Capabilities, capabilities: Capabilities): boolean {
  return bundle.every(
    (wanted) => wanted.direction === 'in' && capabilities.some((c) => c.code === wanted.code && listed(bundle, c)),
  )
}

function convert(bundle: Capabilities, capabilities: Capabilities): Capabilities | undefined {
  const output: Capabilities = []
  for (const capability of capabilities) {
    const values = bundle.filter((b) => b.code === capability.code)
    if (values.length === 0 || listed(bundle, capability)) output.push(capability)
    else if (values.length === 1) output.push(values[0]!)
    else return undefined
  }
  for (const code of new Set(bundle.map((b) => b.code))) {
    if (output.some((c) => c.code === code)) continue
    const values = bundle.filter((b) => b.code === code)
    if (values.length !== 1) return undefined
    output.push(values[0]!)
  }
  return output
}

function listed(bundle: Capabilities, capability: Capability): boolean {
  return bundle.some((b) => b.code === capability.code && b.value === capability.value)
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest
  const { testSource } = await import('@/player/testing/elements')
  const { CapabilityCode } = await import('./define')
  type CapabilityCode = import('./define').CapabilityCode
  const { PACKAGING, ROLE, CODEC } = CapabilityCode
  const of = (direction: 'in' | 'out', code: CapabilityCode, ...values: string[]): Capabilities =>
    values.map((value) => ({ direction, code, value }))
  const pad = (packaging: string, role: string): Capabilities => [
    ...of('out', PACKAGING, packaging),
    ...of('out', ROLE, role),
  ]
  const transform: CapabilityBundles = [
    [...of('in', PACKAGING, 'loc'), ...of('in', ROLE, 'video', 'audio')],
    [...of('out', PACKAGING, 'raw'), ...of('out', ROLE, 'video', 'audio')],
  ]

  describe('matchesCapabilities', () => {
    test('values of one code are alternatives, every code must hold, and out bundles never match', () => {
      const matches = (capabilities: Capabilities, bundles = transform) =>
        matchesCapabilities(bundles, { capabilities } as Pad)
      expect(matches(pad('loc', 'video'))).toBe(true)
      expect(matches(pad('loc', 'audio'))).toBe(true)
      expect(matches(pad('loc', 'subtitle'))).toBe(false)
      expect(matches(pad('cmaf', 'video'))).toBe(false)
      expect(matches(pad('raw', 'video'), [transform[1]!])).toBe(false)
    })
  })

  describe('convertedCapabilities', () => {
    test('listed values stay, unlisted ones take the only value, other codes carry over', () => {
      const codec = of('out', CODEC, 'avc3.64001f')
      expect(convertedCapabilities(transform, [...pad('loc', 'video'), ...codec])).toEqual([
        ...pad('raw', 'video'),
        ...codec,
      ])
      // The first out bundle that converts the pad
      const bundles = [transform[0]!, of('out', PACKAGING, 'raw', 'cmaf'), transform[1]!]
      expect(convertedCapabilities(bundles, pad('loc', 'audio'))).toEqual(pad('raw', 'audio'))
    })

    test('undefined without an accepting in bundle or a converting out bundle', () => {
      expect(convertedCapabilities(transform, pad('cmaf', 'video'))).toBeUndefined()
      const ambiguous: CapabilityBundles = [of('in', PACKAGING, 'loc'), of('out', PACKAGING, 'raw', 'cmaf')]
      expect(convertedCapabilities(ambiguous, pad('loc', 'video'))).toBeUndefined()
    })
  })

  describe('selectSource', () => {
    const probed = (name: string, accepts: boolean, priority?: number) =>
      testSource({ name, probe: () => accepts, ...(priority !== undefined && { priority }) })

    test('among the probes that accept, lower priority, then earlier in the list; none is undefined', () => {
      const pick = (...defs: ReturnType<typeof probed>[]) => selectSource(defs, {})?.name
      expect(pick(probed('a', false, 1), probed('b', true))).toBe('b')
      expect(pick(probed('a', true), probed('b', true, 50))).toBe('b')
      expect(pick(probed('a', true, 50), probed('b', true, 100))).toBe('a')
      expect(pick(probed('registered', true), probed('builtin', true))).toBe('registered')
      expect(pick(probed('a', false, 1), probed('b', false))).toBeUndefined()
    })
  })
}
