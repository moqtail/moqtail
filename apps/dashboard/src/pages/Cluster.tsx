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

import type { ClusterInfo } from '../api/types';
import { Badge, Card, ErrorNote, Facts, Loading, Table } from '../components/ui';
import { useDashboard } from '../context';
import { ago } from '../lib/format';
import { usePoll } from '../lib/poll';
import { href } from '../lib/route';

function ConnLink(props: { id: number | null }) {
  if (props.id === null) return <span class="text-neutral-600">–</span>;
  return (
    <a class="link" href={href('connections', props.id)}>
      {props.id}
    </a>
  );
}

/** The relays a route passed, with this relay's own hop id marked. */
function HopPath(props: { hops: number[]; self: number | null }) {
  if (props.hops.length === 0) return <span class="text-neutral-600">local</span>;
  return (
    <>
      {props.hops.map((h, i) => (
        <span key={i}>
          {i > 0 && <span class="text-neutral-600"> → </span>}
          <span class={h === props.self ? 'text-blue-300' : ''}>{h}</span>
        </span>
      ))}
    </>
  );
}

/** Whether the relay is part of a mesh at all. */
export function isClustered(c: ClusterInfo): boolean {
  return c.hop_id !== null || c.configured_peers.length > 0 || c.peers.length > 0;
}

export function Cluster() {
  const { api, refreshMs } = useDashboard();
  const polled = usePoll(signal => api.get<ClusterInfo>('/cluster', undefined, signal), refreshMs, [
    api,
  ]);
  if (polled.error && !polled.data) return <ErrorNote error={polled.error} />;
  if (!polled.data) return <Loading />;
  const c = polled.data;

  if (!isClustered(c)) {
    return (
      <Card title="Cluster">
        <p class="text-xs text-neutral-400">
          This relay is not part of a mesh. To join one, give it a <code>hop_id</code> and list the
          relays it should dial in <code>peers</code>:
        </p>
        <pre class="mt-3 rounded-lg border border-white/6 bg-neutral-950 p-3 font-mono text-xs text-neutral-300">
          {`hop_id: 1
peers:
  - moqt://relay-2.example:4433
  - moqt://relay-3.example:4433`}
        </pre>
        <p class="mt-3 text-xs text-neutral-500">
          One link carries advertisements both ways, so two relays need only one of them to list the
          other.
        </p>
      </Card>
    );
  }

  const up = c.links.filter(l => l.up).length;
  const adverts = c.peers.reduce((n, p) => n + p.adverts.length, 0);

  return (
    <div class="space-y-4">
      <Card title="This relay">
        <Facts
          rows={[
            ['Hop id', c.hop_id === null ? 'none (not identified in the mesh)' : String(c.hop_id)],
            ['Peers it dials', `${up} of ${c.links.length} up`],
            ['Peer sessions', `${c.peers.length}, dialed by either side`],
            ['Namespaces advertised', `${adverts}, over all peers`],
          ]}
        />
      </Card>
      <Card title={`Peers this relay dials · ${c.links.length}`}>
        {c.links.length === 0 ? (
          <p class="text-xs text-neutral-500">
            None; other relays dial this one. Their sessions are listed below.
          </p>
        ) : (
          <Table>
            <thead>
              <tr>
                <th>Peer</th>
                <th>Link</th>
                <th>Since</th>
                <th>Remote</th>
                <th>Connection</th>
                <th>Last error</th>
              </tr>
            </thead>
            <tbody>
              {c.links.map(l => (
                <tr key={l.url}>
                  <td>{l.url}</td>
                  <td>
                    <Badge tone={l.up ? 'ok' : 'bad'}>{l.up ? 'up' : 'down'}</Badge>
                  </td>
                  <td>{l.since_ms === null ? 'dialing' : ago(l.since_ms)}</td>
                  <td>{l.remote ?? '–'}</td>
                  <td>
                    <ConnLink id={l.conn_id} />
                  </td>
                  <td class="whitespace-normal text-neutral-400">{l.last_error ?? '–'}</td>
                </tr>
              ))}
            </tbody>
          </Table>
        )}
      </Card>
      {c.peers.length === 0 ? (
        <Card title="Peer sessions">
          <p class="text-xs text-neutral-500">No session with another relay is open.</p>
        </Card>
      ) : (
        c.peers.map(p => (
          <Card
            key={p.conn_id}
            title={`Peer hop ${p.hop_id ?? '–'} · ${p.remote} · ${p.adverts.length} namespaces advertised`}
          >
            <p class="mb-2 text-xs text-neutral-500">
              Connection <ConnLink id={p.conn_id} />, relay cost {p.relay_cost}. The routes this
              relay advertises to it, each the best one that did not already pass through it:
            </p>
            {p.adverts.length === 0 ? (
              <p class="text-xs text-neutral-500">Nothing advertised.</p>
            ) : (
              <Table>
                <thead>
                  <tr>
                    <th>Namespace</th>
                    <th>Route came from</th>
                    <th>Hop path</th>
                    <th>Cost</th>
                    <th>Generation</th>
                  </tr>
                </thead>
                <tbody>
                  {p.adverts.map(a => (
                    <tr key={a.namespace}>
                      <td>{a.namespace}</td>
                      <td>
                        <ConnLink id={a.source_conn_id} />
                      </td>
                      <td>
                        <HopPath hops={a.hop_path} self={c.hop_id} />
                      </td>
                      <td>{a.cost}</td>
                      <td>{a.generation}</td>
                    </tr>
                  ))}
                </tbody>
              </Table>
            )}
          </Card>
        ))
      )}
    </div>
  );
}
