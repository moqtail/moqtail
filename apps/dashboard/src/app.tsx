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

import { useMemo, useState } from 'preact/hooks';
import { ApiClient } from './api/client';
import type { ApiIndex } from './api/types';
import { ErrorNote } from './components/ui';
import { type Dashboard, DashboardContext } from './context';
import { Connection } from './pages/Connection';
import { Connections } from './pages/Connections';
import { Events } from './pages/Events';
import { Latency } from './pages/Latency';
import { Overview } from './pages/Overview';
import { Settings } from './pages/Settings';
import { Track, Tracks } from './pages/Tracks';
import { Upstreams, Zones } from './pages/Upstreams';
import {
  loadActive,
  loadRefreshMs,
  loadRelays,
  loadTokens,
  saveActive,
  saveRefreshMs,
  saveRelays,
  saveTokens,
  tokensRemembered,
} from './lib/relays';
import { usePoll } from './lib/poll';
import { href, useRoute } from './lib/route';

/** The navigation; a page shows only when the relay serves its feature. */
const PAGES: { page: string; label: string; feature?: string }[] = [
  { page: 'overview', label: 'Overview', feature: 'state' },
  { page: 'connections', label: 'Connections', feature: 'state' },
  { page: 'tracks', label: 'Tracks', feature: 'state' },
  { page: 'latency', label: 'Latency', feature: 'latency' },
  { page: 'upstreams', label: 'Upstreams', feature: 'state' },
  { page: 'zones', label: 'Zones', feature: 'state' },
  { page: 'events', label: 'Live events', feature: 'events' },
  { page: 'settings', label: 'Settings' },
];

export function App() {
  const route = useRoute();
  const [relays, setRelays] = useState(loadRelays);
  const [activeId, setActiveId] = useState(() => loadActive(relays));
  const [tokens, setTokens] = useState(() => loadTokens(activeId));
  const [refreshMs, setRefreshMs] = useState(loadRefreshMs);
  const relay = relays.find(r => r.id === activeId) ?? relays[0];

  const api = useMemo(
    () => new ApiClient(relay.baseUrl, tokens.read),
    [relay.baseUrl, tokens.read],
  );
  const writeApi = useMemo(
    () => (tokens.write ? new ApiClient(relay.baseUrl, tokens.write) : undefined),
    [relay.baseUrl, tokens.write],
  );

  // The index says which features the relay serves. It is asked again every ten
  // seconds, so a relay that was down or restarted with other features is noticed.
  const indexPoll = usePoll(signal => api.get<ApiIndex>('', undefined, signal), 10000, [api]);
  const index = indexPoll.data;
  const indexError = index ? undefined : indexPoll.error;

  const dashboard: Dashboard = { relay, api, writeApi, refreshMs, index };
  const selectRelay = (id: string) => {
    setActiveId(id);
    saveActive(id);
    setTokens(loadTokens(id));
  };

  let content;
  switch (route.page) {
    case 'connections':
      content = route.id ? <Connection key={route.id} id={route.id} /> : <Connections />;
      break;
    case 'tracks':
      content = route.id ? <Track key={route.id} id={route.id} /> : <Tracks />;
      break;
    case 'latency':
      content = <Latency />;
      break;
    case 'upstreams':
      content = <Upstreams />;
      break;
    case 'zones':
      content = <Zones />;
      break;
    case 'events':
      content = <Events />;
      break;
    case 'settings':
      content = (
        <Settings
          key={relay.id}
          relays={relays}
          activeId={relay.id}
          tokens={tokens}
          remember={tokensRemembered(relay.id)}
          refreshMs={refreshMs}
          onRelays={next => {
            setRelays(next);
            saveRelays(next);
            if (!next.some(r => r.id === activeId)) selectRelay(next[0].id);
          }}
          onTokens={(next, remember) => {
            setTokens(next);
            saveTokens(relay.id, next, remember);
          }}
          onRefreshMs={value => {
            setRefreshMs(value);
            saveRefreshMs(value);
          }}
        />
      );
      break;
    default:
      content = <Overview />;
  }

  return (
    <DashboardContext.Provider value={dashboard}>
      <div class="min-h-screen bg-zinc-50 text-zinc-900 dark:bg-zinc-950 dark:text-zinc-100">
        <header class="flex flex-wrap items-center gap-4 border-b border-zinc-200 bg-white px-4 py-2 dark:border-zinc-800 dark:bg-zinc-900">
          <span class="font-semibold">MOQtail relay</span>
          <label class="text-sm">
            <span class="sr-only">Relay</span>
            <select
              class="rounded border px-1 dark:bg-zinc-800"
              value={relay.id}
              onChange={e => selectRelay(e.currentTarget.value)}
            >
              {relays.map(r => (
                <option key={r.id} value={r.id}>
                  {r.name}
                </option>
              ))}
            </select>
          </label>
          <nav aria-label="Pages" class="flex flex-wrap gap-3 text-sm">
            {PAGES.filter(p => !p.feature || !index || index.features.includes(p.feature)).map(
              p => (
                <a
                  key={p.page}
                  href={href(p.page)}
                  aria-current={route.page === p.page ? 'page' : undefined}
                  class={route.page === p.page ? 'font-semibold underline' : 'hover:underline'}
                >
                  {p.label}
                </a>
              ),
            )}
          </nav>
          {index && <span class="ml-auto text-xs text-zinc-500">relay {index.version}</span>}
        </header>
        <main class="mx-auto max-w-7xl space-y-4 p-4">
          {indexError && route.page !== 'settings' && <ErrorNote error={indexError} />}
          {content}
        </main>
      </div>
    </DashboardContext.Provider>
  );
}
