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

import type { ConnectionDetail, Namespaces } from '../api/types';
import { Actions } from '../components/Actions';
import { Subscriptions } from '../components/Subscriptions';
import { Timeline } from '../components/Timeline';
import { Badge, Card, ErrorNote, Facts, Loading, Table } from '../components/ui';
import { hasFeature, useDashboard } from '../context';
import { ago, bytes, clock, count, ms } from '../lib/format';
import { usePoll } from '../lib/poll';
import { href } from '../lib/route';
import { stateTone } from './Connections';

export function Connection(props: { id: string }) {
  const dashboard = useDashboard();
  const { api, refreshMs } = dashboard;
  const polled = usePoll(
    async signal => {
      const [detail, namespaces] = await Promise.all([
        api.get<ConnectionDetail>(`/connections/${props.id}`, undefined, signal),
        api.get<Namespaces>('/namespaces', undefined, signal),
      ]);
      // SUBSCRIBE_NAMESPACE and SUBSCRIBE_TRACKS are listed relay-wide; keep this
      // connection's.
      const id = Number(props.id);
      return { detail, namespaceSubs: namespaces.subscriptions.filter(s => s.conn_id === id) };
    },
    refreshMs,
    [api, props.id],
  );

  if (polled.error && !polled.data) return <ErrorNote error={polled.error} />;
  if (!polled.data) return <Loading />;
  const { detail: c, namespaceSubs } = polled.data;
  const open = c.state !== 'closed';

  return (
    <div class="space-y-4">
      <div class="flex flex-wrap items-center gap-2">
        <h1 class="text-lg font-semibold">Connection {c.conn_id}</h1>
        <Badge tone={stateTone(c.state)}>{c.state}</Badge>
        {c.role === 'cluster_peer' && <Badge>cluster peer</Badge>}
        {open && hasFeature(dashboard, 'events') && (
          <a class="btn ml-auto" href={href('events', c.conn_id)}>
            Live events of this connection
          </a>
        )}
      </div>
      {open && dashboard.index?.features.includes('actions') && <Actions connId={c.conn_id} />}
      <div class="grid gap-4 lg:grid-cols-2">
        <Card title="Connection">
          <Facts
            rows={[
              ['Remote', c.remote],
              ['Transport', c.transport],
              ['Implementation', c.implementation ?? '–'],
              ['Connected', `${clock(c.connected_at_ms)} (${ago(c.connected_at_ms)})`],
              [
                'Control messages',
                `${count(c.control_messages.received)} received, ${count(c.control_messages.sent)} sent`,
              ],
              [
                'Parse errors',
                c.control_messages.parse_errors ? (
                  <Badge tone="bad">{c.control_messages.parse_errors}</Badge>
                ) : (
                  '0'
                ),
              ],
              [
                'Requests',
                `${c.requests.ok} ok, ${c.requests.error} refused, ${c.requests.in_flight} waiting`,
              ],
              ...(c.close
                ? ([
                    [
                      'Closed',
                      `${clock(c.close.at_ms)} by ${c.close.by}${c.close.code !== null ? `, code ${c.close.code}` : ''}: ${c.close.reason}`,
                    ],
                  ] as [string, string][])
                : []),
            ]}
          />
        </Card>
        <Card title="QUIC">
          <Facts
            rows={[
              ['RTT', ms(c.quic.rtt_ms)],
              ['Congestion window', bytes(c.quic.cwnd)],
              ['Congestion events', count(c.quic.congestion_events)],
              ['Lost', `${count(c.quic.lost_packets)} packets, ${bytes(c.quic.lost_bytes)}`],
              ['Sent', `${bytes(c.quic.sent_bytes)} in ${count(c.quic.sent_datagrams)} datagrams`],
              [
                'Received',
                `${bytes(c.quic.received_bytes)} in ${count(c.quic.received_datagrams)} datagrams`,
              ],
            ]}
          />
        </Card>
        <Card title="SETUP options">
          <Facts rows={c.setup_options.map(o => [o.name, o.value] as [string, string])} />
          {c.setup_options.length === 0 && <p class="text-xs text-neutral-500">None.</p>}
        </Card>
        <Card title="Publishes">
          <Facts
            rows={[
              ['Announced namespaces', c.announced_namespaces.join(', ') || '–'],
              ['Published tracks', c.published_tracks.map(t => t.track).join(', ') || '–'],
            ]}
          />
        </Card>
      </div>
      <Card title={`Track subscriptions · ${c.subscriptions.length}`}>
        <Subscriptions subscriptions={c.subscriptions} showTrack />
      </Card>
      <Card title={`Namespace subscriptions · ${namespaceSubs.length}`}>
        {namespaceSubs.length === 0 ? (
          <p class="text-xs text-neutral-500">
            No SUBSCRIBE_NAMESPACE or SUBSCRIBE_TRACKS from this connection.
          </p>
        ) : (
          <Table>
            <thead>
              <tr>
                <th>Prefix</th>
                <th>Request</th>
              </tr>
            </thead>
            <tbody>
              {namespaceSubs.map(s => (
                <tr key={`${s.prefix}-${s.kind}`}>
                  <td>{s.prefix || '(everything)'}</td>
                  <td>
                    <Badge tone={s.kind === 'tracks' ? 'ok' : 'plain'}>
                      {s.kind === 'tracks' ? 'SUBSCRIBE_TRACKS' : 'SUBSCRIBE_NAMESPACE'}
                    </Badge>
                  </td>
                </tr>
              ))}
            </tbody>
          </Table>
        )}
        <p class="mt-2 text-xs text-neutral-500">
          For SUBSCRIBE_TRACKS the relay pushes each matching track to this connection with PUBLISH;
          those appear in the message timeline.
        </p>
      </Card>
      {hasFeature(dashboard, 'qlog') && <Timeline connId={c.conn_id} open={open} />}
    </div>
  );
}
