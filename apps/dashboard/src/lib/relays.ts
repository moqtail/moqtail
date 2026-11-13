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

// The relays the dashboard knows, kept in the browser or set by the host in
// config.json. Tokens are kept for the
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

/** A relay in `config.json`. The name defaults to the address. */
type ConfiguredRelay = Omit<Relay, 'id' | 'name'> & { name?: string };

/** What the host serving the dashboard sets in `config.json`, next to `index.html`. */
export interface SiteConfig {
  /** The one relay this dashboard shows. The relay list can then not be changed. */
  relay?: ConfiguredRelay;
  /** The relays offered until the user edits the list. */
  relays?: ConfiguredRelay[];
}

/** Reads `config.json`. A missing or unreadable file is the same as an empty one. */
export async function loadSiteConfig(): Promise<SiteConfig> {
  try {
    const response = await fetch('config.json', { cache: 'no-store' });
    if (!response.ok) return {};
    const config = (await response.json()) as SiteConfig;
    return typeof config === 'object' && config ? config : {};
  } catch (e) {
    console.warn('config.json ignored:', e);
    return {};
  }
}

const isRelay = (r: unknown): r is ConfiguredRelay =>
  typeof r === 'object' && r !== null && typeof (r as Relay).baseUrl === 'string';

/** The relay `config.json` pins the dashboard to, if any. */
export function pinnedRelay(config: SiteConfig): Relay | undefined {
  if (!isRelay(config.relay)) return undefined;
  return {
    ...config.relay,
    name: config.relay.name ?? (config.relay.baseUrl || location.host),
    id: 'configured',
  };
}

export function loadRelays(config: SiteConfig): Relay[] {
  const pinned = pinnedRelay(config);
  if (pinned) return [pinned];
  try {
    const stored = JSON.parse(localStorage.getItem(RELAYS_KEY) ?? '[]') as Relay[];
    if (stored.length) return stored;
  } catch {
    // Unreadable storage starts over with the defaults below.
  }
  const offered = Array.isArray(config.relays) ? config.relays.filter(isRelay) : [];
  if (offered.length) {
    return offered.map((r, i) => ({ ...r, name: r.name ?? r.baseUrl, id: `configured-${i}` }));
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
