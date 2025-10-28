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
import { MaxCacheDuration, DeliveryTimeout } from './version'
import { AuthorizationToken } from './common'

export type VersionSpecificParameter = MaxCacheDuration | DeliveryTimeout | AuthorizationToken
export namespace VersionSpecificParameter {
  export function fromKeyValuePair(pair: KeyValuePair): VersionSpecificParameter | undefined {
    return (
      MaxCacheDuration.fromKeyValuePair(pair) ||
      DeliveryTimeout.fromKeyValuePair(pair) ||
      AuthorizationToken.fromKeyValuePair(pair)
    )
  }
  export function toKeyValuePair(param: VersionSpecificParameter): KeyValuePair {
    return param.toKeyValuePair()
  }
  export function isMaxCacheDuration(param: VersionSpecificParameter): param is MaxCacheDuration {
    return param instanceof MaxCacheDuration
  }
  export function isDeliveryTimeout(param: VersionSpecificParameter): param is DeliveryTimeout {
    return param instanceof DeliveryTimeout
  }
  export function isAuthorizationToken(param: VersionSpecificParameter): param is AuthorizationToken {
    return param instanceof AuthorizationToken
  }
}
export class VersionSpecificParameters {
  private kvps: KeyValuePair[] = []

  addMaxCacheDuration(duration: bigint | number): this {
    this.kvps.push(new MaxCacheDuration(BigInt(duration)).toKeyValuePair())
    return this
  }

  addDeliveryTimeout(timeout: bigint | number): this {
    this.kvps.push(new DeliveryTimeout(BigInt(timeout)).toKeyValuePair())
    return this
  }

  addAuthorizationToken(auth: AuthorizationToken): this {
    this.kvps.push(auth.toKeyValuePair())
    return this
  }

  addRaw(pair: KeyValuePair): this {
    this.kvps.push(pair)
    return this
  }

  build(): KeyValuePair[] {
    return this.kvps
  }

  static fromKeyValuePairs(kvps: KeyValuePair[]): VersionSpecificParameter[] {
    const result: VersionSpecificParameter[] = []
    for (const kvp of kvps) {
      const parsed =
        MaxCacheDuration.fromKeyValuePair(kvp) ||
        DeliveryTimeout.fromKeyValuePair(kvp) ||
        AuthorizationToken.fromKeyValuePair(kvp)
      if (parsed) result.push(parsed)
    }
    return result
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest

  describe('VersionParameters', () => {
    test('build and fromKeyValuePairs returns correct parameters', () => {
      const builder = new VersionSpecificParameters().addMaxCacheDuration(100n).addDeliveryTimeout(55n)
      const kvps = builder.build()
      const parsed = VersionSpecificParameters.fromKeyValuePairs(kvps)
      expect(parsed.length).toBe(2)
      expect(parsed[0] && VersionSpecificParameter.isMaxCacheDuration(parsed[0]) && parsed[0].duration === 100n).toBe(
        true,
      )
      expect(
        parsed[1] && VersionSpecificParameter.isDeliveryTimeout(parsed[1]) && parsed[1].objectTimeout === 55n,
      ).toBe(true)
    })
    test('fromKeyValuePairs skips unknown parameter', () => {
      const unknown = KeyValuePair.tryNewVarInt(998, 1n)
      const kvps = new VersionSpecificParameters().addRaw(unknown).build()
      const parsed = VersionSpecificParameters.fromKeyValuePairs(kvps)
      expect(parsed.length).toBe(0)
    })
  })
}
