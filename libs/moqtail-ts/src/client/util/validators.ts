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

if (import.meta.vitest) {
  const { describe, it, expect } = import.meta.vitest
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
