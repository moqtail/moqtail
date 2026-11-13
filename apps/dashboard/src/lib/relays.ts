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

// The relays the dashboard knows, kept in the browser. Tokens are kept for the
// browser session only, unless the user asks to remember them on this computer.

export interface Relay {
  id: string;
  name: string;
  /** Where the API answers, such as `http://127.0.0.1:4480`. */
  baseUrl: string;
  /** The relay's MoQ address for its metrics tracks, such as `https://relay.example:4433`. */
  moqUrl?: string;
}

export interface Tokens {
  read?: string;
  write?: string;
}

const RELAYS_KEY = 'moqtail.dashboard.relays';
const ACTIVE_KEY = 'moqtail.dashboard.active';
const TOKENS_KEY = 'moqtail.dashboard.tokens';
const REFRESH_KEY = 'moqtail.dashboard.refresh_ms';

export function loadRelays(): Relay[] {
  try {
    const stored = JSON.parse(localStorage.getItem(RELAYS_KEY) ?? '[]') as Relay[];
    if (stored.length) return stored;
  } catch {
    // Unreadable storage starts over with the default below.
  }
  return [{ id: 'local', name: 'Local relay', baseUrl: 'http://127.0.0.1:4480' }];
}

export function saveRelays(relays: Relay[]): void {
  localStorage.setItem(RELAYS_KEY, JSON.stringify(relays));
}

export function loadActive(relays: Relay[]): string {
  const id = localStorage.getItem(ACTIVE_KEY);
  return relays.some(r => r.id === id) ? id! : relays[0].id;
}

export function saveActive(id: string): void {
  localStorage.setItem(ACTIVE_KEY, id);
}

type TokenStore = Record<string, Tokens>;

function readTokens(storage: Storage): TokenStore {
  try {
    return JSON.parse(storage.getItem(TOKENS_KEY) ?? '{}') as TokenStore;
  } catch {
    return {};
  }
}

export function loadTokens(relayId: string): Tokens {
  return readTokens(sessionStorage)[relayId] ?? readTokens(localStorage)[relayId] ?? {};
}

/** Keeps a relay's tokens for this browser session, or on this computer when
 *  `remember` is set. Tokens are removed from the other store. */
export function saveTokens(relayId: string, tokens: Tokens, remember: boolean): void {
  const [keep, drop] = remember ? [localStorage, sessionStorage] : [sessionStorage, localStorage];
  const kept = readTokens(keep);
  kept[relayId] = tokens;
  keep.setItem(TOKENS_KEY, JSON.stringify(kept));
  const dropped = readTokens(drop);
  delete dropped[relayId];
  drop.setItem(TOKENS_KEY, JSON.stringify(dropped));
}

export function tokensRemembered(relayId: string): boolean {
  return relayId in readTokens(localStorage);
}

export function loadRefreshMs(): number {
  const value = Number(localStorage.getItem(REFRESH_KEY));
  return value >= 500 ? value : 2000;
}

export function saveRefreshMs(value: number): void {
  localStorage.setItem(REFRESH_KEY, String(value));
}
