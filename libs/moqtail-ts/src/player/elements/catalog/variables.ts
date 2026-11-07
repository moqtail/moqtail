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

/** Variable substitution (MSF 5.4) over a parsed catalog, which also rejects unsafe keys. */

import { PlayerError } from '@/player/error'
import { invalidCatalog } from './strings'

const VARIABLE = /%([A-Za-z0-9_-]+)%/g
const SAFE_VALUE = /^[A-Za-z0-9_@-]*$/
const UNSAFE_KEYS = new Set(['__proto__', 'constructor', 'prototype'])

/**
 * Replaces `%name%` in every string value (not key) of a `JSON.parse` result, in place,
 * and returns it. A variable that's missing or has a value outside the MSF 5.4.1
 * character set stays as written, with a `VARIABLE_SKIPPED` warning.
 * @throws PlayerError for a `__proto__`, `constructor`, or `prototype` key anywhere.
 */
export function substituteVariables(
  value: unknown,
  variables: Record<string, string>,
  warnings: PlayerError[],
): unknown {
  if (typeof value === 'string') return substitute(value, variables, warnings)
  if (typeof value !== 'object' || value === null) return value
  const container = value as Record<string, unknown>
  for (const key of Object.keys(container)) {
    if (UNSAFE_KEYS.has(key)) throw invalidCatalog(`the key "${key}" isn't allowed`)
    container[key] = substituteVariables(container[key], variables, warnings)
  }
  return value
}

function substitute(s: string, variables: Record<string, string>, warnings: PlayerError[]): string {
  return s.replace(VARIABLE, (reference, name: string) => {
    // Own keys only, so `%constructor%` can't reach the prototype
    const value = Object.hasOwn(variables, name) ? variables[name] : undefined
    if (value === undefined) {
      warnings.push(new PlayerError('VARIABLE_SKIPPED', `Variable ${reference} isn't in the URL fragment`))
      return reference
    }
    if (!SAFE_VALUE.test(value)) {
      warnings.push(new PlayerError('VARIABLE_SKIPPED', `Variable ${reference} has unsafe characters (MSF 5.4.1)`))
      return reference
    }
    return value
  })
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest
  const run = (json: string, variables: Record<string, string>) => {
    const warnings: PlayerError[] = []
    return { value: substituteVariables(JSON.parse(json), variables, warnings), warnings }
  }

  describe('substituteVariables', () => {
    test('substitutes string values, recursively, but not keys', () => {
      const { value, warnings } = run('{"%id%": ["tracking-%id%", {"c4m": "%token%"}, 1]}', {
        id: 'bob',
        token: '12@34',
      })
      expect(value).toEqual({ '%id%': ['tracking-bob', { c4m: '12@34' }, 1] })
      expect(warnings).toEqual([])
    })

    test('keeps unsafe and unresolved variables as written, with a warning', () => {
      const { value, warnings } = run('["%a%", "%b%", "%constructor%"]', { a: 'x;y' })
      expect(value).toEqual(['%a%', '%b%', '%constructor%'])
      expect(warnings.map((w) => w.code)).toEqual(['VARIABLE_SKIPPED', 'VARIABLE_SKIPPED', 'VARIABLE_SKIPPED'])
    })

    test('rejects unsafe keys anywhere', () => {
      expect(() => run('{"tracks": [{"__proto__": {}}]}', {})).toThrow(/__proto__/)
      expect(() => run('{"constructor": 1}', {})).toThrow(PlayerError)
    })
  })
}
