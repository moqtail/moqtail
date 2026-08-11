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

import { KeyValuePair } from '../../common/pair'
import { SetupOptionType } from '../constant'
import { Parameter } from '../parameter'

/**
 * Raw-QUIC only: the authority component of the `moqt://` URI, carried in SETUP because
 * there is no HTTP CONNECT to carry it (draft-18 §10.3.1.1).
 *
 * Client-only; MUST NOT be sent by a server or over WebTransport. moqtail-ts always
 * connects over WebTransport, so its own handshake never sends this — see
 * `MOQtailClient.new`.
 */
export class Authority implements Parameter {
  static readonly TYPE = SetupOptionType.Authority
  constructor(public readonly authority: string) {}

  toKeyValuePair(): KeyValuePair {
    const bytes = new TextEncoder().encode(this.authority)
    return KeyValuePair.tryNewBytes(Authority.TYPE, bytes)
  }

  static fromKeyValuePair(pair: KeyValuePair): Authority | undefined {
    if (Number(pair.typeValue) !== Authority.TYPE || !(pair.value instanceof Uint8Array)) return undefined
    const authority = new TextDecoder().decode(pair.value)
    return new Authority(authority)
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest

  describe('Authority', () => {
    test('fromKeyValuePair returns instance for valid pair', () => {
      const pair = new Authority('example.com').toKeyValuePair()
      const param = Authority.fromKeyValuePair(pair)
      expect(param).toBeInstanceOf(Authority)
      expect(param?.authority).toBe('example.com')
    })
    test('fromKeyValuePair returns undefined for wrong type', () => {
      const pair = KeyValuePair.tryNewVarInt(SetupOptionType.MaxAuthTokenCacheSize, 1n)
      const param = Authority.fromKeyValuePair(pair)
      expect(param).toBeUndefined()
    })
  })
}
