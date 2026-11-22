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

import { CombinedFilter } from '@/client/publication/filter'
import { RequestErrorCode } from '@/model/constants'
import { requestError } from '@/model/messages'
import type { RequestError } from '@/model/messages'
import { InvalidFilterError, RANGE_FILTERS } from '@/model/parameters'
import type { MessageParameters } from '@/model/parameters'

/** Returns true when `trackAlias` is a valid (non-negative) track alias. Zero is a legal value. */
export function isValidTrackAlias(trackAlias: bigint | undefined): trackAlias is bigint {
  return trackAlias !== undefined && trackAlias >= 0n
}

/**
 * The REQUEST_ERROR INVALID_FILTER for a request whose Range Filters this client
 * cannot take: more ranges than the `limit` it advertised (with a limit of 0, any
 * filter at all), or a filter that cannot be applied. `undefined` when they are
 * fine, which includes carrying none.
 */
export function rangeFilterRejection(params: MessageParameters, limit: bigint): RequestError | undefined {
  try {
    assertWithinFilterRangeLimit(params, limit, 'request')
    new CombinedFilter(params)
    if (params.fillParameters) new CombinedFilter(params.fillParameters)

    return undefined
  } catch (error) {
    if (!(error instanceof InvalidFilterError)) throw error

    return requestError(RequestErrorCode.InvalidFilter, error.details)
  }
}

/**
 * Throws unless `params` stay within a MAX_FILTER_RANGES of `limit`: the peer's,
 * so a request it would have to refuse is never sent, or this client's own, for
 * a request it received.
 *
 * A limit of 0 allows no Range Filter parameter at all, not even a
 * zero-length removal. Otherwise the Ranges of every filter in the request count
 * against the limit together. FILL_PARAMETERS describes a separate fill fetch
 * stream, so the filters inside it count against a limit of their own.
 *
 * @throws :{@link InvalidFilterError} When the limit is exceeded.
 */
export function assertWithinFilterRangeLimit(params: MessageParameters, limit: bigint, context: string): void {
  const filters = RANGE_FILTERS.flatMap(([key]) => params[key] ?? [])
  if (filters.length > 0) {
    if (limit === 0n) throw new InvalidFilterError(context, 'no range filters are allowed (MAX_FILTER_RANGES is 0)')

    const count = BigInt(filters.reduce((sum, f) => sum + f.deltas.length, 0))
    if (count > limit) {
      throw new InvalidFilterError(context, `${count} filter ranges exceed MAX_FILTER_RANGES of ${limit}`)
    }
  }

  if (params.fillParameters) {
    assertWithinFilterRangeLimit(params.fillParameters, limit, `${context} (FILL_PARAMETERS)`)
  }
}

if (import.meta.vitest) {
  const { describe, it, expect } = import.meta.vitest
  const { encodeRanges } = await import('@/model/parameters')
  type RangeFilter = import('@/model/parameters').RangeFilter

  const twoRanges: RangeFilter = { setId: 0, deltas: encodeRanges([{ start: 0n, end: 0n }, { start: 5n }]) }
  const oneRange: RangeFilter = { setId: 0, deltas: encodeRanges([{ start: 1n, end: 1n }]) }
  const objectId = (setId: number, start: bigint, end: bigint): RangeFilter => ({
    setId,
    deltas: encodeRanges([{ start, end }]),
  })

  describe('assertWithinFilterRangeLimit', () => {
    const check = (params: MessageParameters, limit: bigint) => () =>
      assertWithinFilterRangeLimit(params, limit, 'test')

    it('allows any request without filters, whatever the limit', () => {
      expect(check({ forward: true }, 0n)).not.toThrow()
    })
    it('allows no filter at all, not even a removal, when the peer advertised none', () => {
      expect(check({ objectIdFilter: [twoRanges] }, 0n)).toThrow(InvalidFilterError)
      expect(check({ priorityFilter: [{ deltas: [] }] }, 0n)).toThrow(InvalidFilterError)
    })
    it('counts the ranges of every filter in the request together', () => {
      expect(check({ objectIdFilter: [twoRanges], subgroupFilter: [oneRange] }, 3n)).not.toThrow()
      expect(check({ objectIdFilter: [twoRanges], subgroupFilter: [oneRange] }, 2n)).toThrow(InvalidFilterError)
    })
    it('counts the filters inside FILL_PARAMETERS against a limit of their own', () => {
      const fill = { objectIdFilter: [twoRanges] }

      expect(check({ objectIdFilter: [twoRanges], fillParameters: fill }, 2n)).not.toThrow()

      const overFill = { objectIdFilter: [twoRanges], subgroupFilter: [oneRange] }

      expect(check({ fillParameters: overFill }, 2n)).toThrow(InvalidFilterError)
      expect(check({ fillParameters: { subgroupFilter: [oneRange] } }, 0n)).toThrow(InvalidFilterError)
    })
  })

  describe('rangeFilterRejection', () => {
    it('lets a request without range filters through, whatever the limit', () => {
      expect(rangeFilterRejection({ forward: true }, 0n)).toBeUndefined()
    })
    it('takes filters within the limit', () => {
      expect(rangeFilterRejection({ objectIdFilter: [objectId(0, 0n, 0n), objectId(1, 4n, 4n)] }, 2n)).toBeUndefined()
    })
    it('refuses any filter with a limit of 0, removals and fills included', () => {
      const removals: MessageParameters[] = [
        { subgroupFilter: [{ deltas: [] }] },
        { fillParameters: { objectIdFilter: [{ deltas: [] }] } },
      ]

      for (const params of removals) {
        expect(rangeFilterRejection(params, 0n)?.errorCode).toBe(RequestErrorCode.InvalidFilter)
      }
    })
    it('refuses more ranges than the limit, and a filter that cannot be applied', () => {
      const two = { objectIdFilter: [objectId(0, 0n, 0n), objectId(1, 4n, 4n)] }

      expect(rangeFilterRejection(two, 1n)?.errorCode).toBe(RequestErrorCode.InvalidFilter)

      const repeated = { objectIdFilter: [objectId(0, 0n, 0n), objectId(0, 4n, 4n)] }

      expect(rangeFilterRejection(repeated, 8n)?.errorCode).toBe(RequestErrorCode.InvalidFilter)

      const repeatedFill = { fillParameters: { objectIdFilter: [objectId(0, 0n, 0n), objectId(0, 1n, 1n)] } }

      expect(rangeFilterRejection(repeatedFill, 8n)?.errorCode).toBe(RequestErrorCode.InvalidFilter)
    })
  })

  describe('isValidTrackAlias', () => {
    it('accepts zero and positive aliases', () => {
      expect(isValidTrackAlias(0n)).toBe(true)
      expect(isValidTrackAlias(9999999999n)).toBe(true)
    })
    it('rejects undefined and negative aliases', () => {
      expect(isValidTrackAlias(undefined)).toBe(false)
      expect(isValidTrackAlias(-1n)).toBe(false)
    })
  })
}
