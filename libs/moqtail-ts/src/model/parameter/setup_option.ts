/**
 * Copyright 2025 The MOQtail Authors
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

import { KeyValuePair } from '../common/pair'
import { MaxRequestId } from './setup/max_request_id'
import { Path, MaxAuthTokenCacheSize, Authority, MoqtImplementation } from './setup'
import { AuthorizationToken } from './common'
import { SetupOptionType, TokenAliasType } from './constant'
import { ProtocolViolationError } from '../error/error'

export type SetupOption =
  Path | MaxRequestId | MaxAuthTokenCacheSize | AuthorizationToken | Authority | MoqtImplementation
export namespace SetupOption {
  export function fromKeyValuePair(pair: KeyValuePair): SetupOption | undefined {
    return (
      Path.fromKeyValuePair(pair) ||
      MaxRequestId.fromKeyValuePair(pair) ||
      MaxAuthTokenCacheSize.fromKeyValuePair(pair) ||
      AuthorizationToken.fromKeyValuePair(pair) ||
      Authority.fromKeyValuePair(pair) ||
      MoqtImplementation.fromKeyValuePair(pair)
    )
  }
  export function toKeyValuePair(param: SetupOption): KeyValuePair {
    return param.toKeyValuePair()
  }
  export function isPath(param: SetupOption): param is Path {
    return param instanceof Path
  }
  export function isMaxRequestId(param: SetupOption): param is MaxRequestId {
    return param instanceof MaxRequestId
  }
  export function isMaxAuthTokenCacheSize(param: SetupOption): param is MaxAuthTokenCacheSize {
    return param instanceof MaxAuthTokenCacheSize
  }
  export function isAuthorizationToken(param: SetupOption): param is AuthorizationToken {
    return param instanceof AuthorizationToken
  }
  export function isAuthority(param: SetupOption): param is Authority {
    return param instanceof Authority
  }
  export function isMoqtImplementation(param: SetupOption): param is MoqtImplementation {
    return param instanceof MoqtImplementation
  }
}

/**
 * Throws if `params` carries AUTHORITY, which MUST NOT be sent over WebTransport
 * (draft-18 §10.3.1.1). moqtail-ts always connects over WebTransport — there is no
 * native-QUIC transport to make this conditional on — so `MOQtailClient.new` calls this
 * on the built SetupOptions before sending its Setup.
 */
export function assertNoAuthorityOverWebTransport(params: KeyValuePair[]): void {
  if (params.some((p) => Number(p.typeValue) === SetupOptionType.Authority)) {
    throw new ProtocolViolationError(
      'assertNoAuthorityOverWebTransport',
      'AUTHORITY setup option MUST NOT be sent over WebTransport (draft-18 §10.3.1.1)',
    )
  }
}

export class SetupOptions {
  private kvps: KeyValuePair[] = []

  addMaxAuthTokenCacheSize(maxSize: bigint | number): this {
    this.kvps.push(new MaxAuthTokenCacheSize(BigInt(maxSize)).toKeyValuePair())
    return this
  }

  addMaxRequestId(maxId: bigint | number): this {
    this.kvps.push(new MaxRequestId(BigInt(maxId)).toKeyValuePair())
    return this
  }

  addPath(moqtPath: string): this {
    this.kvps.push(new Path(moqtPath).toKeyValuePair())
    return this
  }

  addAuthorizationToken(auth: AuthorizationToken): this {
    this.kvps.push(auth.toKeyValuePair())
    return this
  }

  /**
   * Raw-QUIC only. MUST NOT be sent over WebTransport (draft-18 §10.3.1.1) — moqtail-ts
   * always connects over WebTransport, so `MOQtailClient.new` rejects a SetupOptions
   * that carries this.
   */
  addAuthority(authority: string): this {
    this.kvps.push(new Authority(authority).toKeyValuePair())
    return this
  }

  addMoqtImplementation(info: string): this {
    this.kvps.push(new MoqtImplementation(info).toKeyValuePair())
    return this
  }

  addRaw(pair: KeyValuePair): this {
    this.kvps.push(pair)
    return this
  }

  build(): KeyValuePair[] {
    return this.kvps
  }

  static fromKeyValuePairs(kvps: KeyValuePair[]): SetupOption[] {
    const result: SetupOption[] = []
    for (const kvp of kvps) {
      const parsed =
        Path.fromKeyValuePair(kvp) ||
        MaxRequestId.fromKeyValuePair(kvp) ||
        MaxAuthTokenCacheSize.fromKeyValuePair(kvp) ||
        AuthorizationToken.fromKeyValuePair(kvp) ||
        Authority.fromKeyValuePair(kvp) ||
        MoqtImplementation.fromKeyValuePair(kvp)
      if (parsed) result.push(parsed)
    }
    return result
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest

  describe('SetupOptions', () => {
    test('build and fromKeyValuePairs returns correct parameters', () => {
      const kvps = new SetupOptions()
        .addPath('abc')
        .addMaxRequestId(42n)
        .addMaxAuthTokenCacheSize(123n)
        .addAuthorizationToken(AuthorizationToken.newUseAlias(1n))
        .addAuthority('example.com')
        .addMoqtImplementation('moqtail-ts/0.1')
        .build()
      const parsed = SetupOptions.fromKeyValuePairs(kvps)
      expect(parsed.length).toBe(6)
      expect(parsed[0] && SetupOption.isPath(parsed[0]) && parsed[0].moqtPath === 'abc').toBe(true)
      expect(parsed[1] && SetupOption.isMaxRequestId(parsed[1]) && parsed[1].maxId === 42n).toBe(true)
      expect(parsed[2] && SetupOption.isMaxAuthTokenCacheSize(parsed[2]) && parsed[2].maxSize === 123n).toBe(true)
      expect(
        parsed[3] &&
          SetupOption.isAuthorizationToken(parsed[3]) &&
          parsed[3].variant.aliasType === TokenAliasType.UseAlias,
      ).toBe(true)
      expect(parsed[4] && SetupOption.isAuthority(parsed[4]) && parsed[4].authority === 'example.com').toBe(true)
      expect(parsed[5] && SetupOption.isMoqtImplementation(parsed[5]) && parsed[5].info === 'moqtail-ts/0.1').toBe(true)
    })
    test('fromKeyValuePairs skips unknown parameter', () => {
      const unknown = KeyValuePair.tryNewVarInt(998, 1n)
      const kvps = new SetupOptions().addRaw(unknown).addPath('wololoo').build()
      const parsed = SetupOptions.fromKeyValuePairs(kvps)
      expect(parsed.length).toBe(1)
      expect(parsed[0] && SetupOption.isPath(parsed[0]) && parsed[0].moqtPath === 'wololoo').toBe(true)
    })
  })

  describe('assertNoAuthorityOverWebTransport', () => {
    test('throws when params carry AUTHORITY', () => {
      const params = new SetupOptions().addPath('/x').addAuthority('example.com').build()
      expect(() => assertNoAuthorityOverWebTransport(params)).toThrow(ProtocolViolationError)
    })
    test('does not throw when params carry no AUTHORITY', () => {
      const params = new SetupOptions().addPath('/x').addMaxRequestId(1n).build()
      expect(() => assertNoAuthorityOverWebTransport(params)).not.toThrow()
    })
    test('does not throw for an empty params list', () => {
      expect(() => assertNoAuthorityOverWebTransport([])).not.toThrow()
    })
  })
}
