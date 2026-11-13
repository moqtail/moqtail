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
import type { Page, TrackDetail, TrackInfo } from '../api/types';
import { Subscriptions } from '../components/Subscriptions';
import { Badge, Card, ErrorNote, Facts, Loading, Pager, SortHeader, Table } from '../components/ui';
import { useDashboard } from '../context';
import { bytes, count, ms } from '../lib/format';
import { usePoll } from '../lib/poll';
import { href } from '../lib/route';

const LIMIT = 50;

/** A track's status. A track kept only in the cache has no publisher to confirm
 *  it, so it stays pending for good; it is shown as cached only. */
export function trackStatus(t: TrackInfo): { text: string; tone: 'ok' | 'bad' | 'warn' | 'plain' } {
  if (t.origin === 'cache_only' && t.status === 'pending')
    return { text: 'cached only', tone: 'plain' };
  const tone = t.status === 'confirmed' ? 'ok' : t.status === 'rejected' ? 'bad' : 'warn';
  return { text: t.status, tone };
}

export function Tracks() {
  const { api, refreshMs } = useDashboard();
  const [sort, setSort] = useState('-subscribers');
  const [offset, setOffset] = useState(0);
  const [prefix, setPrefix] = useState('');
  const polled = usePoll(
    signal => api.get<Page<TrackInfo>>('/tracks', { sort, limit: LIMIT, offset, prefix }, signal),
    refreshMs,
    [api, sort, offset, prefix],
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
      title="Tracks"
      actions={
        <label class="flex items-center gap-1.5 text-xs text-neutral-400">
          Namespace
          <input
            class="w-32"
            value={prefix}
            onInput={e => {
              setPrefix(e.currentTarget.value);
              setOffset(0);
            }}
            placeholder="prefix"
          />
        </label>
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
                {header('Id', 'track_id')}
                <th>Track</th>
                <th>Origin</th>
                <th>Status</th>
                {header('Publishers', 'publishers')}
                {header('Subscribers', 'subscribers')}
                <th>Largest</th>
                {header('Cached groups', 'cached_groups')}
                <th>Processing p50 / p99</th>
              </tr>
            </thead>
            <tbody>
              {polled.data.items.map(t => (
                <tr key={t.track_id}>
                  <td>{t.track_id}</td>
                  <td>
                    <a class="link" href={href('tracks', t.track_id)}>
                      {t.namespace} : {t.name}
                    </a>
                  </td>
                  <td>{t.origin}</td>
                  <td>
                    <Badge tone={trackStatus(t).tone}>{trackStatus(t).text}</Badge>
                  </td>
                  <td>{t.publishers.length}</td>
                  <td>{t.subscribers}</td>
                  <td>{t.largest_location ?? '–'}</td>
                  <td>{count(t.cached_groups)}</td>
                  <td>
                    {t.processing_ms
                      ? `${ms(t.processing_ms.p50)} / ${ms(t.processing_ms.p99)}`
                      : '–'}
                  </td>
                </tr>
              ))}
            </tbody>
          </Table>
          {polled.data.items.length === 0 && (
            <p class="mt-2 text-xs text-neutral-500">No tracks.</p>
          )}
          <Pager
            total={polled.data.total}
            offset={polled.data.offset}
            limit={LIMIT}
            onOffset={setOffset}
            noun="tracks"
          />
        </>
      )}
    </Card>
  );
}

export function Track(props: { id: string }) {
  const { api, refreshMs } = useDashboard();
  const polled = usePoll(
    signal => api.get<TrackDetail>(`/tracks/${props.id}`, undefined, signal),
    refreshMs,
    [api, props.id],
  );
  if (polled.error && !polled.data) return <ErrorNote error={polled.error} />;
  if (!polled.data) return <Loading />;
  const t = polled.data;
  const groups = t.cached_group_ids;

  return (
    <div class="space-y-4">
      <h1 class="text-lg font-semibold">
        {t.namespace} : {t.name}
      </h1>
      <Card title="Fan-out">
        <div class="flex flex-wrap items-center gap-4 text-sm">
          <div>
            <div class="text-xs text-neutral-500">Publishers</div>
            {t.publishers.length ? (
              t.publishers.map(p => (
                <div key={p.conn_id}>
                  <a class="link" href={href('connections', p.conn_id)}>
                    connection {p.conn_id}
                  </a>{' '}
                  <span class="text-neutral-500">alias {p.alias}</span>
                </div>
              ))
            ) : (
              <div class="text-neutral-500">none</div>
            )}
          </div>
          <div aria-hidden="true" class="text-2xl text-neutral-600">
            →
          </div>
          <div class="rounded-lg border border-white/6 bg-neutral-950 px-3 py-2 text-center font-mono text-xs">
            relay
            <div class="text-xs text-neutral-500">
              {count(t.cached_groups)} groups, {bytes(t.cached_bytes)} cached
            </div>
          </div>
          <div aria-hidden="true" class="text-2xl text-neutral-600">
            →
          </div>
          <div>
            <div class="text-xs text-neutral-500">Subscribers</div>
            {t.subscribers} subscription(s){t.forwarding ? '' : ', none forwarding'}
          </div>
        </div>
      </Card>
      <Card title="Track">
        <Facts
          rows={[
            ['Track id', String(t.track_id)],
            ['Origin', t.origin],
            ['Status', trackStatus(t).text],
            ['Largest location', t.largest_location ?? '–'],
            ['Upstream forward', t.upstream_forward ? 'yes' : 'no'],
            [
              'Cached groups',
              groups.length
                ? `${groups[0]}..${groups[groups.length - 1]} (${groups.length})`
                : 'none',
            ],
            [
              'Processing p50 / p99 (last minute)',
              t.processing_ms ? `${ms(t.processing_ms.p50)} / ${ms(t.processing_ms.p99)}` : '–',
            ],
          ]}
        />
      </Card>
      <Card title="Subscriptions">
        <Subscriptions subscriptions={t.subscriptions} showConn />
      </Card>
    </div>
  );
}
