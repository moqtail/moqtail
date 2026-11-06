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

/** The test content on relay.moqtail.dev. Adding a source is one line at the bottom. */

import { Tuple } from '@/model/common/tuple'
import { formatMsfUrl } from '@/player/msf_url'

export const RELAY = 'moqt://relay.moqtail.dev'

export interface TestSource {
  namespace: string
  live: boolean
  /** Seconds per video group. Undefined when the tracks mix GoP lengths. */
  gop?: number
  /** The catalog URI, with extra fragment parameters: `ch07_vod.uri('location-range=2-4')`. */
  uri(...params: string[]): string
}

const registry = new Map<string, TestSource>()

function source(namespace: string, options: { live: boolean; gop?: number }): TestSource {
  const catalog = formatMsfUrl(RELAY, Tuple.fromUtf8Path(namespace), 'catalog')
  const entry = { namespace, ...options, uri: (...params: string[]) => [catalog, ...params].join('&') }
  registry.set(namespace, entry)
  return entry
}

/** The source a URI's namespace names, if it's one of these. */
export const sourceOf = (namespace: string): TestSource | undefined => registry.get(namespace)

/** 2 s GoP, 1080p, 720p, and 540p, and audio. */
export const ch07 = source('moqtail/ch07', { live: true, gop: 2 })
/** 500 ms, 2 s, and 6 s GoPs of each `ch07` quality, and audio. */
export const ch07_all = source('moqtail/ch07_all', { live: true })
/** `ch07` as VOD. */
export const ch07_vod = source('moqtail/ch07_vod', { live: false, gop: 2 })
