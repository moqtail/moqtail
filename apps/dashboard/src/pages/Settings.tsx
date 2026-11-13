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

import { useState } from 'preact/hooks';
import { Card } from '../components/ui';
import type { Relay, Tokens } from '../lib/relays';

export interface SettingsProps {
  relays: Relay[];
  activeId: string;
  tokens: Tokens;
  remember: boolean;
  refreshMs: number;
  onRelays: (relays: Relay[]) => void;
  onTokens: (tokens: Tokens, remember: boolean) => void;
  onRefreshMs: (value: number) => void;
}

const field = 'mt-1 block w-full';

function RelayForm(props: { relay: Relay; onSave: (relay: Relay) => void; onRemove?: () => void }) {
  const [draft, setDraft] = useState(props.relay);
  const set = (key: keyof Relay) => (e: { currentTarget: HTMLInputElement }) =>
    setDraft({ ...draft, [key]: e.currentTarget.value });
  return (
    <form
      class="grid gap-2 sm:grid-cols-[1fr_2fr_auto] sm:items-end"
      onSubmit={e => {
        e.preventDefault();
        props.onSave({ ...draft, baseUrl: draft.baseUrl.trim() });
      }}
    >
      <label class="text-xs text-neutral-400">
        Name
        <input class={field} required value={draft.name} onInput={set('name')} />
      </label>
      <label class="text-xs text-neutral-400">
        API address
        <input
          class={field}
          required
          type="url"
          placeholder="http://127.0.0.1:4480"
          value={draft.baseUrl}
          onInput={set('baseUrl')}
        />
      </label>
      <div class="flex gap-2">
        <button class="btn">Save</button>
        {props.onRemove && (
          <button type="button" class="btn btn-danger" onClick={props.onRemove}>
            Remove
          </button>
        )}
      </div>
    </form>
  );
}

export function Settings(props: SettingsProps) {
  const [tokens, setTokens] = useState(props.tokens);
  const [remember, setRemember] = useState(props.remember);
  const [saved, setSaved] = useState(false);
  const active = props.relays.find(r => r.id === props.activeId);

  return (
    <div class="space-y-4">
      <Card title="Relays">
        <div class="space-y-3">
          {props.relays.map(relay => (
            <RelayForm
              key={relay.id}
              relay={relay}
              onSave={updated =>
                props.onRelays(props.relays.map(r => (r.id === relay.id ? updated : r)))
              }
              onRemove={
                props.relays.length > 1
                  ? () => props.onRelays(props.relays.filter(r => r.id !== relay.id))
                  : undefined
              }
            />
          ))}
          <h3 class="pt-2 text-sm font-semibold">Add a relay</h3>
          <RelayForm
            key={props.relays.length}
            relay={{ id: '', name: '', baseUrl: '' }}
            onSave={relay =>
              props.onRelays([...props.relays, { ...relay, id: crypto.randomUUID() }])
            }
          />
          <p class="text-xs text-neutral-500">
            The relay must list this page's origin ({location.origin}) in{' '}
            <code>api_cors_origins</code>.
          </p>
        </div>
      </Card>
      <Card title={`Tokens for ${active?.name ?? 'this relay'}`}>
        <form
          class="space-y-2"
          onSubmit={e => {
            e.preventDefault();
            props.onTokens(tokens, remember);
            setSaved(true);
          }}
        >
          <label class="block text-xs text-neutral-400">
            Read token
            <input
              class={field}
              type="password"
              autoComplete="off"
              value={tokens.read ?? ''}
              onInput={e => {
                setTokens({ ...tokens, read: e.currentTarget.value });
                setSaved(false);
              }}
            />
          </label>
          <label class="block text-xs text-neutral-400">
            Write token, for actions on connections
            <input
              class={field}
              type="password"
              autoComplete="off"
              value={tokens.write ?? ''}
              onInput={e => {
                setTokens({ ...tokens, write: e.currentTarget.value });
                setSaved(false);
              }}
            />
          </label>
          <label class="flex items-center gap-2 text-xs text-neutral-400">
            <input
              type="checkbox"
              checked={remember}
              onChange={e => {
                setRemember(e.currentTarget.checked);
                setSaved(false);
              }}
            />
            Remember on this computer (otherwise forgotten when the browser closes)
          </label>
          <button class="btn btn-primary">Save tokens</button>
          {saved && <span class="ml-2 text-sm text-emerald-400">Saved.</span>}
        </form>
      </Card>
      <Card title="Refresh">
        <label class="text-xs text-neutral-400">
          Reload pages every{' '}
          <select
            value={props.refreshMs}
            onChange={e => props.onRefreshMs(Number(e.currentTarget.value))}
          >
            {[1000, 2000, 5000, 10000, 30000].map(v => (
              <option key={v} value={v}>
                {v / 1000} s
              </option>
            ))}
          </select>
        </label>
      </Card>
    </div>
  );
}
