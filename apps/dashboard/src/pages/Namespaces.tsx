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

import type { Namespaces as NamespacesData, Page, TrackInfo } from '../api/types';
import { Badge, Card, ErrorNote, Loading, Table } from '../components/ui';
import { useDashboard } from '../context';
import { usePoll } from '../lib/poll';
import { href } from '../lib/route';

/** Whether `namespace` (`a/b/c`) falls under `prefix` (`a/b`), field by field. */
export function underPrefix(namespace: string, prefix: string): boolean {
  return prefix === '' || namespace === prefix || namespace.startsWith(`${prefix}/`);
}

interface Row {
  namespace: string;
  announcedBy: number[];
  tracks: TrackInfo[];
  /** Connections holding a namespace subscription that covers this namespace. */
  watchedBy: number[];
}

/** Every namespace the relay knows, from announcements and from published tracks. */
export function namespaceRows(data: NamespacesData, tracks: TrackInfo[]): Row[] {
  const rows = new Map<string, Row>();
  const row = (namespace: string) => {
    let r = rows.get(namespace);
    if (!r) {
      r = { namespace, announcedBy: [], tracks: [], watchedBy: [] };
      rows.set(namespace, r);
    }
    return r;
  };
  for (const a of data.announced) row(a.namespace).announcedBy.push(...a.announced_by);
  for (const t of tracks) row(t.namespace).tracks.push(t);
  for (const r of rows.values()) {
    const watchers = data.subscriptions.filter(s => underPrefix(r.namespace, s.prefix));
    r.watchedBy = [...new Set(watchers.map(s => s.conn_id))];
  }
  return [...rows.values()].sort((a, b) => a.namespace.localeCompare(b.namespace));
}

function ConnLinks(props: { ids: number[] }) {
  if (props.ids.length === 0) return <span class="text-neutral-600">–</span>;
  return (
    <>
      {props.ids.map((id, i) => (
        <span key={id}>
          {i > 0 && ', '}
          <a class="link" href={href('connections', id)}>
            {id}
          </a>
        </span>
      ))}
    </>
  );
}

const KIND_LABEL: Record<string, string> = {
  namespace: 'SUBSCRIBE_NAMESPACE',
  tracks: 'SUBSCRIBE_TRACKS',
};

export function Namespaces() {
  const { api, refreshMs } = useDashboard();
  const polled = usePoll(
    async signal => {
      const [namespaces, tracks] = await Promise.all([
        api.get<NamespacesData>('/namespaces', undefined, signal),
        api.get<Page<TrackInfo>>('/tracks', { limit: 1000 }, signal),
      ]);
      return { namespaces, tracks: tracks.items };
    },
    refreshMs,
    [api],
  );
  if (polled.error && !polled.data) return <ErrorNote error={polled.error} />;
  if (!polled.data) return <Loading />;
  const { namespaces, tracks } = polled.data;
  const rows = namespaceRows(namespaces, tracks);

  return (
    <div class="space-y-4">
      <Card title="Namespaces">
        <p class="mb-3 text-xs text-neutral-500">
          Namespaces announced with PUBLISH_NAMESPACE and namespaces of published tracks. A
          namespace subscription covers every namespace under its prefix.
        </p>
        <Table>
          <thead>
            <tr>
              <th>Namespace</th>
              <th>Announced by</th>
              <th>Tracks</th>
              <th>Subscribers</th>
              <th>Covered by namespace subscriptions of</th>
            </tr>
          </thead>
          <tbody>
            {rows.map(r => (
              <tr key={r.namespace}>
                <td>{r.namespace}</td>
                <td>
                  <ConnLinks ids={r.announcedBy} />
                </td>
                <td class="whitespace-normal">
                  {r.tracks.length === 0 ? (
                    <span class="text-neutral-600">–</span>
                  ) : (
                    r.tracks.map((t, i) => (
                      <span key={t.track_id}>
                        {i > 0 && ', '}
                        <a class="link" href={href('tracks', t.track_id)}>
                          {t.name}
                        </a>
                      </span>
                    ))
                  )}
                </td>
                <td>{r.tracks.reduce((sum, t) => sum + t.subscribers, 0)}</td>
                <td>
                  <ConnLinks ids={r.watchedBy} />
                </td>
              </tr>
            ))}
          </tbody>
        </Table>
        {rows.length === 0 && <p class="mt-2 text-xs text-neutral-500">No namespaces.</p>}
      </Card>
      <Card title="Namespace subscriptions">
        <Table>
          <thead>
            <tr>
              <th>Prefix</th>
              <th>Request</th>
              <th>Connection</th>
              <th>Namespaces covered</th>
            </tr>
          </thead>
          <tbody>
            {namespaces.subscriptions.map(s => (
              <tr key={`${s.conn_id}-${s.prefix}-${s.kind}`}>
                <td>{s.prefix || '(everything)'}</td>
                <td>
                  <Badge tone={s.kind === 'tracks' ? 'ok' : 'plain'}>
                    {KIND_LABEL[s.kind] ?? s.kind}
                  </Badge>
                </td>
                <td>
                  <ConnLinks ids={[s.conn_id]} />
                </td>
                <td>{rows.filter(r => underPrefix(r.namespace, s.prefix)).length}</td>
              </tr>
            ))}
          </tbody>
        </Table>
        {namespaces.subscriptions.length === 0 && (
          <p class="mt-2 text-xs text-neutral-500">
            No SUBSCRIBE_NAMESPACE or SUBSCRIBE_TRACKS held.
          </p>
        )}
      </Card>
    </div>
  );
}
