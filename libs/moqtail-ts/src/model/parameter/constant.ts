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

export enum SetupParameterType {
  Path = 0x01,
  MaxRequestId = 0x02,
  AuthorizationToken = 0x03,
  MaxAuthTokenCacheSize = 0x04,
}

export function setupParameterTypeFromNumber(value: number): SetupParameterType {
  switch (value) {
    case 0x01:
      return SetupParameterType.Path
    case 0x02:
      return SetupParameterType.MaxRequestId
    case 0x03:
      return SetupParameterType.AuthorizationToken
    case 0x04:
      return SetupParameterType.MaxAuthTokenCacheSize
    default:
      throw new InvalidTypeError('setupParameterTypeFromNumber', `Invalid setup parameter type: ${value}`)
  }
}

export enum MessageParameterType {
  DeliveryTimeout = 0x02,
  AuthorizationToken = 0x03,
  Expires = 0x08,
  LargestObject = 0x09,
  Forward = 0x10,
  SubscriberPriority = 0x20,
  SubscriptionFilter = 0x21,
  GroupOrder = 0x22,
  NewGroupRequest = 0x32,
}

export function messageParameterTypeFromNumber(value: bigint | number): MessageParameterType {
  const numValue = Number(value)
  switch (numValue) {
    case 0x02:
      return MessageParameterType.DeliveryTimeout
    case 0x03:
      return MessageParameterType.AuthorizationToken
    case 0x08:
      return MessageParameterType.Expires
    case 0x09:
      return MessageParameterType.LargestObject
    case 0x10:
      return MessageParameterType.Forward
    case 0x20:
      return MessageParameterType.SubscriberPriority
    case 0x21:
      return MessageParameterType.SubscriptionFilter
    case 0x22:
      return MessageParameterType.GroupOrder
    case 0x32:
      return MessageParameterType.NewGroupRequest
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
