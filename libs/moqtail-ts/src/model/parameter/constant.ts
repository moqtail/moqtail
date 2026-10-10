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
  /**
   * §10.3.1.6. How many Ranges the peer may send across all Range Filter parameters of
   * one subscription or fetch. The default is 0, and 0 prohibits Range Filters outright
   * rather than meaning unlimited.
   */
  MaxFilterRanges = 0x06,
  MoqtImplementation = 0x07,
  /**
   * §10.3.1.7. How many REQUEST_UPDATEs the peer may leave unanswered on one request
   * stream. 0 -- the default -- means no limit, the opposite of `MaxFilterRanges`'s 0.
   */
  MaxRequestUpdates = 0x08,
  /**
   * PROVISIONAL: sender-side track switching is not an adopted extension, so this
   * codepoint is absent from the registry the conformance fixture tracks. It sits in the
   * 0x07-0x7F range of this space, which that registry leaves free.
   */
  SstsAlgorithms = 0x09,
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
    case 0x06:
      return SetupOptionType.MaxFilterRanges
    case 0x07:
      return SetupOptionType.MoqtImplementation
    case 0x08:
      return SetupOptionType.MaxRequestUpdates
    case 0x09:
      return SetupOptionType.SstsAlgorithms
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
  LocationFilter = 0x21,
  GroupOrder = 0x22,
  FillParameters = 0x23,
  SwitchFrom = 0x24,
  /**
   * The five Range Filters (§5.1.4). 0x26 and 0x28 are length-prefixed despite being
   * even, which §1.4.3's parity rule says they cannot be; §5.1.4's own figures give both
   * an explicit Length. The figures win here because they are what a peer puts on the
   * wire, and `valueShapeOf` carries the exception.
   */
  SubgroupFilter = 0x25,
  ObjectIdFilter = 0x26,
  PriorityFilter = 0x27,
  ObjectPropertyFilter = 0x28,
  TrackPropertyFilter = 0x29,
  NewGroupRequest = 0x32,
  /**
   * Registered codepoint with no typed parameter yet: its consumer is REQUEST_UPDATE for
   * SUBSCRIBE_TRACKS (TS-11), and the value is a Track Namespace tuple rather than the
   * varint its even Type implies.
   */
  TrackNamespacePrefix = 0x34,
  /**
   * Whether the response carries Track Properties (§10.2.21). A uint8, so the odd
   * Type's Length is exactly 1 -- any other length is a KEY_VALUE_FORMATTING_ERROR,
   * while a length-1 value outside 0..1 is a PROTOCOL_VIOLATION.
   */
  IncludeProperties = 0x35,
  /**
   * PROVISIONAL: sender-side track switching is not an adopted extension, so this
   * codepoint is absent from the registry the conformance fixture tracks; see its
   * `local_extensions`.
   */
  SwitchingSetAssignment = 0x41,
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
      return MessageParameterType.LocationFilter
    case 0x22:
      return MessageParameterType.GroupOrder
    case 0x23:
      return MessageParameterType.FillParameters
    case 0x24:
      return MessageParameterType.SwitchFrom
    case 0x25:
      return MessageParameterType.SubgroupFilter
    case 0x26:
      return MessageParameterType.ObjectIdFilter
    case 0x27:
      return MessageParameterType.PriorityFilter
    case 0x28:
      return MessageParameterType.ObjectPropertyFilter
    case 0x29:
      return MessageParameterType.TrackPropertyFilter
    case 0x32:
      return MessageParameterType.NewGroupRequest
    case 0x34:
      return MessageParameterType.TrackNamespacePrefix
    case 0x35:
      return MessageParameterType.IncludeProperties
    case 0x41:
      return MessageParameterType.SwitchingSetAssignment
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

  // Asserted against dev/conformance/draft20/, which is shared with moqtail-rs.
  //
  // The lookup goes through the *FromNumber functions, not the enums' own reverse
  // mappings, because the reverse mapping answers from the enum alone: a member
  // declared and then left out of its switch would read as conforming here while
  // rejecting that codepoint on the wire. moqtail-rs asserts through `try_from` for
  // the same reason.
  describe('draft-20 conformance', () => {
    const fixture = async () => await import('../../../test/conformance')
    const nameVia =
      <T extends number>(from: (value: number) => T, names: Record<number, string>) =>
      (codepoint: bigint): string | undefined => {
        try {
          return names[from(Number(codepoint))]
        } catch {
          return undefined
        }
      }

    test('SetupOptionType matches parameter_types.json', async () => {
      const { parameterTypes, assertRegistry, pascalIdent } = await fixture()
      assertRegistry(parameterTypes().setup_options, pascalIdent(), nameVia(setupOptionTypeFromNumber, SetupOptionType))
    })

    test('MessageParameterType matches parameter_types.json', async () => {
      const { parameterTypes, assertRegistry, pascalIdent } = await fixture()
      // The draft writes OBJECTID_FILTER as one word; this package spells that id
      // `ObjectId` everywhere else, so the identifier keeps the package's spelling.
      assertRegistry(
        parameterTypes().message_parameters,
        pascalIdent({ OBJECTID_FILTER: 'ObjectIdFilter' }),
        nameVia(messageParameterTypeFromNumber, MessageParameterType),
      )
    })
  })
}
