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

import { InvalidTypeError } from '../error'

export enum SetupOptionType {
  Path = 0x01,
  AuthorizationToken = 0x03,
  MaxAuthTokenCacheSize = 0x04,
  /** Raw-QUIC only. Client-only; MUST NOT be sent over WebTransport (draft-18 §10.3.1.1). */
  Authority = 0x05,
  MoqtImplementation = 0x07,
}

export function setupOptionTypeFromNumber(value: number): SetupOptionType {
  switch (value) {
    case 0x01:
      return SetupOptionType.Path
    case 0x03:
      return SetupOptionType.AuthorizationToken
    case 0x04:
      return SetupOptionType.MaxAuthTokenCacheSize
    case 0x05:
      return SetupOptionType.Authority
    case 0x07:
      return SetupOptionType.MoqtImplementation
    default:
      throw new InvalidTypeError('setupOptionTypeFromNumber', `Invalid setup option type: ${value}`)
  }
}

export enum MessageParameterType {
  ObjectDeliveryTimeout = 0x02,
  AuthorizationToken = 0x03,
  RendezvousTimeout = 0x04,
  SubgroupDeliveryTimeout = 0x06,
  Expires = 0x08,
  LargestObject = 0x09,
  FillTimeout = 0x0a,
  Forward = 0x10,
  SubscriberPriority = 0x20,
  SubscriptionFilter = 0x21,
  GroupOrder = 0x22,
  FillParameters = 0x23,
  SwitchFrom = 0x24,
  NewGroupRequest = 0x32,
  /**
   * Registered codepoint with no typed parameter yet: its consumer is REQUEST_UPDATE for
   * SUBSCRIBE_TRACKS (TS-11), and the value is a Track Namespace tuple rather than the
   * varint its even Type implies.
   */
  TrackNamespacePrefix = 0x34,
}

export function messageParameterTypeFromNumber(value: bigint | number): MessageParameterType {
  const numValue = Number(value)
  switch (numValue) {
    case 0x02:
      return MessageParameterType.ObjectDeliveryTimeout
    case 0x03:
      return MessageParameterType.AuthorizationToken
    case 0x04:
      return MessageParameterType.RendezvousTimeout
    case 0x06:
      return MessageParameterType.SubgroupDeliveryTimeout
    case 0x08:
      return MessageParameterType.Expires
    case 0x09:
      return MessageParameterType.LargestObject
    case 0x0a:
      return MessageParameterType.FillTimeout
    case 0x10:
      return MessageParameterType.Forward
    case 0x20:
      return MessageParameterType.SubscriberPriority
    case 0x21:
      return MessageParameterType.SubscriptionFilter
    case 0x22:
      return MessageParameterType.GroupOrder
    case 0x23:
      return MessageParameterType.FillParameters
    case 0x24:
      return MessageParameterType.SwitchFrom
    case 0x32:
      return MessageParameterType.NewGroupRequest
    case 0x34:
      return MessageParameterType.TrackNamespacePrefix
    default:
      throw new InvalidTypeError('messageParameterTypeFromNumber', `Unknown message parameter type: ${value}`)
  }
}

export enum TokenAliasType {
  Delete = 0x0,
  Register = 0x1,
  UseAlias = 0x2,
  UseValue = 0x3,
}

export function tokenAliasTypeFromNumber(value: number): TokenAliasType {
  switch (value) {
    case 0x0:
      return TokenAliasType.Delete
    case 0x1:
      return TokenAliasType.Register
    case 0x2:
      return TokenAliasType.UseAlias
    case 0x3:
      return TokenAliasType.UseValue
    default:
      throw new InvalidTypeError('tokenAliasTypeFromNumber', `Invalid token alias type: ${value}`)
  }
}

if (import.meta.vitest) {
  const { describe, test } = import.meta.vitest

  // Asserted against dev/conformance/draft18/, which is shared with moqtail-rs. These
  // enums have no tryFrom, so the lookup uses the enum's own reverse mapping.
  describe('draft-18 conformance', () => {
    const fixture = async () => await import('../../../test/conformance')

    test('SetupOptionType matches parameter_types.json', async () => {
      const { parameterTypes, assertRegistry, pascalIdent } = await fixture()
      assertRegistry(parameterTypes().setup_options, pascalIdent(), (codepoint) => SetupOptionType[Number(codepoint)])
    })

    test('MessageParameterType matches parameter_types.json', async () => {
      const { parameterTypes, assertRegistry, pascalIdent } = await fixture()
      assertRegistry(
        parameterTypes().message_parameters,
        pascalIdent(),
        (codepoint) => MessageParameterType[Number(codepoint)],
      )
    })
  })
}
