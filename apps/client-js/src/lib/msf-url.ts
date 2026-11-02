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
 * Thin adapter over `moqtail/player` that keeps the app's slash-joined namespace
 * strings. Goes away when client-js moves to the library player.
 */

import { Tuple } from 'moqtail';
import { formatMsfUrl, parseMsfUrl as parseMsfSource } from 'moqtail/player';

export interface MsfUrlParts {
  relayUrl: string; // moqt://host[:port][/path][?query] -- the scheme actually connected with
  namespace: string; // slash-separated tuple fields, e.g. "moqtail/testsrc"
  trackName?: string;
}

/**
 * Parse a moqt:// MSF URL into its relay, namespace, and track name.
 * Returns null if the input is not a valid MSF URL.
 */
export function parseMsfUrl(url: string): MsfUrlParts | null {
  try {
    const source = parseMsfSource(url);
    const namespace = source.namespace.fields.map(f => f.toUtf8()).join('/');
    return { relayUrl: source.url, namespace, trackName: source.track };
  } catch {
    return null;
  }
}

/**
 * Build a moqt:// MSF URL from relay URL, namespace, and optional track name.
 * Returns an empty string if relayUrl cannot be parsed.
 */
export function buildMsfUrl(relayUrl: string, namespace: string, trackName?: string): string {
  let parsed: URL;
  try {
    parsed = new URL(relayUrl);
  } catch {
    return '';
  }

  const path = parsed.pathname !== '/' ? parsed.pathname : '';
  // An MSF URL always names a track (MSF 11.1.2); the app's entry point is the catalog.
  return formatMsfUrl(
    `moqt://${parsed.host}${path}`,
    Tuple.fromUtf8Path(namespace),
    trackName ?? 'catalog',
  );
}
