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

// Names of the REQUEST_ERROR codes, as in libs/moqtail-rs/src/model/error/request_error.rs.

const REQUEST_ERRORS: Record<number, string> = {
  0x0: 'INTERNAL_ERROR',
  0x1: 'UNAUTHORIZED',
  0x2: 'TIMEOUT',
  0x3: 'NOT_SUPPORTED',
  0x4: 'MALFORMED_AUTH_TOKEN',
  0x5: 'EXPIRED_AUTH_TOKEN',
  0x6: 'GOING_AWAY',
  0x9: 'EXCESSIVE_LOAD',
  0x10: 'DOES_NOT_EXIST',
  0x11: 'INVALID_RANGE',
  0x12: 'MALFORMED_TRACK',
  0x20: 'UNINTERESTED',
  0x30: 'PREFIX_OVERLAP',
  0x31: 'NAMESPACE_TOO_LARGE',
  0x32: 'INVALID_SWITCH',
  0x33: 'UNSUPPORTED_EXTENSION',
  0x34: 'REDIRECT',
  0x35: 'CONFLICTING_FILTERS',
  0x36: 'INVALID_FILTER',
};

/** A REQUEST_ERROR code by name, such as `DOES_NOT_EXIST (0x10)`. */
export function requestError(code: number | string): string {
  const n = Number(code);
  const hex = `0x${n.toString(16)}`;
  return REQUEST_ERRORS[n] ? `${REQUEST_ERRORS[n]} (${hex})` : `code ${hex}`;
}
