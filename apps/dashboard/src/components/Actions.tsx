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

import { useEffect, useState } from 'preact/hooks';
import { ApiError } from '../api/client';
import { useDashboard } from '../context';
import { href } from '../lib/route';
import { Card, ErrorNote } from './ui';

const button = 'rounded border px-2 py-1 text-sm disabled:opacity-40';
const field = 'rounded border px-2 py-1 text-sm dark:bg-zinc-800';

/** Runs one change at a time and reports how it went. */
function useAction() {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<ApiError>();
  const [done, setDone] = useState<string>();
  const run = async (label: string, action: () => Promise<unknown>) => {
    setBusy(true);
    setError(undefined);
    setDone(undefined);
    try {
      await action();
      setDone(label);
    } catch (e) {
      setError(e instanceof ApiError ? e : new ApiError(0, 'error', String(e)));
    } finally {
      setBusy(false);
    }
  };
  return { busy, error, done, run };
}

function NoWriteToken() {
  return (
    <p class="text-sm text-zinc-500">
      Set the write token in{' '}
      <a class="text-blue-600 underline dark:text-blue-400" href={href('settings')}>
        Settings
      </a>{' '}
      to act on the relay.
    </p>
  );
}

function Outcome(props: { error?: ApiError; done?: string }) {
  if (props.error) return <ErrorNote error={props.error} />;
  return props.done ? (
    <p role="status" class="text-sm text-green-700 dark:text-green-400">
      {props.done}
    </p>
  ) : null;
}

/** GOAWAY and close for one connection. */
export function Actions(props: { connId: number }) {
  const { writeApi } = useDashboard();
  const [timeoutMs, setTimeoutMs] = useState(10000);
  const action = useAction();
  if (!writeApi) return <NoWriteToken />;
  const path = `/connections/${props.connId}`;

  return (
    <div class="space-y-2">
      <div class="flex flex-wrap items-center gap-2">
        <label class="text-sm">
          Move within{' '}
          <input
            class={`${field} w-24`}
            type="number"
            min={0}
            max={600000}
            step={1000}
            value={timeoutMs}
            onInput={e => setTimeoutMs(Number(e.currentTarget.value))}
          />{' '}
          ms
        </label>
        <button
          type="button"
          class={button}
          disabled={action.busy}
          onClick={() =>
            action.run('GOAWAY sent.', () =>
              writeApi.write('POST', `${path}/goaway`, { timeout_ms: timeoutMs }),
            )
          }
        >
          Send GOAWAY
        </button>
        <button
          type="button"
          class={`${button} text-red-700 dark:text-red-400`}
          disabled={action.busy}
          onClick={() => {
            if (confirm(`Close connection ${props.connId} now? The client is not asked first.`)) {
              void action.run('Connection closed.', () => writeApi.write('DELETE', path));
            }
          }}
        >
          Close now
        </button>
      </div>
      <Outcome error={action.error} done={action.done} />
    </div>
  );
}

/** Drain, the GOAWAY redirect URI and the log filter, for the whole relay. */
export function RelayActions() {
  const { api, writeApi } = useDashboard();
  const [filter, setFilter] = useState('');
  const [uri, setUri] = useState('');
  const action = useAction();

  useEffect(() => {
    const controller = new AbortController();
    api.get<{ filter: string | null }>('/logging', undefined, controller.signal).then(
      r => setFilter(r.filter ?? ''),
      () => {},
    );
    api.get<{ uri: string | null }>('/config/redirect_uri', undefined, controller.signal).then(
      r => setUri(r.uri ?? ''),
      () => {},
    );
    return () => controller.abort();
  }, [api]);

  return (
    <Card title="Relay actions">
      {!writeApi ? (
        <NoWriteToken />
      ) : (
        <div class="space-y-3">
          <form
            class="flex flex-wrap items-center gap-2"
            onSubmit={e => {
              e.preventDefault();
              void action.run('Log filter changed.', () =>
                writeApi.write('PUT', '/logging', undefined, { filter }),
              );
            }}
          >
            <label class="text-sm">
              Log filter{' '}
              <input
                class={`${field} w-64`}
                value={filter}
                placeholder="info,relay=debug"
                onInput={e => setFilter(e.currentTarget.value)}
              />
            </label>
            <button class={button} disabled={action.busy}>
              Apply
            </button>
          </form>
          <form
            class="flex flex-wrap items-center gap-2"
            onSubmit={e => {
              e.preventDefault();
              void action.run('Redirect URI changed.', () =>
                writeApi.write('PUT', '/config/redirect_uri', undefined, { uri: uri || null }),
              );
            }}
          >
            <label class="text-sm">
              GOAWAY sends clients to{' '}
              <input
                class={`${field} w-64`}
                value={uri}
                placeholder="none"
                onInput={e => setUri(e.currentTarget.value)}
              />
            </label>
            <button class={button} disabled={action.busy}>
              Apply
            </button>
          </form>
          <button
            type="button"
            class={`${button} text-red-700 dark:text-red-400`}
            disabled={action.busy}
            onClick={() => {
              if (
                confirm(
                  'Drain the relay? It refuses new requests and sends GOAWAY to every client until it is restarted.',
                )
              ) {
                void action.run('Draining.', () => writeApi.write('POST', '/drain'));
              }
            }}
          >
            Drain the relay
          </button>
          <Outcome error={action.error} done={action.done} />
        </div>
      )}
    </Card>
  );
}
