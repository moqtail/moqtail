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
import { DEFAULT_PRIORITY, type CapabilityBundles, type Identifiers, type SourceElementDefinition } from './define'
import type { Pad } from './pad'

/**
 * A pad matches a bundle when every capability of the bundle appears on the pad with an
 * equal value; extra pad capabilities are fine. Only `in` bundles describe what an element
 * accepts, so `out` bundles never match.
 */
export function matchesCapabilities(bundles: CapabilityBundles, pad: Pad): boolean {
  return bundles.some((bundle) =>
    bundle.every(
      (wanted) =>
        wanted.direction === 'in' && pad.capabilities.some((c) => c.code === wanted.code && c.value === wanted.value),
    ),
  )
}

/**
 * The source definition for `identifiers`: highest probe score, then lower priority, then
 * earlier in `definitions`. The caller lists player-registered definitions before built-in
 * ones, in registration order. Undefined when every score is 0.
 */
export function selectSource(
  definitions: readonly SourceElementDefinition[],
  identifiers: Identifiers,
): SourceElementDefinition | undefined {
  let best: SourceElementDefinition | undefined
  let bestScore = 0
  for (const definition of definitions) {
    const score = definition.probe(identifiers)
    const wins =
      score > bestScore ||
      (score === bestScore &&
        best !== undefined &&
        (definition.priority ?? DEFAULT_PRIORITY) < (best.priority ?? DEFAULT_PRIORITY))
    if (wins) {
      best = definition
      bestScore = score
    }
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
  // indexOf is -1 for a source, so the walk starts at the first transform.
  const start = transforms.indexOf(pad.element as TransformElement) + 1
  const transform = transforms.slice(start).find((t) => t.negotiate(pad))
  return transform ? [transform] : sinks.filter((s) => s.negotiate(pad))
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest
  const { testSource } = await import('@/player/testing/elements')

  describe('selectSource', () => {
    const scored = (name: string, score: number, priority?: number) =>
      testSource({ name, probe: () => score, ...(priority !== undefined && { priority }) })

    test('highest score, then lower priority, then earlier in the list; all zero is undefined', () => {
      const pick = (...defs: ReturnType<typeof scored>[]) => selectSource(defs, {})?.name
      expect(pick(scored('a', 1), scored('b', 2))).toBe('b')
      expect(pick(scored('a', 1), scored('b', 1, 50))).toBe('b')
      expect(pick(scored('a', 1, 50), scored('b', 1, 100))).toBe('a')
      expect(pick(scored('registered', 1), scored('builtin', 1))).toBe('registered')
      expect(pick(scored('a', 0, 1), scored('b', 0))).toBeUndefined()
    })
  })
}
