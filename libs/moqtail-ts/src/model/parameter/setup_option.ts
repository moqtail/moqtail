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
import { Path, MaxAuthTokenCacheSize, Authority, MoqtImplementation } from './setup'
import { AuthorizationToken } from './common'
import { SetupOptionType, TokenAliasType } from './constant'
import { ProtocolViolationError } from '../error/error'

export type SetupOption = Path | MaxAuthTokenCacheSize | AuthorizationToken | Authority | MoqtImplementation
export namespace SetupOption {
  export function fromKeyValuePair(pair: KeyValuePair): SetupOption | undefined {
    return (
      Path.fromKeyValuePair(pair) ||
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

/** Throws if `params` carries AUTHORITY, which MUST NOT be sent over WebTransport. */
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

  addPath(moqtPath: string): this {
    this.kvps.push(new Path(moqtPath).toKeyValuePair())
    return this
  }

  addAuthorizationToken(auth: AuthorizationToken): this {
    this.kvps.push(auth.toKeyValuePair())
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
        .addMaxAuthTokenCacheSize(123n)
        .addAuthorizationToken(AuthorizationToken.newUseAlias(1n))
        .addRaw(new Authority('example.com').toKeyValuePair())
        .addMoqtImplementation('moqtail-ts/0.1')
        .build()
      const parsed = SetupOptions.fromKeyValuePairs(kvps)
      expect(parsed.length).toBe(5)
      expect(parsed[0] && SetupOption.isPath(parsed[0]) && parsed[0].moqtPath === 'abc').toBe(true)
      expect(parsed[1] && SetupOption.isMaxAuthTokenCacheSize(parsed[1]) && parsed[1].maxSize === 123n).toBe(true)
      expect(
        parsed[2] &&
          SetupOption.isAuthorizationToken(parsed[2]) &&
          parsed[2].variant.aliasType === TokenAliasType.UseAlias,
      ).toBe(true)
      expect(parsed[3] && SetupOption.isAuthority(parsed[3]) && parsed[3].authority === 'example.com').toBe(true)
      expect(parsed[4] && SetupOption.isMoqtImplementation(parsed[4]) && parsed[4].info === 'moqtail-ts/0.1').toBe(true)
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
      const params = new SetupOptions().addPath('/x').addRaw(new Authority('example.com').toKeyValuePair()).build()
      expect(() => assertNoAuthorityOverWebTransport(params)).toThrow(ProtocolViolationError)
    })
    test('does not throw when params carry no AUTHORITY', () => {
      const params = new SetupOptions().addPath('/x').addMaxAuthTokenCacheSize(1n).build()
      expect(() => assertNoAuthorityOverWebTransport(params)).not.toThrow()
    })
    test('does not throw for an empty params list', () => {
      expect(() => assertNoAuthorityOverWebTransport([])).not.toThrow()
    })
  })
}
