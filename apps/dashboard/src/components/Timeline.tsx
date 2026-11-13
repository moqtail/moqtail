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

import { useEffect, useMemo, useState } from 'preact/hooks';
import type { QlogEvent } from '../api/types';
import { ApiError } from '../api/client';
import { Badge, Card, ErrorNote } from './ui';
import { useDashboard } from '../context';
import { JsonSeqParser } from '../lib/stream';
import { buildTimeline, type TimelineMessage } from '../lib/timeline';

/** Events kept in memory for one connection's timeline. */
const MAX_EVENTS = 20000;

function Line(props: { message: TimelineMessage; showHex: boolean; nested?: boolean }) {
  const m = props.message;
  return (
    <li class={props.nested ? 'ml-6' : ''}>
      <span class="inline-block w-20 text-right text-neutral-500">{m.at.toFixed(3)}s</span>{' '}
      <span
        class={m.direction === '<-' ? 'text-blue-400' : 'text-emerald-400'}
        aria-label={m.direction === '<-' ? 'received' : 'sent'}
      >
        {m.direction}
      </span>{' '}
      <span class={m.type === 'request_error' ? 'text-red-400' : ''}>{m.text}</span>
      {props.showHex && <div class="ml-24 text-xs break-all text-neutral-500">{m.hex}</div>}
    </li>
  );
}

/**
 * Every control message and stream header of one connection, requests grouped with
 * their responses. While the connection is open the trace is followed live.
 */
export function Timeline(props: { connId: number; open: boolean }) {
  const { api } = useDashboard();
  const [events, setEvents] = useState<QlogEvent[]>([]);
  const [error, setError] = useState<ApiError>();
  const [showHex, setShowHex] = useState(false);
  const [filter, setFilter] = useState('');

  useEffect(() => {
    setEvents([]);
    setError(undefined);
    const controller = new AbortController();
    const parser = new JsonSeqParser();
    api
      .stream(
        `/qlog/${props.connId}`,
        { follow: props.open },
        text => {
          const added = parser
            .push(text)
            .filter((r): r is QlogEvent => typeof (r as QlogEvent).time === 'number');
          if (added.length) setEvents(previous => [...previous, ...added].slice(-MAX_EVENTS));
        },
        controller.signal,
      )
      .catch(e => {
        if (!(e instanceof DOMException && e.name === 'AbortError')) {
          setError(e instanceof ApiError ? e : new ApiError(0, 'error', String(e)));
        }
      });
    return () => controller.abort();
  }, [api, props.connId, props.open]);

  const timeline = useMemo(() => buildTimeline(events), [events]);
  const matches = (text: string) => !filter || text.toLowerCase().includes(filter.toLowerCase());

  const download = async () => {
    const text = await api.text(`/qlog/${props.connId}`);
    const url = URL.createObjectURL(new Blob([text], { type: 'application/qlog+json-seq' }));
    const link = Object.assign(document.createElement('a'), {
      href: url,
      download: `conn${props.connId}.sqlog`,
    });
    link.click();
    URL.revokeObjectURL(url);
  };

  return (
    <Card
      title="Message timeline"
      actions={
        <div class="flex items-center gap-3 text-sm">
          {props.open && <Badge tone="ok">live</Badge>}
          <input
            class="w-36"
            placeholder="filter, e.g. subscribe"
            value={filter}
            onInput={e => setFilter(e.currentTarget.value)}
            aria-label="Filter messages"
          />
          <label class="flex items-center gap-1">
            <input
              type="checkbox"
              checked={showHex}
              onChange={e => setShowHex(e.currentTarget.checked)}
            />
            Raw hex
          </label>
          <button type="button" class="btn" onClick={download}>
            Download .sqlog
          </button>
        </div>
      }
    >
      {error && <ErrorNote error={error} />}
      {timeline.peerSetup && (
        <p class="mb-2 text-xs">
          <span class="text-neutral-500">Peer SETUP:</span> {timeline.peerSetup.text}
        </p>
      )}
      <ol class="space-y-0.5 font-mono text-xs">
        {timeline.entries.map((entry, i) => {
          if (entry.kind === 'error') {
            return (
              <li key={i} class="text-red-400">
                <span class="inline-block w-20 text-right">{entry.at.toFixed(3)}s</span> !!{' '}
                {entry.message}
                <div class="ml-24 break-all">{entry.hex}</div>
              </li>
            );
          }
          if (entry.kind === 'stream_end') {
            const text = `${entry.end === 'reset' ? 'RESET_STREAM' : 'STOP_SENDING'} stream ${entry.streamId}${entry.label ? ` (${entry.label})` : ''} error_code=${entry.errorCode}`;
            return matches(text) ? (
              <li key={i} class="text-amber-300">
                <span class="inline-block w-20 text-right text-neutral-500">
                  {entry.at.toFixed(3)}s
                </span>{' '}
                <span aria-label={entry.direction === '<-' ? 'received' : 'sent'}>
                  {entry.direction}
                </span>{' '}
                {text}
              </li>
            ) : null;
          }
          if (entry.kind === 'close') {
            return (
              <li key={i} class="mt-1 border-t border-white/6 pt-1 text-red-300">
                <span class="inline-block w-20 text-right text-neutral-500">
                  {entry.at.toFixed(3)}s
                </span>{' '}
                CONNECTION CLOSED by {entry.by}
                {entry.errorCode !== null && ` error_code=${entry.errorCode}`}
                {entry.reason && ` '${entry.reason}'`}
              </li>
            );
          }
          if (entry.kind === 'message') {
            return matches(entry.message.text) ? (
              <Line key={i} message={entry.message} showHex={showHex} />
            ) : null;
          }
          const shown = matches(entry.request.text) || entry.responses.some(r => matches(r.text));
          return shown ? (
            <li key={i}>
              <ol>
                <Line message={entry.request} showHex={showHex} />
                {entry.responses.map((r, j) => (
                  <Line key={j} message={r} showHex={showHex} nested />
                ))}
              </ol>
            </li>
          ) : null;
        })}
      </ol>
      {timeline.data.length > 0 && (
        <ul class="mt-3 text-xs text-neutral-300">
          {timeline.data.map(d => (
            <li key={`${d.direction}${d.label}`}>
              data {d.direction === '<-' ? 'from peer' : 'to peer'}: {d.label}: {d.streams}{' '}
              stream(s)
              {d.groups &&
                `, groups ${d.groups.first}..${d.groups.last} (${d.groups.distinct} distinct)`}
              {`; ${d.fin} ended with FIN`}
              {d.cut > 0 && <span class="text-amber-300">, {d.cut} cut short</span>}
            </li>
          ))}
        </ul>
      )}
      {!error && events.length === 0 && <p class="text-xs text-neutral-500">No events yet.</p>}
    </Card>
  );
}
