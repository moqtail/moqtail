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

// The shapes of the relay API's responses under /api/v1. They follow the Rust
// types in apps/relay-api and libs/relay-hooks; a field added there is added here.

export interface ApiErrorBody {
  error: string;
  detail?: string;
}

export interface IndexEntry {
  method: string;
  path: string;
  feature: string;
  enabled: boolean;
}

export interface ApiIndex {
  version: string;
  features: string[];
  endpoints: IndexEntry[];
}

export interface Page<T> {
  total: number;
  offset: number;
  limit: number;
  items: T[];
}

/** Percentiles in milliseconds; a value is the upper bound of its histogram bucket. */
export interface Summary {
  count: number;
  p50: number | null;
  p95: number | null;
  p99: number | null;
  max: number | null;
}

export interface ResultCounts {
  ok: number;
  /** Count by error code. */
  error: Record<string, number>;
}

export interface Overview {
  version: string;
  pid: number;
  started_at_ms: number;
  accepting: boolean;
  draining: boolean;
  features: string[];
  connections: Record<string, number>;
  closed_connections_kept: number;
  tracks: number;
  subscriptions: number;
  cached_groups: number;
  /** Keyed by `type/direction`. */
  requests: Record<string, ResultCounts>;
  traffic?: {
    objects_received: number;
    bytes_received: number;
    objects_sent: number;
    bytes_sent: number;
    objects_dropped: number;
  };
}

export interface QuicStats {
  rtt_ms: number;
  cwnd: number;
  congestion_events: number;
  lost_packets: number;
  lost_bytes: number;
  sent_bytes: number;
  received_bytes: number;
  sent_datagrams: number;
  received_datagrams: number;
}

export interface CloseInfo {
  at_ms: number;
  by: string;
  code: number | null;
  reason: string;
}

export interface MessageCounts {
  received: number;
  sent: number;
  parse_errors: number;
}

export interface RequestCounts {
  in_flight: number;
  ok: number;
  error: number;
}

export type ConnectionState = 'active' | 'idle' | 'draining' | 'closed';

export interface ConnectionSummary {
  conn_id: number;
  state: ConnectionState;
  transport: string;
  remote: string;
  role: 'client' | 'cluster_peer';
  connected_at_ms: number;
  implementation: string | null;
  announced_namespaces: number;
  published_tracks: number;
  subscriptions: number;
  quic: QuicStats;
  close: CloseInfo | null;
  control_messages: MessageCounts;
  requests: RequestCounts;
}

export interface SubscriptionInfo {
  request_id: number;
  track: string;
  track_id: number;
  track_alias: number;
  forward: boolean;
  subscriber_priority: number;
  group_order: string;
  filter: string;
  start: string | null;
  end_group: number | null;
  last_sent: string | null;
  last_received: string | null;
  streams_opened: number;
  queued_bytes: number;
  processing_ms?: Summary;
  /** Set in a track's subscription list. */
  conn_id?: number;
}

export interface ConnectionDetail extends Omit<
  ConnectionSummary,
  'announced_namespaces' | 'published_tracks' | 'subscriptions'
> {
  setup_options: { name: string; value: string }[];
  announced_namespaces: string[];
  published_tracks: { request_id: number; track: string }[];
  subscriptions: SubscriptionInfo[];
}

export interface TrackInfo {
  track_id: number;
  namespace: string;
  name: string;
  origin: string;
  status: string;
  publishers: { conn_id: number; alias: number }[];
  subscribers: number;
  forwarding: boolean;
  largest_location: string | null;
  cached_groups: number;
  upstream_forward: boolean;
  processing_ms?: Summary;
}

export interface TrackDetail extends TrackInfo {
  subscriptions: SubscriptionInfo[];
  cached_group_ids: number[];
  cached_bytes: number;
}

export interface Namespaces {
  announced: { namespace: string; announced_by: number[] }[];
  subscriptions: { prefix: string; conn_id: number; kind: string }[];
}

export interface Upstream {
  conn_id: number;
  kind: 'publisher' | 'cluster_peer';
  up: boolean;
  remote: string;
  tracks: number;
  announced_namespaces: number;
  rtt_ms: number;
  in_flight: number;
  ok: number;
  failed: number;
  timed_out: number;
  last_error: { at_ms: number; request: string; code: number; reason: string } | null;
  request_ms: { p50: number; p95: number } | null;
}

export interface ZoneReport {
  zone: string;
  tracks: number;
  publishers: number;
  subscribers: number;
  objects_in: number;
  bytes_in: number;
  objects_out: number;
  bytes_out: number;
  requests_ok: number;
  requests_error: number;
}

/** The link to one peer this relay dials. */
export interface PeerLink {
  url: string;
  up: boolean;
  remote: string | null;
  conn_id: number | null;
  /** When the link last went up or down; null before the first dial finished. */
  since_ms: number | null;
  last_error: string | null;
}

export interface ClusterInfo {
  hop_id: number | null;
  configured_peers: string[];
  links: PeerLink[];
  peers: {
    conn_id: number;
    remote: string;
    hop_id: number | null;
    relay_cost: number;
    adverts: {
      namespace: string;
      source_conn_id: number;
      hop_path: number[];
      cost: number;
      generation: number;
    }[];
  }[];
}

export interface LatencyReport {
  window: string;
  covered_s: number | null;
  stages_ms: { ingest: Summary; queue: Summary; write: Summary; total: Summary };
  first_object_ms: Summary;
  slowest_tracks: ({ track_id: number; track: string | null } & Summary)[];
  slowest_subscriptions: ({ conn_id: number; request_id: number; track_id: number } & Summary)[];
}

export interface SlowEntry {
  at_ms: number;
  kind: 'object' | 'first_object';
  total_ms: number;
  stages_ms?: { ingest: number; queue: number; write: number };
  track_id: number;
  conn_id: number;
  request_id: number;
  location?: string;
}

export interface Slowlog {
  threshold_ms: number;
  first_object_threshold_ms: number;
  dropped: number;
  entries: SlowEntry[];
}

export interface TraceFile {
  conn_id: number | null;
  kind: 'moqt' | 'quic';
  file: string;
  bytes: number;
  modified_at_ms: number;
  open: boolean;
}

/** One qlog event, as written to a trace file and sent by /events. */
export interface QlogEvent {
  time: number;
  name: string;
  data: Record<string, unknown>;
  /** Set on live events, which come from every connection. */
  conn_id?: number;
}
