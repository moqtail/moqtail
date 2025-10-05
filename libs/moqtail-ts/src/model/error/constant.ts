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

import { InvalidTypeError } from './error'

export enum TerminationCode {
  NO_ERROR = 0x0,
  INTERNAL_ERROR = 0x1,
  UNAUTHORIZED = 0x2,
  PROTOCOL_VIOLATION = 0x3,
  INVALID_REQUEST_ID = 0x4,
  DUPLICATE_TRACK_ALIAS = 0x5,
  KEY_VALUE_FORMATTING_ERROR = 0x6,
  TOO_MANY_REQUESTS = 0x7,
  INVALID_PATH = 0x8,
  MALFORMED_PATH = 0x9,
  GOAWAY_TIMEOUT = 0x10,
  CONTROL_MESSAGE_TIMEOUT = 0x11,
  DATA_STREAM_TIMEOUT = 0x12,
  AUTH_TOKEN_CACHE_OVERFLOW = 0x13,
  DUPLICATE_AUTH_TOKEN_ALIAS = 0x14,
  VERSION_NEGOTIATION_FAILED = 0x15,
  MALFORMED_AUTH_TOKEN = 0x16,
  UNKNOWN_AUTH_TOKEN_ALIAS = 0x17,
  EXPIRED_AUTH_TOKEN = 0x18,
  INVALID_AUTHORITY = 0x19,
  MALFORMED_AUTHORITY = 0x1a,
}

export namespace TerminationCode {
  /**
   * Tries to convert a number to a TerminationCode enum member.
   *
   * @param code - The numeric code to convert.
   * @returns The corresponding TerminationCode member, or throws InvalidTypeError if the code is not valid.
   */
  export function tryFrom(code: number): TerminationCode {
    switch (code) {
      case TerminationCode.NO_ERROR:
        return TerminationCode.NO_ERROR
      case TerminationCode.INTERNAL_ERROR:
        return TerminationCode.INTERNAL_ERROR
      case TerminationCode.UNAUTHORIZED:
        return TerminationCode.UNAUTHORIZED
      case TerminationCode.PROTOCOL_VIOLATION:
        return TerminationCode.PROTOCOL_VIOLATION
      case TerminationCode.INVALID_REQUEST_ID:
        return TerminationCode.INVALID_REQUEST_ID
      case TerminationCode.DUPLICATE_TRACK_ALIAS:
        return TerminationCode.DUPLICATE_TRACK_ALIAS
      case TerminationCode.KEY_VALUE_FORMATTING_ERROR:
        return TerminationCode.KEY_VALUE_FORMATTING_ERROR
      case TerminationCode.TOO_MANY_REQUESTS:
        return TerminationCode.TOO_MANY_REQUESTS
      case TerminationCode.INVALID_PATH:
        return TerminationCode.INVALID_PATH
      case TerminationCode.MALFORMED_PATH:
        return TerminationCode.MALFORMED_PATH
      case TerminationCode.GOAWAY_TIMEOUT:
        return TerminationCode.GOAWAY_TIMEOUT
      case TerminationCode.CONTROL_MESSAGE_TIMEOUT:
        return TerminationCode.CONTROL_MESSAGE_TIMEOUT
      case TerminationCode.DATA_STREAM_TIMEOUT:
        return TerminationCode.DATA_STREAM_TIMEOUT
      case TerminationCode.AUTH_TOKEN_CACHE_OVERFLOW:
        return TerminationCode.AUTH_TOKEN_CACHE_OVERFLOW
      case TerminationCode.DUPLICATE_AUTH_TOKEN_ALIAS:
        return TerminationCode.DUPLICATE_AUTH_TOKEN_ALIAS
      case TerminationCode.VERSION_NEGOTIATION_FAILED:
        return TerminationCode.VERSION_NEGOTIATION_FAILED
      default:
        throw new InvalidTypeError('TerminationCode.tryFrom', `Unknown termination code: ${code}`)
    }
  }
}
