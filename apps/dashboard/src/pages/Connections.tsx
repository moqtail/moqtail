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
import type { ConnectionSummary, Page } from '../api/types';
import { Badge, Card, ErrorNote, Loading, Pager, SortHeader, Table } from '../components/ui';
import { useDashboard } from '../context';
import { ago, bytes, count, ms } from '../lib/format';
import { usePoll } from '../lib/poll';
import { href } from '../lib/route';

const LIMIT = 50;

export function stateTone(state: string): 'ok' | 'warn' | 'bad' | 'plain' {
  return state === 'active'
    ? 'ok'
    : state === 'draining'
      ? 'warn'
      : state === 'closed'
        ? 'bad'
        : 'plain';
}

export function Connections() {
  const { api, refreshMs } = useDashboard();
  const [closed, setClosed] = useState(false);
  const [sort, setSort] = useState('-connected_at');
  const [offset, setOffset] = useState(0);
  const [prefix, setPrefix] = useState('');

  const polled = usePoll(
    signal =>
      api.get<Page<ConnectionSummary>>(
        '/connections',
        { closed, sort, limit: LIMIT, offset, prefix },
        signal,
      ),
    refreshMs,
    [api, closed, sort, offset, prefix],
  );

  const header = (label: string, key: string) => (
    <SortHeader
      label={label}
      sortKey={key}
      sort={sort}
      onSort={s => {
        setSort(s);
        setOffset(0);
      }}
    />
  );

  return (
    <Card
      title={closed ? 'Closed connections' : 'Open connections'}
      actions={
        <div class="flex items-center gap-3 text-sm">
          <label class="flex items-center gap-1">
            Namespace
            <input
              class="w-32 rounded border px-1 dark:bg-zinc-800"
              value={prefix}
              onInput={e => {
                setPrefix(e.currentTarget.value);
                setOffset(0);
              }}
              placeholder="prefix"
            />
          </label>
          <label class="flex items-center gap-1">
            <input
              type="checkbox"
              checked={closed}
              onChange={e => {
                setClosed(e.currentTarget.checked);
                setOffset(0);
              }}
            />
            Closed
          </label>
        </div>
      }
    >
      {polled.error && <ErrorNote error={polled.error} />}
      {!polled.data ? (
        <Loading />
      ) : (
        <>
          <Table>
            <thead>
              <tr>
                {header('Id', 'conn_id')}
                <th>State</th>
                <th>Remote</th>
                <th>Implementation</th>
                <th>Transport</th>
                {header('Connected', 'connected_at')}
                {header('RTT', 'rtt')}
                {header('Lost', 'lost_packets')}
                {header('Subs', 'subscriptions')}
                <th>Requests ok / err</th>
                {header('Sent', 'bytes_out')}
                {header('Received', 'bytes_in')}
              </tr>
            </thead>
            <tbody>
              {polled.data.items.map(c => (
                <tr key={c.conn_id}>
                  <td>
                    <a
                      class="text-blue-600 hover:underline dark:text-blue-400"
                      href={href('connections', c.conn_id)}
                    >
                      {c.conn_id}
                    </a>
                  </td>
                  <td>
                    <Badge tone={stateTone(c.state)}>{c.state}</Badge>
                    {c.role === 'cluster_peer' && <Badge>peer</Badge>}
                  </td>
                  <td>{c.remote}</td>
                  <td>{c.implementation ?? '–'}</td>
                  <td>{c.transport}</td>
                  <td>{ago(c.connected_at_ms)}</td>
                  <td>{ms(c.quic.rtt_ms)}</td>
                  <td>{count(c.quic.lost_packets)}</td>
                  <td>{c.subscriptions}</td>
                  <td>
                    {c.requests.ok} / {c.requests.error}
                    {c.control_messages.parse_errors > 0 && (
                      <Badge tone="bad">{c.control_messages.parse_errors} parse errors</Badge>
                    )}
                  </td>
                  <td>{bytes(c.quic.sent_bytes)}</td>
                  <td>{bytes(c.quic.received_bytes)}</td>
                </tr>
              ))}
            </tbody>
          </Table>
          {polled.data.items.length === 0 && (
            <p class="mt-2 text-sm text-zinc-500">No {closed ? 'closed' : 'open'} connections.</p>
          )}
          <Pager
            total={polled.data.total}
            offset={polled.data.offset}
            limit={LIMIT}
            onOffset={setOffset}
          />
        </>
      )}
    </Card>
  );
}
