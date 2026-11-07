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

import type { ObjectOrdering } from '@/client/types'

/** Subscriber priority of the catalog: above every other subscription (MSF 5). */
export const CATALOG_PRIORITY = 0

/** Subscriber priority of media and timeline tracks: below the catalog (MSF 5); 128 is the moqt-22 default. */
export const MEDIA_PRIORITY = 128

/** Default limit for a decompressed catalog or timeline payload (`opts.maxCatalogBytes`). */
export const DEFAULT_MAX_CATALOG_BYTES = 16 * 1024 * 1024

/** Default `opts.client.reconnectAttempts`. */
export const DEFAULT_RECONNECT_ATTEMPTS = 5

/** Default wait for the first catalog or timeline object (`opts.catalogTimeoutMs`). */
export const DEFAULT_CATALOG_TIMEOUT_MS = 10_000

/** Media ordering without `opts.objectOrdering`: demo-abr's pool TTL. */
export const DEFAULT_MEDIA_ORDERING: ObjectOrdering = { window: { durationMs: 150 } }
