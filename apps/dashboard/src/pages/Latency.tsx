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
import type { LatencyReport, Slowlog, Summary } from '../api/types';
import { Card, ErrorNote, Loading, Pager, Table } from '../components/ui';
import { useDashboard } from '../context';
import { clock, count, ms } from '../lib/format';
import { usePoll } from '../lib/poll';
import { href } from '../lib/route';

/** Slowlog entries per page; the relay keeps up to `slowlog_size` of them. */
const SLOWLOG_PAGE = 25;

function SummaryCells(props: { s: Summary }) {
  return (
    <>
      <td>{count(props.s.count)}</td>
      <td>{ms(props.s.p50)}</td>
      <td>{ms(props.s.p95)}</td>
      <td>{ms(props.s.p99)}</td>
      <td>{ms(props.s.max)}</td>
    </>
  );
}

const SUMMARY_HEAD = (
  <>
    <th>Count</th>
    <th>p50</th>
    <th>p95</th>
    <th>p99</th>
    <th>Max</th>
  </>
);

export function Latency() {
  const { api, refreshMs } = useDashboard();
  const [window, setWindow] = useState('60s');
  const [slowOffset, setSlowOffset] = useState(0);
  const polled = usePoll(
    async signal => {
      const [report, slowlog] = await Promise.all([
        api.get<LatencyReport>('/latency', { window }, signal),
        api.get<Slowlog>('/latency/slow', undefined, signal),
      ]);
      return { report, slowlog };
    },
    refreshMs,
    [api, window],
  );

  if (polled.error && !polled.data) return <ErrorNote error={polled.error} />;
  if (!polled.data) return <Loading />;
  const { report, slowlog } = polled.data;

  return (
    <div class="space-y-4">
      <Card
        title="Object processing time"
        actions={
          <label class="text-xs text-neutral-400">
            Window{' '}
            <select value={window} onChange={e => setWindow(e.currentTarget.value)}>
              <option value="10s">10 seconds</option>
              <option value="60s">1 minute</option>
              <option value="5m">5 minutes</option>
              <option value="start">since start</option>
            </select>
          </label>
        }
      >
        <p class="mb-2 text-xs text-neutral-500">
          From arrival to written, per delivery. Values are histogram bucket bounds, within a factor
          of two above the real value.
          {report.covered_s !== null && ` Covers the last ${Math.round(report.covered_s)} s.`}
        </p>
        <Table>
          <thead>
            <tr>
              <th>Stage</th>
              {SUMMARY_HEAD}
            </tr>
          </thead>
          <tbody>
            {(['ingest', 'queue', 'write', 'total'] as const).map(stage => (
              <tr key={stage}>
                <td>{stage}</td>
                <SummaryCells s={report.stages_ms[stage]} />
              </tr>
            ))}
            <tr>
              <td>first object</td>
              <SummaryCells s={report.first_object_ms} />
            </tr>
          </tbody>
        </Table>
      </Card>
      <div class="grid gap-4 lg:grid-cols-2">
        <Card title="Slowest tracks">
          <Table>
            <thead>
              <tr>
                <th>Track</th>
                {SUMMARY_HEAD}
              </tr>
            </thead>
            <tbody>
              {report.slowest_tracks.map(t => (
                <tr key={t.track_id}>
                  <td>
                    <a class="link" href={href('tracks', t.track_id)}>
                      {t.track ?? t.track_id}
                    </a>
                  </td>
                  <SummaryCells s={t} />
                </tr>
              ))}
            </tbody>
          </Table>
        </Card>
        <Card title="Slowest subscriptions (last minute)">
          <Table>
            <thead>
              <tr>
                <th>Connection / request</th>
                {SUMMARY_HEAD}
              </tr>
            </thead>
            <tbody>
              {report.slowest_subscriptions.map(s => (
                <tr key={`${s.conn_id}-${s.request_id}`}>
                  <td>
                    <a class="link" href={href('connections', s.conn_id)}>
                      {s.conn_id}
                    </a>{' '}
                    / {s.request_id}
                  </td>
                  <SummaryCells s={s} />
                </tr>
              ))}
            </tbody>
          </Table>
        </Card>
      </div>
      <Card
        title={`Slowlog: deliveries over ${ms(slowlog.threshold_ms)}, first objects over ${ms(slowlog.first_object_threshold_ms)}`}
      >
        {slowlog.dropped > 0 && (
          <p class="mb-2 text-xs text-neutral-500">
            {count(slowlog.dropped)} entries were not kept because the list was busy.
          </p>
        )}
        <Table>
          <thead>
            <tr>
              <th>Time</th>
              <th>Kind</th>
              <th>Total</th>
              <th>Ingest / queue / write</th>
              <th>Track</th>
              <th>Connection / request</th>
              <th>Location</th>
            </tr>
          </thead>
          <tbody>
            {slowlog.entries.slice(slowOffset, slowOffset + SLOWLOG_PAGE).map((e, i) => (
              <tr key={i}>
                <td>{clock(e.at_ms)}</td>
                <td>{e.kind}</td>
                <td>{ms(e.total_ms)}</td>
                <td>
                  {e.stages_ms
                    ? `${ms(e.stages_ms.ingest)} / ${ms(e.stages_ms.queue)} / ${ms(e.stages_ms.write)}`
                    : '–'}
                </td>
                <td>
                  <a class="link" href={href('tracks', e.track_id)}>
                    {e.track_id}
                  </a>
                </td>
                <td>
                  <a class="link" href={href('connections', e.conn_id)}>
                    {e.conn_id}
                  </a>{' '}
                  / {e.request_id}
                </td>
                <td>{e.location ?? '–'}</td>
              </tr>
            ))}
          </tbody>
        </Table>
        {slowlog.entries.length === 0 ? (
          <p class="mt-2 text-xs text-neutral-500">Nothing over the thresholds.</p>
        ) : (
          <Pager
            total={slowlog.entries.length}
            offset={Math.min(slowOffset, slowlog.entries.length - 1)}
            limit={SLOWLOG_PAGE}
            onOffset={setSlowOffset}
            noun="entries kept, newest first"
          />
        )}
      </Card>
    </div>
  );
}
