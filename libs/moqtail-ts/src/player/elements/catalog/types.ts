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

/** Catalog types, derived from the schemas so they can't drift from the validation. */

import type * as v from 'valibot'
import type { catalogSchema } from './schema'

export type { InitData } from './schema'

type CatalogSchema = ReturnType<typeof catalogSchema>

/** @public A catalog as a publisher writes it (MSF 5.1). */
export type CatalogJson = v.InferInput<CatalogSchema>

/** @public A parsed, validated catalog. */
export type Catalog = v.InferOutput<CatalogSchema> & {
  /** The document after variable substitution, which delta updates apply to (MSF 5.3). */
  json: CatalogDocument
}

/**
 * @public
 * A parsed catalog track. Field names follow MSF 5.2 and CMSF 3.5.2, unknown fields are kept.
 * `nameBytes` and `namespace` are the MOQT bytes (MSF 11.2), the namespace inherited from the
 * catalog track when the track has none, and `init` is resolved from `initRef`.
 */
export type CatalogTrack = Catalog['tracks'][number]

/** @public The media timeline template of MSF 7.4.1, with named fields. */
export type MediaTimelineTemplate = NonNullable<CatalogTrack['template']>

/** A valid catalog document, as JSON. */
export interface CatalogDocument {
  tracks: Record<string, unknown>[]
  [field: string]: unknown
}
