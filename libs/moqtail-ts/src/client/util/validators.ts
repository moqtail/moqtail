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

import { MessageParameter, ReasonPhrase } from '../../model'
import { RequestError, RequestErrorCode } from '../../model/control'
import { InvalidFilterError } from '../../model/parameter/message/range_filter'
import { NetFilter } from '../../model/parameter/message/net_filter'

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
export function rangeFilterRejection(params: readonly MessageParameter[], limit: bigint): RequestError | undefined {
  try {
    assertWithinFilterRangeLimit(params, limit, 'request')
    new NetFilter(params)
    for (const fill of params.filter(MessageParameter.isFillParameters)) new NetFilter(fill.parameters)
    return undefined
  } catch (error) {
    if (!(error instanceof InvalidFilterError)) throw error
    return new RequestError(RequestErrorCode.InvalidFilter, 0n, new ReasonPhrase(error.details))
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
export function assertWithinFilterRangeLimit(
  params: readonly MessageParameter[],
  limit: bigint,
  context: string,
): void {
  const filters = params.filter(MessageParameter.isRangeFilter)
  if (filters.length > 0) {
    if (limit === 0n) throw new InvalidFilterError(context, 'no range filters are allowed (MAX_FILTER_RANGES is 0)')
    const count = BigInt(filters.reduce((sum, f) => sum + f.rangeCount, 0))
    if (count > limit) {
      throw new InvalidFilterError(context, `${count} filter ranges exceed MAX_FILTER_RANGES of ${limit}`)
    }
  }
  for (const fill of params.filter(MessageParameter.isFillParameters)) {
    assertWithinFilterRangeLimit(fill.parameters, limit, `${context} (FILL_PARAMETERS)`)
  }
}

if (import.meta.vitest) {
  const { describe, it, expect } = import.meta.vitest
  describe('assertWithinFilterRangeLimit', async () => {
    const { Forward, RangeFilter, RangeFilterKind, FillParameters, encodeRanges } =
      await import('../../model/parameter/message')
    const twoRanges = new RangeFilter(
      RangeFilterKind.ObjectId,
      0,
      undefined,
      encodeRanges([{ start: 0n, end: 0n }, { start: 5n }]),
    )
    const oneRange = new RangeFilter(RangeFilterKind.Subgroup, 0, undefined, encodeRanges([{ start: 1n, end: 1n }]))
    const check = (params: MessageParameter[], limit: bigint) => () =>
      assertWithinFilterRangeLimit(params, limit, 'test')

    it('allows any request without filters, whatever the limit', () => {
      expect(check([new Forward(true)], 0n)).not.toThrow()
    })
    it('allows no filter at all, not even a removal, when the peer advertised none', () => {
      expect(check([twoRanges], 0n)).toThrow(InvalidFilterError)
      expect(check([new RangeFilter(RangeFilterKind.Priority)], 0n)).toThrow(InvalidFilterError)
    })
    it('counts the ranges of every filter in the request together', () => {
      expect(check([twoRanges, oneRange], 3n)).not.toThrow()
      expect(check([twoRanges, oneRange], 2n)).toThrow(InvalidFilterError)
    })
    it('counts the filters inside FILL_PARAMETERS against a limit of their own', () => {
      expect(check([twoRanges, new FillParameters([twoRanges])], 2n)).not.toThrow()
      expect(check([new FillParameters([twoRanges, oneRange])], 2n)).toThrow(InvalidFilterError)
      expect(check([new FillParameters([oneRange])], 0n)).toThrow(InvalidFilterError)
    })
  })

  describe('rangeFilterRejection', async () => {
    const { Forward, RangeFilter, RangeFilterKind, FillParameters } = await import('../../model/parameter/message')
    const { encodeRanges } = await import('../../model/parameter/message/range_filter')
    const objectId = (setId: number, start: bigint, end: bigint) =>
      new RangeFilter(RangeFilterKind.ObjectId, setId, undefined, encodeRanges([{ start, end }]))

    it('lets a request without range filters through, whatever the limit', () => {
      expect(rangeFilterRejection([new Forward(true)], 0n)).toBeUndefined()
    })
    it('takes filters within the limit', () => {
      expect(rangeFilterRejection([objectId(0, 0n, 0n), objectId(1, 4n, 4n)], 2n)).toBeUndefined()
    })
    it('refuses any filter with a limit of 0, removals and fills included', () => {
      for (const params of [
        [new RangeFilter(RangeFilterKind.Subgroup)],
        [new FillParameters([new RangeFilter(RangeFilterKind.ObjectId)])],
      ]) {
        expect(rangeFilterRejection(params, 0n)?.errorCode).toBe(RequestErrorCode.InvalidFilter)
      }
    })
    it('refuses more ranges than the limit, and a filter that cannot be applied', () => {
      expect(rangeFilterRejection([objectId(0, 0n, 0n), objectId(1, 4n, 4n)], 1n)?.errorCode).toBe(
        RequestErrorCode.InvalidFilter,
      )
      expect(rangeFilterRejection([objectId(0, 0n, 0n), objectId(0, 4n, 4n)], 8n)?.errorCode).toBe(
        RequestErrorCode.InvalidFilter,
      )
      expect(
        rangeFilterRejection([new FillParameters([objectId(0, 0n, 0n), objectId(0, 1n, 1n)])], 8n)?.errorCode,
      ).toBe(RequestErrorCode.InvalidFilter)
    })
  })

  describe('isValidTrackAlias', () => {
    it('accepts zero', () => {
      expect(isValidTrackAlias(0n)).toBe(true)
    })
    it('accepts positive alias', () => {
      expect(isValidTrackAlias(1n)).toBe(true)
      expect(isValidTrackAlias(9999999999n)).toBe(true)
    })
    it('rejects undefined', () => {
      expect(isValidTrackAlias(undefined)).toBe(false)
    })
    it('rejects negative alias', () => {
      expect(isValidTrackAlias(-1n)).toBe(false)
    })
  })
}
