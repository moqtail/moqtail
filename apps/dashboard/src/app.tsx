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
import { ApiClient, type ApiError } from './api/client';
import type { ApiIndex } from './api/types';
import { ErrorNote } from './components/ui';
import { type Dashboard, DashboardContext } from './context';
import { Connection } from './pages/Connection';
import { Connections } from './pages/Connections';
import { Events } from './pages/Events';
import { Latency } from './pages/Latency';
import { Namespaces } from './pages/Namespaces';
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

/** Whether the relay answers, as the other MOQtail apps show it: a dot and a word. */
function RelayStatus(props: { index?: ApiIndex; error?: ApiError }) {
  const [dot, label] = props.index
    ? ['bg-emerald-400', 'Connected']
    : props.error
      ? ['bg-red-400', 'Unreachable']
      : ['bg-yellow-400 animate-pulse', 'Connecting…'];
  return (
    <span class="flex items-center gap-1.5 text-xs text-neutral-400 select-none">
      <span class={`h-2 w-2 rounded-full ${dot}`} />
      {label}
    </span>
  );
}

/** The navigation; a page shows only when the relay serves its feature. */
const PAGES: { page: string; label: string; feature?: string }[] = [
  { page: 'overview', label: 'Overview', feature: 'state' },
  { page: 'connections', label: 'Connections', feature: 'state' },
  { page: 'tracks', label: 'Tracks', feature: 'state' },
  { page: 'namespaces', label: 'Namespaces', feature: 'state' },
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
    case 'namespaces':
      content = <Namespaces />;
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
      <div class="min-h-screen">
        <header class="sticky top-0 z-10 border-b border-white/6 bg-neutral-950/80 backdrop-blur-sm">
          <div class="flex h-12 items-center justify-between gap-4 px-4 md:px-5">
            <div class="flex items-center gap-2.5">
              <img src="/favicon.svg" alt="MOQtail logo" class="h-5 w-5" />
              <a
                href="https://moqtail.dev"
                target="_blank"
                rel="noreferrer"
                class="text-sm font-semibold tracking-tight transition-colors hover:text-neutral-300"
              >
                MOQtail Relay
              </a>
              <span class="text-neutral-700 select-none">·</span>
              <RelayStatus index={index} error={indexError} />
            </div>
            <div class="flex items-center gap-3">
              {index && (
                <span class="hidden font-mono text-[10px] text-neutral-600 tabular-nums sm:block">
                  {index.version}
                </span>
              )}
              <label>
                <span class="sr-only">Relay</span>
                <select value={relay.id} onChange={e => selectRelay(e.currentTarget.value)}>
                  {relays.map(r => (
                    <option key={r.id} value={r.id}>
                      {r.name}
                    </option>
                  ))}
                </select>
              </label>
            </div>
          </div>
          <nav aria-label="Pages" class="flex gap-1 overflow-x-auto px-3 md:px-4">
            {PAGES.filter(p => !p.feature || !index || index.features.includes(p.feature)).map(
              p => (
                <a
                  key={p.page}
                  href={href(p.page)}
                  aria-current={route.page === p.page ? 'page' : undefined}
                  class={`border-b-2 px-2 py-2 text-xs font-medium whitespace-nowrap transition-colors ${
                    route.page === p.page
                      ? 'border-blue-500 text-neutral-100'
                      : 'border-transparent text-neutral-400 hover:text-neutral-100'
                  }`}
                >
                  {p.label}
                </a>
              ),
            )}
          </nav>
        </header>
        <main class="mx-auto max-w-7xl space-y-4 p-4 md:p-5">
          {indexError && route.page !== 'settings' && <ErrorNote error={indexError} />}
          {content}
        </main>
      </div>
    </DashboardContext.Provider>
  );
}
