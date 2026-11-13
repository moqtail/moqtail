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

import { useRef } from 'preact/hooks';
import type { LatencyReport, Overview as OverviewData } from '../api/types';
import { RelayActions } from '../components/Actions';
import { Chart } from '../components/Chart';
import { Badge, Card, ErrorNote, Loading, Stat, Table } from '../components/ui';
import { hasFeature, useDashboard } from '../context';
import { bytes, count, duration, ms, rate } from '../lib/format';
import { usePoll } from '../lib/poll';

/** Points kept for the charts: ten minutes at the default two-second refresh. */
const HISTORY = 300;

interface History {
  times: number[];
  objectsIn: (number | null)[];
  objectsOut: (number | null)[];
  bytesIn: (number | null)[];
  bytesOut: (number | null)[];
  p50: (number | null)[];
  p99: (number | null)[];
  last?: { at: number; traffic: NonNullable<OverviewData['traffic']> };
}

function emptyHistory(): History {
  return { times: [], objectsIn: [], objectsOut: [], bytesIn: [], bytesOut: [], p50: [], p99: [] };
}

/** Adds one point: rates from the change in the running totals since the last poll. */
function record(
  history: History,
  overview: OverviewData,
  latency: LatencyReport | undefined,
): History {
  const at = Date.now() / 1000;
  const next: History = {
    ...history,
    times: [...history.times, at].slice(-HISTORY),
    p50: [...history.p50, latency?.stages_ms.total.p50 ?? null].slice(-HISTORY),
    p99: [...history.p99, latency?.stages_ms.total.p99 ?? null].slice(-HISTORY),
  };
  const traffic = overview.traffic;
  const per = (now: number, then: number, elapsed: number) => Math.max(0, now - then) / elapsed;
  const previous = history.last;
  const elapsed = previous ? at - previous.at : 0;
  const push = (list: (number | null)[], value: number | null) => [...list, value].slice(-HISTORY);
  next.objectsIn = push(
    history.objectsIn,
    traffic && previous && elapsed > 0
      ? per(traffic.objects_received, previous.traffic.objects_received, elapsed)
      : null,
  );
  next.objectsOut = push(
    history.objectsOut,
    traffic && previous && elapsed > 0
      ? per(traffic.objects_sent, previous.traffic.objects_sent, elapsed)
      : null,
  );
  next.bytesIn = push(
    history.bytesIn,
    traffic && previous && elapsed > 0
      ? per(traffic.bytes_received, previous.traffic.bytes_received, elapsed)
      : null,
  );
  next.bytesOut = push(
    history.bytesOut,
    traffic && previous && elapsed > 0
      ? per(traffic.bytes_sent, previous.traffic.bytes_sent, elapsed)
      : null,
  );
  next.last = traffic ? { at, traffic } : undefined;
  return next;
}

export function Overview() {
  const dashboard = useDashboard();
  const { api, refreshMs } = dashboard;
  const history = useRef<History>(emptyHistory());
  const latencyOn = hasFeature(dashboard, 'latency');

  const polled = usePoll(
    async signal => {
      const [overview, latency] = await Promise.all([
        api.get<OverviewData>('/overview', undefined, signal),
        latencyOn
          ? api.get<LatencyReport>('/latency', { window: '10s' }, signal).catch(() => undefined)
          : undefined,
      ]);
      history.current = record(history.current, overview, latency);
      return { overview, latency };
    },
    refreshMs,
    [api],
  );

  if (polled.error && !polled.data) return <ErrorNote error={polled.error} />;
  if (!polled.data) return <Loading />;
  const { overview, latency } = polled.data;
  const h = history.current;
  const lastRate = (list: (number | null)[]) => list[list.length - 1] ?? null;
  const connections = Object.values(overview.connections).reduce((a, b) => a + b, 0);

  return (
    <div class="space-y-4">
      {polled.error && <ErrorNote error={polled.error} />}
      <div class="flex flex-wrap items-center gap-2 text-sm">
        <Badge tone={overview.draining ? 'warn' : overview.accepting ? 'ok' : 'bad'}>
          {overview.draining ? 'draining' : overview.accepting ? 'ready' : 'starting'}
        </Badge>
        <span>{overview.version}</span>
        <span class="text-neutral-500">
          up {duration((Date.now() - overview.started_at_ms) / 1000)} · pid {overview.pid}
        </span>
      </div>
      <div class="grid grid-cols-2 gap-3 md:grid-cols-4 lg:grid-cols-6">
        <Stat
          label="Connections"
          value={count(connections)}
          hint={Object.entries(overview.connections)
            .map(([s, n]) => `${n} ${s}`)
            .join(', ')}
        />
        <Stat label="Tracks" value={count(overview.tracks)} />
        <Stat label="Subscriptions" value={count(overview.subscriptions)} />
        <Stat label="Cached groups" value={count(overview.cached_groups)} />
        <Stat
          label="Objects out"
          value={lastRate(h.objectsOut) === null ? '–' : rate(lastRate(h.objectsOut)!, 'objects')}
        />
        <Stat label="Processing p99 (10s)" value={ms(latency?.stages_ms.total.p99)} />
      </div>
      {overview.traffic ? (
        <div class="grid gap-4 lg:grid-cols-2">
          <Card title="Objects per second">
            <Chart
              title="Objects per second"
              times={h.times}
              values={[h.objectsIn, h.objectsOut]}
              series={[
                { label: 'in', color: '#60a5fa' },
                { label: 'out', color: '#34d399' },
              ]}
              format={v => rate(v, 'objects')}
            />
          </Card>
          <Card title="Bytes per second">
            <Chart
              title="Bytes per second"
              times={h.times}
              values={[h.bytesIn, h.bytesOut]}
              series={[
                { label: 'in', color: '#60a5fa' },
                { label: 'out', color: '#34d399' },
              ]}
              format={v => rate(v, 'bytes')}
            />
          </Card>
        </div>
      ) : (
        <p class="text-xs text-neutral-500">Traffic rates need the relay's `metrics` feature.</p>
      )}
      {latency && (
        <Card title="Object processing time, last 10 seconds">
          <Chart
            title="Object processing time"
            times={h.times}
            values={[h.p50, h.p99]}
            series={[
              { label: 'p50', color: '#a78bfa' },
              { label: 'p99', color: '#f87171' },
            ]}
            format={v => ms(v)}
          />
        </Card>
      )}
      <Card title="Requests">
        <Table>
          <thead>
            <tr>
              <th>Type</th>
              <th>Direction</th>
              <th>OK</th>
              <th>Refused (by code)</th>
            </tr>
          </thead>
          <tbody>
            {Object.entries(overview.requests).map(([key, results]) => {
              const [type, direction] = key.split('/');
              const refused = Object.entries(results.error);
              return (
                <tr key={key}>
                  <td>{type}</td>
                  <td>{direction}</td>
                  <td>{count(results.ok)}</td>
                  <td>
                    {refused.length ? refused.map(([code, n]) => `${n} × ${code}`).join(', ') : '–'}
                  </td>
                </tr>
              );
            })}
          </tbody>
        </Table>
      </Card>
      {overview.traffic && (
        <p class="text-xs text-neutral-500">
          Since start: {bytes(overview.traffic.bytes_received)} in,{' '}
          {bytes(overview.traffic.bytes_sent)} out, {count(overview.traffic.objects_dropped)}{' '}
          objects dropped.
        </p>
      )}
      {dashboard.index?.features.includes('actions') && <RelayActions />}
    </div>
  );
}
