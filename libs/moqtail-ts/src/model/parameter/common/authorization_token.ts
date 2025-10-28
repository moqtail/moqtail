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

import { KeyValuePair } from '../../common/pair'
import { TokenAliasType, tokenAliasTypeFromNumber, CommonType } from '../constant'
import { Parameter } from '../parameter'
import { ByteBuffer, FrozenByteBuffer } from '../../common/byte_buffer'

export type AuthTokenVariant =
  | { aliasType: TokenAliasType.Delete; tokenAlias: bigint }
  | { aliasType: TokenAliasType.Register; tokenAlias: bigint; tokenType: bigint; tokenValue: Uint8Array }
  | { aliasType: TokenAliasType.UseAlias; tokenAlias: bigint }
  | { aliasType: TokenAliasType.UseValue; tokenType: bigint; tokenValue: Uint8Array }

export class AuthorizationToken implements Parameter {
  static readonly TYPE = CommonType.AuthorizationToken
  private constructor(public readonly variant: AuthTokenVariant) {}

  static newDelete(tokenAlias: bigint | number): AuthorizationToken {
    return new AuthorizationToken({
      aliasType: TokenAliasType.Delete,
      tokenAlias: BigInt(tokenAlias),
    })
  }

  static newRegister(
    tokenAlias: bigint | number,
    tokenType: bigint | number,
    tokenValue: Uint8Array,
  ): AuthorizationToken {
    return new AuthorizationToken({
      aliasType: TokenAliasType.Register,
      tokenAlias: BigInt(tokenAlias),
      tokenType: BigInt(tokenType),
      tokenValue,
    })
  }

  static newUseAlias(tokenAlias: bigint | number): AuthorizationToken {
    return new AuthorizationToken({
      aliasType: TokenAliasType.UseAlias,
      tokenAlias: BigInt(tokenAlias),
    })
  }

  static newUseValue(tokenType: bigint | number, tokenValue: Uint8Array): AuthorizationToken {
    return new AuthorizationToken({
      aliasType: TokenAliasType.UseValue,
      tokenType: BigInt(tokenType),
      tokenValue,
    })
  }

  toKeyValuePair(): KeyValuePair {
    const payload = new ByteBuffer()
    payload.putVI(this.variant.aliasType)
    switch (this.variant.aliasType) {
      case TokenAliasType.Delete:
        payload.putVI(this.variant.tokenAlias)
        break
      case TokenAliasType.Register:
        payload.putVI(this.variant.tokenAlias)
        payload.putVI(this.variant.tokenType)
        payload.putBytes(this.variant.tokenValue)
        break
      case TokenAliasType.UseAlias:
        payload.putVI(this.variant.tokenAlias)
        break
      case TokenAliasType.UseValue:
        payload.putVI(this.variant.tokenType)
        payload.putBytes(this.variant.tokenValue)
        break
    }
    return KeyValuePair.tryNewBytes(AuthorizationToken.TYPE, payload.toUint8Array())
  }

  static fromKeyValuePair(pair: KeyValuePair): AuthorizationToken | undefined {
    if (Number(pair.typeValue) !== AuthorizationToken.TYPE || !(pair.value instanceof Uint8Array)) return undefined
    const frozen = new FrozenByteBuffer(pair.value)
    const aliasTypeRaw = frozen.getNumberVI()
    const aliasType = tokenAliasTypeFromNumber(aliasTypeRaw)
    switch (aliasType) {
      case TokenAliasType.Delete: {
        const tokenAlias = frozen.getVI()
        return new AuthorizationToken({ aliasType, tokenAlias })
      }
      case TokenAliasType.Register: {
        const tokenAlias = frozen.getVI()
        const tokenType = frozen.getVI()
        const tokenValue = frozen.getBytes(frozen.remaining)
        return new AuthorizationToken({ aliasType, tokenAlias, tokenType, tokenValue })
      }
      case TokenAliasType.UseAlias: {
        const tokenAlias = frozen.getVI()
        return new AuthorizationToken({ aliasType, tokenAlias })
      }
      case TokenAliasType.UseValue: {
        const tokenType = frozen.getVI()
        const tokenValue = frozen.getBytes(frozen.remaining)
        return new AuthorizationToken({ aliasType, tokenType, tokenValue })
      }
    }
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest

  describe('AuthorizationToken', () => {
    test('fromKeyValuePair returns instance for valid pair', () => {
      const pair = AuthorizationToken.newDelete(54n).toKeyValuePair()
      const param = AuthorizationToken.fromKeyValuePair(pair)
      expect(param).toBeInstanceOf(AuthorizationToken)
      expect(param?.variant.aliasType).toBe(TokenAliasType.Delete)
      if (param?.variant.aliasType === TokenAliasType.Delete) {
        expect(param.variant.tokenAlias).toBe(54n)
      }
    })
    test('fromKeyValuePair returns undefined for wrong type', () => {
      const pair = KeyValuePair.tryNewVarInt(2, 42n)
      const param = AuthorizationToken.fromKeyValuePair(pair)
      expect(param).toBeUndefined()
    })
  })
}
