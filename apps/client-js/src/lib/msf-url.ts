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
 * MSF URL construction and interpretation per draft-ietf-moq-msf §11.1.
 *
 * URL shape:
 *   moqt://host[:port][/path]#msf:<ns-encoded>[--<track-encoded>]
 *
 * §11.1.2 field encoding:
 *   - [a-zA-Z0-9_] → literal
 *   - everything else → .XX  (period + 2 lowercase hex digits)
 *   - namespace tuple fields joined by single  -
 *   - namespace / track-name boundary is  --
 */

export interface MsfUrlParts {
  relayUrl: string; // https://host[:port][/path]
  namespace: string; // slash-separated tuple fields, e.g. "moqtail/testsrc"
  trackName?: string;
}

function encodeMsfField(s: string): string {
  const bytes = new TextEncoder().encode(s);
  let out = '';
  for (const b of bytes) {
    const c = String.fromCharCode(b);
    if (/[a-zA-Z0-9_]/.test(c)) {
      out += c;
    } else {
      out += '.' + b.toString(16).padStart(2, '0');
    }
  }
  return out;
}

function decodeMsfField(s: string): string {
  const bytes: number[] = [];
  let i = 0;
  while (i < s.length) {
    if (s[i] === '.' && i + 2 < s.length) {
      bytes.push(parseInt(s.slice(i + 1, i + 3), 16));
      i += 3;
    } else {
      bytes.push(s.charCodeAt(i));
      i += 1;
    }
  }
  return new TextDecoder().decode(new Uint8Array(bytes));
}

/**
 * Parse a moqt:// MSF URL into its relay, namespace, and optional track name.
 * Returns null if the input is not a valid MSF URL.
 */
export function parseMsfUrl(url: string): MsfUrlParts | null {
  let parsed: URL;
  try {
    parsed = new URL(url);
  } catch {
    return null;
  }

  if (parsed.protocol !== 'moqt:') return null;

  const fragment = parsed.hash.slice(1); // strip leading #
  if (!fragment.startsWith('msf:')) return null;

  // Ignore any &parameters per §11.1.1 — only extract the track identifier.
  const trackId = fragment.slice(4).split('&')[0];

  const sepIdx = trackId.indexOf('--');
  const nsEncoded = sepIdx === -1 ? trackId : trackId.slice(0, sepIdx);
  const trackEncoded = sepIdx === -1 ? undefined : trackId.slice(sepIdx + 2);

  // Single bare hyphens are field separators; encoded hyphens appear as .2d.
  const namespace = nsEncoded.split('-').map(decodeMsfField).join('/');

  const trackName = trackEncoded !== undefined ? decodeMsfField(trackEncoded) : undefined;

  const path = parsed.pathname !== '/' ? parsed.pathname : '';
  const relayUrl = `https://${parsed.host}${path}`;

  return { relayUrl, namespace, trackName };
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
  const nsEncoded = namespace.split('/').filter(Boolean).map(encodeMsfField).join('-');

  const trackPart = trackName !== undefined ? `--${encodeMsfField(trackName)}` : '';

  return `moqt://${parsed.host}${path}#msf:${nsEncoded}${trackPart}`;
}
