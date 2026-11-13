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

import type { ClusterInfo, Namespaces, Upstream, ZoneReport } from '../api/types';
import { Badge, Card, ErrorNote, Loading, Table } from '../components/ui';
import { useDashboard } from '../context';
import { ago, bytes, count, ms } from '../lib/format';
import { usePoll } from '../lib/poll';
import { href } from '../lib/route';

function ConnLink(props: { id: number }) {
  return (
    <a class="link" href={href('connections', props.id)}>
      {props.id}
    </a>
  );
}

export function Upstreams() {
  const { api, refreshMs } = useDashboard();
  const polled = usePoll(
    async signal => {
      const [upstreams, namespaces, clusterInfo] = await Promise.all([
        api.get<Upstream[]>('/upstreams', undefined, signal),
        api.get<Namespaces>('/namespaces', undefined, signal),
        api.get<ClusterInfo>('/cluster', undefined, signal),
      ]);
      return { upstreams, namespaces, clusterInfo };
    },
    refreshMs,
    [api],
  );
  if (polled.error && !polled.data) return <ErrorNote error={polled.error} />;
  if (!polled.data) return <Loading />;
  const { upstreams, namespaces, clusterInfo } = polled.data;

  return (
    <div class="space-y-4">
      <Card title="Upstreams">
        <p class="mb-2 text-xs text-neutral-500">
          Publishers and cluster peers the relay sends requests to.
        </p>
        <Table>
          <thead>
            <tr>
              <th>Connection</th>
              <th>Kind</th>
              <th>Health</th>
              <th>Remote</th>
              <th>Tracks</th>
              <th>RTT</th>
              <th>In flight / ok / failed / timed out</th>
              <th>Request p50 / p95</th>
              <th>Last error</th>
            </tr>
          </thead>
          <tbody>
            {upstreams.map(u => (
              <tr key={u.conn_id}>
                <td>
                  <ConnLink id={u.conn_id} />
                </td>
                <td>{u.kind}</td>
                <td>
                  <Badge tone={u.up ? 'ok' : 'bad'}>{u.up ? 'up' : 'down'}</Badge>
                </td>
                <td>{u.remote}</td>
                <td>{u.tracks}</td>
                <td>{ms(u.rtt_ms)}</td>
                <td>
                  {u.in_flight} / {u.ok} / {u.failed} / {u.timed_out}
                </td>
                <td>{u.request_ms ? `${ms(u.request_ms.p50)} / ${ms(u.request_ms.p95)}` : '–'}</td>
                <td>
                  {u.last_error
                    ? `${u.last_error.request} ${u.last_error.code} ${u.last_error.reason} (${ago(u.last_error.at_ms)})`
                    : '–'}
                </td>
              </tr>
            ))}
          </tbody>
        </Table>
        {upstreams.length === 0 && <p class="mt-2 text-xs text-neutral-500">No upstreams.</p>}
      </Card>
      <div class="grid gap-4 lg:grid-cols-2">
        <Card title="Announced namespaces">
          <Table>
            <thead>
              <tr>
                <th>Namespace</th>
                <th>Announced by</th>
              </tr>
            </thead>
            <tbody>
              {namespaces.announced.map(n => (
                <tr key={n.namespace}>
                  <td>{n.namespace}</td>
                  <td>
                    {n.announced_by.map((id, i) => (
                      <span key={id}>
                        {i > 0 && ', '}
                        <ConnLink id={id} />
                      </span>
                    ))}
                  </td>
                </tr>
              ))}
            </tbody>
          </Table>
        </Card>
        <Card title="Namespace subscriptions">
          <Table>
            <thead>
              <tr>
                <th>Prefix</th>
                <th>Connection</th>
                <th>Kind</th>
              </tr>
            </thead>
            <tbody>
              {namespaces.subscriptions.map(s => (
                <tr key={`${s.conn_id}-${s.prefix}-${s.kind}`}>
                  <td>{s.prefix}</td>
                  <td>
                    <ConnLink id={s.conn_id} />
                  </td>
                  <td>{s.kind}</td>
                </tr>
              ))}
            </tbody>
          </Table>
        </Card>
      </div>
      {(clusterInfo.hop_id !== null || clusterInfo.peers.length > 0) && (
        <Card
          title={`Cluster${clusterInfo.hop_id !== null ? ` (this relay's hop id ${clusterInfo.hop_id})` : ''}`}
        >
          <p class="mb-2 text-xs text-neutral-500">
            Configured peers: {clusterInfo.configured_peers.join(', ') || 'none'}
          </p>
          {clusterInfo.peers.map(p => (
            <div key={p.conn_id} class="mb-3">
              <div class="text-sm">
                Peer <ConnLink id={p.conn_id} /> {p.remote}, hop id {p.hop_id ?? '–'}, cost{' '}
                {p.relay_cost}
              </div>
              <Table>
                <thead>
                  <tr>
                    <th>Namespace</th>
                    <th>Source</th>
                    <th>Hop path</th>
                    <th>Cost</th>
                    <th>Generation</th>
                  </tr>
                </thead>
                <tbody>
                  {p.adverts.map(a => (
                    <tr key={a.namespace}>
                      <td>{a.namespace}</td>
                      <td>{a.source_conn_id}</td>
                      <td>{a.hop_path.join(' → ')}</td>
                      <td>{a.cost}</td>
                      <td>{a.generation}</td>
                    </tr>
                  ))}
                </tbody>
              </Table>
            </div>
          ))}
        </Card>
      )}
    </div>
  );
}

export function Zones() {
  const { api, refreshMs } = useDashboard();
  const polled = usePoll(signal => api.get<ZoneReport[]>('/zones', undefined, signal), refreshMs, [
    api,
  ]);
  if (polled.error && !polled.data) return <ErrorNote error={polled.error} />;
  if (!polled.data) return <Loading />;
  return (
    <Card title="Accounting zones">
      {polled.data.length === 0 ? (
        <p class="text-xs text-neutral-500">
          No zones. List namespace prefixes in the relay's <code>accounting_zones</code> setting to
          count traffic per prefix.
        </p>
      ) : (
        <Table>
          <thead>
            <tr>
              <th>Zone</th>
              <th>Tracks</th>
              <th>Publishers</th>
              <th>Subscribers</th>
              <th>Objects in / out</th>
              <th>Bytes in / out</th>
              <th>Requests ok / error</th>
            </tr>
          </thead>
          <tbody>
            {polled.data.map(z => (
              <tr key={z.zone}>
                <td>{z.zone}</td>
                <td>{z.tracks}</td>
                <td>{z.publishers}</td>
                <td>{z.subscribers}</td>
                <td>
                  {count(z.objects_in)} / {count(z.objects_out)}
                </td>
                <td>
                  {bytes(z.bytes_in)} / {bytes(z.bytes_out)}
                </td>
                <td>
                  {z.requests_ok} / {z.requests_error}
                </td>
              </tr>
            ))}
          </tbody>
        </Table>
      )}
    </Card>
  );
}
