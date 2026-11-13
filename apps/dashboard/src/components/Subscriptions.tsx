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

import type { SubscriptionInfo } from '../api/types';
import { Table } from './ui';
import { bytes, ms } from '../lib/format';
import { href } from '../lib/route';

/** Subscriptions with their progress and processing time. `showConn` adds the
 *  subscriber's connection, for a track's list. */
export function Subscriptions(props: {
  subscriptions: SubscriptionInfo[];
  showConn?: boolean;
  showTrack?: boolean;
}) {
  if (!props.subscriptions.length) return <p class="text-xs text-neutral-500">None.</p>;
  return (
    <Table>
      <thead>
        <tr>
          {props.showConn && <th>Connection</th>}
          <th>Request</th>
          {props.showTrack && <th>Track</th>}
          <th>Alias</th>
          <th>Filter</th>
          <th>Forward</th>
          <th>Priority</th>
          <th>Last sent</th>
          <th>Streams</th>
          <th>Queued</th>
          <th>Processing p50 / p99</th>
        </tr>
      </thead>
      <tbody>
        {props.subscriptions.map(s => (
          <tr key={`${s.conn_id ?? ''}-${s.request_id}`}>
            {props.showConn && (
              <td>
                <a class="link" href={href('connections', s.conn_id!)}>
                  {s.conn_id}
                </a>
              </td>
            )}
            <td>{s.request_id}</td>
            {props.showTrack && (
              <td>
                <a class="link" href={href('tracks', s.track_id)}>
                  {s.track}
                </a>
              </td>
            )}
            <td>{s.track_alias}</td>
            <td>{s.filter}</td>
            <td>{s.forward ? 'yes' : 'no'}</td>
            <td>{s.subscriber_priority}</td>
            <td>{s.last_sent ?? '–'}</td>
            <td>{s.streams_opened}</td>
            <td>{bytes(s.queued_bytes)}</td>
            <td>
              {s.processing_ms ? `${ms(s.processing_ms.p50)} / ${ms(s.processing_ms.p99)}` : '–'}
            </td>
          </tr>
        ))}
      </tbody>
    </Table>
  );
}
