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

import { useEffect, useRef, useState } from 'preact/hooks';
import { ApiError } from '../api/client';
import type { QlogEvent } from '../api/types';
import { Card, ErrorNote } from '../components/ui';
import { useDashboard } from '../context';
import { href } from '../lib/route';
import { SseParser } from '../lib/stream';

/** Lines kept on screen; older ones are dropped so a busy relay cannot fill memory. */
const MAX_LINES = 1000;
/** How often new lines are drawn. Events can arrive hundreds of times a second;
 *  drawing each one would keep the page too busy to type in. */
const FLUSH_MS = 500;

interface Line {
  key: number;
  kind: string;
  event?: QlogEvent;
  text?: string;
}

interface Filter {
  conn: string;
  name: string;
}

/** Data stream headers: one per stream, so they far outnumber control messages. */
export function isStreamHeader(name: string): boolean {
  return name.startsWith('moqt:subgroup_header') || name.startsWith('moqt:fetch_header');
}

export function Events() {
  const { api } = useDashboard();
  // What is typed, and what is applied: the stream restarts only on Apply.
  const [draft, setDraft] = useState<Filter>({ conn: '', name: '' });
  const [filter, setFilter] = useState<Filter>({ conn: '', name: '' });
  const [streams, setStreams] = useState(false);
  const [paused, setPaused] = useState(false);
  const [lines, setLines] = useState<Line[]>([]);
  const [error, setError] = useState<ApiError>();
  const streamsRef = useRef(streams);
  streamsRef.current = streams;

  useEffect(() => {
    if (paused) return;
    const controller = new AbortController();
    const parser = new SseParser();
    let key = 0;
    let pending: Line[] = [];
    const flush = setInterval(() => {
      if (pending.length === 0) return;
      const added = pending.reverse();
      pending = [];
      setLines(previous => [...added, ...previous].slice(0, MAX_LINES));
    }, FLUSH_MS);
    setError(undefined);
    api
      .stream(
        '/events',
        { conn: filter.conn || undefined, name: filter.name || undefined },
        text => {
          for (const m of parser.push(text)) {
            if (m.event === 'qlog') {
              let event: QlogEvent;
              try {
                event = JSON.parse(m.data) as QlogEvent;
              } catch {
                pending.push({ key: key++, kind: m.event, text: m.data });
                continue;
              }
              if (!streamsRef.current && isStreamHeader(event.name)) continue;
              pending.push({ key: key++, kind: m.event, event });
            } else {
              pending.push({ key: key++, kind: m.event, text: m.data });
            }
          }
          // Only the newest lines can be shown, so a burst is cut early.
          if (pending.length > MAX_LINES) pending = pending.slice(-MAX_LINES);
        },
        controller.signal,
      )
      .catch(e => {
        if (!(e instanceof DOMException && e.name === 'AbortError')) {
          setError(e instanceof ApiError ? e : new ApiError(0, 'error', String(e)));
        }
      });
    return () => {
      clearInterval(flush);
      controller.abort();
    };
  }, [api, filter, paused]);

  const changed = draft.conn !== filter.conn || draft.name !== filter.name;

  return (
    <Card
      title="Live events"
      actions={
        <form
          class="flex flex-wrap items-center gap-2 text-xs text-neutral-400"
          onSubmit={e => {
            e.preventDefault();
            setFilter({ conn: draft.conn.trim(), name: draft.name.trim() });
            setLines([]);
          }}
        >
          <label>
            Connection{' '}
            <input
              class="w-28"
              inputMode="numeric"
              placeholder="any"
              value={draft.conn}
              onInput={e => setDraft({ ...draft, conn: e.currentTarget.value })}
            />
          </label>
          <label>
            Event name starts with{' '}
            <input
              class="w-48"
              placeholder="moqt:control_message"
              value={draft.name}
              onInput={e => setDraft({ ...draft, name: e.currentTarget.value })}
            />
          </label>
          <button class={changed ? 'btn btn-primary' : 'btn'}>Apply</button>
          <label class="flex items-center gap-1.5">
            <input
              type="checkbox"
              checked={streams}
              onChange={e => setStreams(e.currentTarget.checked)}
            />
            Data stream headers
          </label>
          <button type="button" class="btn" onClick={() => setPaused(!paused)}>
            {paused ? 'Resume' : 'Pause'}
          </button>
          <button type="button" class="btn" onClick={() => setLines([])}>
            Clear
          </button>
        </form>
      }
    >
      {error && <ErrorNote error={error} />}
      <p class="mb-2 text-xs text-neutral-500">
        Newest first, the last {MAX_LINES} kept, updated twice a second.{' '}
        {paused ? 'Paused: nothing is received.' : 'Receiving.'}
        {!streams && ' Data stream headers are hidden.'}
      </p>
      <ol class="max-h-[70vh] overflow-auto font-mono text-xs" aria-live="off">
        {lines.map(line => (
          <li key={line.key} class="border-b border-white/6 py-1">
            {line.event ? (
              <>
                <a class="link" href={href('connections', line.event.conn_id)}>
                  [{line.event.conn_id}]
                </a>{' '}
                <span class="font-semibold text-neutral-100">{line.event.name}</span>{' '}
                <span class="text-neutral-400">{JSON.stringify(line.event.data)}</span>
              </>
            ) : (
              <span class="text-neutral-500">
                {line.kind}: {line.text}
              </span>
            )}
          </li>
        ))}
      </ol>
    </Card>
  );
}
