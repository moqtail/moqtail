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

/** Returns true when `trackAlias` is a valid (non-negative) track alias. Zero is a legal value. */
export function isValidTrackAlias(trackAlias: bigint | undefined): trackAlias is bigint {
  return trackAlias !== undefined && trackAlias >= 0n
}

/**
 * The REQUEST_ERROR for a request that carries a Range Filter. This client never
 * advertises MAX_FILTER_RANGES, so the peer is allowed none, and any it sends —
 * a removal or one tucked inside FILL_PARAMETERS included — is INVALID_FILTER.
 */
export function rangeFilterRejection(params: readonly MessageParameter[]): RequestError | undefined {
  if (!MessageParameter.carriesRangeFilter(params)) return undefined
  return new RequestError(RequestErrorCode.InvalidFilter, 0n, new ReasonPhrase('Range filters are not supported'))
}

/**
 * Throws unless `params` stay within the peer's MAX_FILTER_RANGES, so a request the
 * peer would have to refuse is never sent.
 *
 * A peer that advertised none allows no Range Filter parameter at all, not even a
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
    if (limit === 0n) throw new InvalidFilterError(context, 'the peer allows no range filters (MAX_FILTER_RANGES is 0)')
    const count = BigInt(filters.reduce((sum, f) => sum + f.rangeCount, 0))
    if (count > limit) {
      throw new InvalidFilterError(context, `${count} filter ranges exceed the peer's MAX_FILTER_RANGES of ${limit}`)
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
    it('lets a request without range filters through', () => {
      expect(rangeFilterRejection([new Forward(true)])).toBeUndefined()
    })
    it('rejects any range filter with INVALID_FILTER', () => {
      for (const params of [
        [new RangeFilter(RangeFilterKind.Subgroup)],
        [new FillParameters([new RangeFilter(RangeFilterKind.ObjectId)])],
      ]) {
        expect(rangeFilterRejection(params)?.errorCode).toBe(RequestErrorCode.InvalidFilter)
      }
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
