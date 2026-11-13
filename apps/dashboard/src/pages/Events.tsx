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
import type { QlogEvent } from '../api/types';
import { Card, ErrorNote } from '../components/ui';
import { useDashboard } from '../context';
import { SseParser } from '../lib/stream';
import { href } from '../lib/route';

/** Lines kept on screen; older ones are dropped so a busy relay cannot fill memory. */
const MAX_LINES = 1000;

interface Line {
  key: number;
  kind: string;
  event?: QlogEvent;
  text?: string;
}

export function Events() {
  const { api } = useDashboard();
  const [conn, setConn] = useState('');
  const [name, setName] = useState('');
  const [paused, setPaused] = useState(false);
  const [lines, setLines] = useState<Line[]>([]);
  const [error, setError] = useState<ApiError>();

  useEffect(() => {
    if (paused) return;
    const controller = new AbortController();
    const parser = new SseParser();
    let key = 0;
    setError(undefined);
    api
      .stream(
        '/events',
        { conn: conn || undefined, name: name || undefined },
        text => {
          const added: Line[] = parser.push(text).map(m => {
            if (m.event === 'qlog') {
              try {
                return { key: key++, kind: m.event, event: JSON.parse(m.data) as QlogEvent };
              } catch {
                // An unreadable event is shown as text.
              }
            }
            return { key: key++, kind: m.event, text: m.data };
          });
          if (added.length)
            setLines(previous => [...added.reverse(), ...previous].slice(0, MAX_LINES));
        },
        controller.signal,
      )
      .catch(e => {
        if (!(e instanceof DOMException && e.name === 'AbortError')) {
          setError(e instanceof ApiError ? e : new ApiError(0, 'error', String(e)));
        }
      });
    return () => controller.abort();
  }, [api, conn, name, paused]);

  return (
    <Card
      title="Live events"
      actions={
        <div class="flex flex-wrap items-center gap-2 text-sm">
          <label>
            Connection{' '}
            <input
              class="w-20 rounded border px-1 dark:bg-zinc-800"
              inputMode="numeric"
              value={conn}
              onChange={e => setConn(e.currentTarget.value.trim())}
            />
          </label>
          <label>
            Event name{' '}
            <input
              class="w-48 rounded border px-1 dark:bg-zinc-800"
              placeholder="moqt:control_message"
              value={name}
              onChange={e => setName(e.currentTarget.value.trim())}
            />
          </label>
          <button type="button" class="rounded border px-2" onClick={() => setPaused(!paused)}>
            {paused ? 'Resume' : 'Pause'}
          </button>
          <button type="button" class="rounded border px-2" onClick={() => setLines([])}>
            Clear
          </button>
        </div>
      }
    >
      {error && <ErrorNote error={error} />}
      <p class="mb-2 text-xs text-zinc-500">
        Newest first, the last {MAX_LINES} kept.{' '}
        {paused ? 'Paused: nothing is received.' : 'Receiving.'}
      </p>
      <ol class="max-h-[70vh] overflow-auto font-mono text-xs" aria-live="off">
        {lines.map(line => (
          <li key={line.key} class="border-b border-zinc-100 py-0.5 dark:border-zinc-800">
            {line.event ? (
              <>
                <a
                  class="text-blue-600 hover:underline dark:text-blue-400"
                  href={href('connections', line.event.conn_id)}
                >
                  [{line.event.conn_id}]
                </a>{' '}
                <span class="font-semibold">{line.event.name}</span>{' '}
                {JSON.stringify(line.event.data)}
              </>
            ) : (
              <span class="text-zinc-500">
                {line.kind}: {line.text}
              </span>
            )}
          </li>
        ))}
      </ol>
    </Card>
  );
}
