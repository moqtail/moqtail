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

/**
 * @public
 * Every error the player reports. The first group is fatal (`error` event), the
 * second are warnings (`warning` event).
 */
export type PlayerErrorCode =
  | 'UNSUPPORTED_SOURCE'
  | 'UNSUPPORTED_ORIGIN'
  | 'CATALOG_TIMEOUT'
  | 'SESSION_LOST'
  | 'MEDIA_FAILURE'
  | 'SEEK_FAILED'
  | 'NO_MEDIA_SINK'
  | 'ABORTED'
  // Warnings
  | 'PAD_UNLINKED'
  | 'TRACK_SKIPPED'
  | 'VARIABLE_SKIPPED'
  | 'INVALID_CATALOG_UPDATE'

/**
 * @public
 * Broad failure class, mapped to a `MediaError` code by the web component.
 */
export type PlayerErrorCategory = 'source' | 'network' | 'media' | 'aborted'

// Codes not listed here are `source`.
const DEFAULT_CATEGORY: Partial<Record<PlayerErrorCode, PlayerErrorCategory>> = {
  SESSION_LOST: 'network',
  MEDIA_FAILURE: 'media',
  NO_MEDIA_SINK: 'media',
  ABORTED: 'aborted',
}

/**
 * @public
 * The single error type of the player module. `element` names the element that
 * raised it, when there is one.
 */
export class PlayerError extends Error {
  readonly category: PlayerErrorCategory
  readonly element?: string

  constructor(
    readonly code: PlayerErrorCode,
    message: string,
    options: { category?: PlayerErrorCategory; element?: string; cause?: unknown } = {},
  ) {
    super(message, { cause: options.cause })
    this.name = 'PlayerError'
    this.category = options.category ?? DEFAULT_CATEGORY[code] ?? 'source'
    if (options.element !== undefined) this.element = options.element
  }
}
