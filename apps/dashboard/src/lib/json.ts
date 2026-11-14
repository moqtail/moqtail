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

// Track aliases and hop ids are 64-bit and often past 2^53, where a JavaScript
// number rounds them. They are kept as the digits the relay sent instead.

type Reviver = (key: string, value: unknown, context?: { source?: string }) => unknown;

const keepLargeIntegers: Reviver = (_key, value, context) =>
  typeof value === 'number' &&
  !Number.isSafeInteger(value) &&
  context?.source !== undefined &&
  /^-?\d+$/.test(context.source)
    ? context.source
    : value;

/** `JSON.parse`, but an integer too large for a number stays a string of its digits.
 *  shortcut: browsers without JSON.parse source access (Safari before 18.4) still
 *  round; parse the text by hand if those matter. */
export function parseJson(text: string): unknown {
  return JSON.parse(text, keepLargeIntegers as Parameters<typeof JSON.parse>[1]);
}
