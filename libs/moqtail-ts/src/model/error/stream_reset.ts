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

import { InvalidEnumValue } from './error'

/**
 * @public
 * Codes carried when resetting a request stream or sending STOP_SENDING on one, per
 * draft-18 §3.3.3.
 *
 * A separate registry from {@link (RequestErrorCode:enum)}, which disagrees with it on
 * the same names: `GOING_AWAY` is `0x4` here but `0x6` there.
 */
export enum StreamResetCode {
  InternalError = 0x0,
  Cancelled = 0x1,
  DeliveryTimeout = 0x2,
  SessionClosed = 0x3,
  GoingAway = 0x4,
  TooFarBehind = 0x5,
  UnknownObjectStatus = 0x6,
  ExpiredAuthToken = 0x7,
  ExcessiveLoad = 0x9,
  MalformedTrack = 0x12,
}

export namespace StreamResetCode {
  /**
   * Converts a numeric application error code to a StreamResetCode. The code is a
   * `number` because that is what the transport reports it as
   * (`WebTransportError.streamErrorCode`, an `unsigned long`).
   *
   * @throws :{@link InvalidEnumValue} if the code is not a defined stream reset code.
   */
  export function tryFrom(code: number): StreamResetCode {
    switch (code) {
      case StreamResetCode.InternalError:
        return StreamResetCode.InternalError
      case StreamResetCode.Cancelled:
        return StreamResetCode.Cancelled
      case StreamResetCode.DeliveryTimeout:
        return StreamResetCode.DeliveryTimeout
      case StreamResetCode.SessionClosed:
        return StreamResetCode.SessionClosed
      case StreamResetCode.GoingAway:
        return StreamResetCode.GoingAway
      case StreamResetCode.TooFarBehind:
        return StreamResetCode.TooFarBehind
      case StreamResetCode.UnknownObjectStatus:
        return StreamResetCode.UnknownObjectStatus
      case StreamResetCode.ExpiredAuthToken:
        return StreamResetCode.ExpiredAuthToken
      case StreamResetCode.ExcessiveLoad:
        return StreamResetCode.ExcessiveLoad
      case StreamResetCode.MalformedTrack:
        return StreamResetCode.MalformedTrack
      default:
        throw new InvalidEnumValue('StreamResetCode.tryFrom', code)
    }
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest

  describe('StreamResetCode.tryFrom', () => {
    test('round-trips every enum member', () => {
      for (const [name, code] of Object.entries(StreamResetCode)) {
        if (typeof code !== 'number') continue
        expect(StreamResetCode.tryFrom(code), name).toBe(code)
      }
    })

    test('rejects unassigned codes', () => {
      expect(() => StreamResetCode.tryFrom(0x8)).toThrow(InvalidEnumValue)
      expect(() => StreamResetCode.tryFrom(0x11)).toThrow(InvalidEnumValue)
    })
  })

  describe('draft-18 conformance', () => {
    const fixture = async () => await import('../../../test/conformance')

    test('StreamResetCode matches stream_reset_codes.json', async () => {
      const { streamResetCodes, assertRegistry, pascalIdent } = await fixture()
      assertRegistry(streamResetCodes(), pascalIdent(), (codepoint) => {
        try {
          return StreamResetCode[StreamResetCode.tryFrom(Number(codepoint))]
        } catch {
          return undefined
        }
      })
    })
  })
}
