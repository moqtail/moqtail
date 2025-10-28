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

import { InvalidTypeError } from '../error'

export enum CommonType {
  AuthorizationToken = 0x03,
}

export function commonTypeFromNumber(value: number): CommonType {
  switch (value) {
    case 0x03:
      return CommonType.AuthorizationToken
    default:
      throw new InvalidTypeError('commonTypeFromNumber', `Invalid common type: ${value}`)
  }
}

export enum SetupParameterType {
  Path = 0x01,
  MaxRequestId = 0x02,
  MaxAuthTokenCacheSize = 0x04,
}

export function setupParameterTypeFromNumber(value: number): SetupParameterType {
  switch (value) {
    case 0x01:
      return SetupParameterType.Path
    case 0x02:
      return SetupParameterType.MaxRequestId
    case 0x04:
      return SetupParameterType.MaxAuthTokenCacheSize
    default:
      throw new InvalidTypeError('setupParameterTypeFromNumber', `Invalid setup parameter type: ${value}`)
  }
}

export enum VersionSpecificParameterType {
  AuthorizationToken = 0x01,
  DeliveryTimeout = 0x02,
  MaxCacheDuration = 0x04,
}

export function versionSpecificParameterTypeFromNumber(value: number): VersionSpecificParameterType {
  switch (value) {
    case 0x01:
      return VersionSpecificParameterType.AuthorizationToken
    case 0x02:
      return VersionSpecificParameterType.DeliveryTimeout
    case 0x04:
      return VersionSpecificParameterType.MaxCacheDuration
    default:
      throw new InvalidTypeError('versionSpecificParameterTypeFromNumber', `Invalid version parameter type: ${value}`)
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
