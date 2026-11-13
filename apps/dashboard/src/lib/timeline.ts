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

// The message timeline of one connection, built from its qlog events the same way
// scripts/qlog-summary.py does: a request is grouped with the responses that came
// back on its stream, and data streams are counted per track.

import type { QlogEvent } from '../api/types';

const REQUESTS = new Set([
  'subscribe',
  'publish',
  'fetch',
  'track_status',
  'publish_namespace',
  'subscribe_namespace',
  'subscribe_tracks',
]);

type Field = { value?: string; value_bytes?: string };
type Message = Record<string, unknown> & { type: string };

/** `<-` arrived from the peer, `->` was sent to it. */
export type Direction = '<-' | '->';

export interface TimelineMessage {
  /** Seconds since the trace's first event. */
  at: number;
  direction: Direction;
  streamId: number;
  type: string;
  /** A one-line description: type, track, ids, codes. */
  text: string;
  hex: string;
}

export type TimelineEntry =
  | { kind: 'request'; request: TimelineMessage; responses: TimelineMessage[] }
  | { kind: 'message'; message: TimelineMessage }
  | { kind: 'error'; at: number; message: string; hex: string }
  /** A data stream cut short: RESET_STREAM or STOP_SENDING, with its error code. */
  | {
      kind: 'stream_end';
      at: number;
      direction: Direction;
      streamId: number;
      end: 'reset' | 'stop_sending';
      errorCode: number;
      /** The track or fetch the stream carried, when its header was seen. */
      label: string | null;
    }
  /** How the connection ended. */
  | { kind: 'close'; at: number; by: string; errorCode: number | null; reason: string };

export interface DataStreams {
  direction: Direction;
  /** `track_alias=N` or `fetch request_id=N`. */
  label: string;
  streams: number;
  /** Streams that ended with FIN. */
  fin: number;
  /** Streams cut short with RESET_STREAM or STOP_SENDING. */
  cut: number;
  groups?: { first: number; last: number; distinct: number };
}

export interface Timeline {
  peerSetup: TimelineMessage | null;
  entries: TimelineEntry[];
  data: DataStreams[];
}

function fieldText(field: Field | undefined): string {
  if (!field) return '';
  return field.value ?? `0x${field.value_bytes ?? ''}`;
}

function tupleText(fields: unknown): string {
  return Array.isArray(fields) ? fields.map(f => fieldText(f as Field)).join('/') : '';
}

/** A parameter or setup option: its name, or its type when qlog has no name for
 *  it, and its value, which is hex in `value_bytes.data` for a byte value. */
function kvpText(p: Record<string, unknown>): string {
  const name = p.name === 'unknown' ? `0x${Number(p.type).toString(16)}` : String(p.name);
  const value = p.value ?? (p.value_bytes as Record<string, unknown> | undefined)?.data;
  return value === undefined ? name : `${name}=${valueText(value)}`;
}

function valueText(value: unknown): string {
  if (value && typeof value === 'object' && !Array.isArray(value)) {
    const v = value as Record<string, unknown>;
    if ('group' in v && 'object' in v && Object.keys(v).length === 2)
      return `${v.group}:${v.object}`;
    return Object.entries(v)
      .map(([k, x]) => `${k}=${valueText(x)}`)
      .join(',');
  }
  if (Array.isArray(value)) return tupleText(value);
  return String(value);
}

export function describe(m: Message): string {
  const parts = [m.type.toUpperCase()];
  for (const key of ['track_namespace', 'track_namespace_prefix', 'track_namespace_suffix']) {
    if (key in m) {
      let ns = tupleText(m[key]);
      if ('track_name' in m) ns += ` : ${fieldText(m.track_name as Field)}`;
      parts.push(ns);
    }
  }
  const standalone = m.standalone_fetch as Record<string, unknown> | undefined;
  if (standalone) {
    parts.push(
      `${tupleText(standalone.track_namespace)} : ${fieldText(standalone.track_name as Field)}`,
    );
  }
  for (const key of ['request_id', 'track_alias', 'error_code', 'status_code', 'stream_count']) {
    if (key in m) parts.push(`${key}=${String(m[key])}`);
  }
  if (typeof m.reason_phrase === 'string' && m.reason_phrase) parts.push(`'${m.reason_phrase}'`);
  const params = m.parameters as Record<string, unknown>[] | undefined;
  if (params?.length) {
    parts.push(`params[${params.map(kvpText).join(', ')}]`);
  }
  if (m.type === 'setup' && Array.isArray(m.setup_options)) {
    const text = (m.setup_options as Record<string, unknown>[]).map(kvpText);
    parts.push(text.join(', ') || '(no options)');
  }
  return parts.join(' ');
}

export function buildTimeline(events: QlogEvent[]): Timeline {
  const timed = events.filter(e => typeof e.time === 'number');
  const start = timed[0]?.time ?? 0;
  let peerSetup: TimelineMessage | null = null;
  const entries: TimelineEntry[] = [];
  const openRequests = new Map<number, Extract<TimelineEntry, { kind: 'request' }>>();
  const data = new Map<string, DataStreams & { groupIds?: Set<number> }>();
  // Which data group each stream belongs to, from its header.
  const streamKeys = new Map<string, string>();

  for (const event of timed) {
    const at = (event.time - start) / 1000;
    const direction: Direction = event.name.endsWith('_parsed') ? '<-' : '->';
    const d = event.data;
    if (event.name.startsWith('moqt:control_message')) {
      const m = d.message as Message;
      const message: TimelineMessage = {
        at,
        direction,
        streamId: Number(d.stream_id),
        type: m.type,
        text: describe(m),
        hex: String((d.raw as { data?: string } | undefined)?.data ?? ''),
      };
      if (m.type === 'setup') {
        if (direction === '<-') peerSetup = message;
        continue;
      }
      const open = openRequests.get(message.streamId);
      if (REQUESTS.has(m.type)) {
        const entry = { kind: 'request' as const, request: message, responses: [] };
        openRequests.set(message.streamId, entry);
        entries.push(entry);
      } else if (open) {
        open.responses.push(message);
      } else {
        entries.push({ kind: 'message', message });
      }
    } else if (event.name.startsWith('moqt:subgroup_header')) {
      const key = `${direction} alias ${String(d.track_alias)}`;
      const entry = data.get(key) ?? {
        direction,
        label: `track_alias=${String(d.track_alias)}`,
        streams: 0,
        fin: 0,
        cut: 0,
        groupIds: new Set<number>(),
      };
      entry.streams += 1;
      streamKeys.set(`${direction} ${String(d.stream_id)}`, key);
      entry.groupIds!.add(Number(d.group_id));
      data.set(key, entry);
    } else if (event.name.startsWith('moqt:fetch_header')) {
      const key = `${direction} fetch ${String(d.request_id)}`;
      const entry = data.get(key) ?? {
        direction,
        label: `fetch request_id=${String(d.request_id)}`,
        streams: 0,
        fin: 0,
        cut: 0,
      };
      entry.streams += 1;
      streamKeys.set(`${direction} ${String(d.stream_id)}`, key);
      data.set(key, entry);
    } else if (event.name.startsWith('moqtail:stream_end')) {
      // STOP_SENDING travels against the data: the stream it names is the other
      // side's, so its header was seen in the opposite direction.
      const end = String(d.end);
      const dataDirection: Direction =
        end === 'stop_sending' ? (direction === '->' ? '<-' : '->') : direction;
      const key = streamKeys.get(`${dataDirection} ${String(d.stream_id)}`);
      const group = key === undefined ? undefined : data.get(key);
      if (end === 'fin') {
        if (group) group.fin += 1;
      } else {
        if (group) group.cut += 1;
        entries.push({
          kind: 'stream_end',
          at,
          direction,
          streamId: Number(d.stream_id),
          end: end === 'reset' ? 'reset' : 'stop_sending',
          errorCode: Number(d.error_code),
          label: group?.label ?? null,
        });
      }
    } else if (event.name === 'moqtail:connection_closed') {
      entries.push({
        kind: 'close',
        at,
        by: String(d.by),
        errorCode: typeof d.error_code === 'number' ? d.error_code : null,
        reason: String(d.reason ?? ''),
      });
    } else if (event.name === 'loglevel:error') {
      entries.push({
        kind: 'error',
        at,
        message: String(d.message ?? 'parse error'),
        hex: String((d.raw as { data?: string } | undefined)?.data ?? ''),
      });
    }
  }

  const dataStreams: DataStreams[] = [...data.values()].map(({ groupIds, ...entry }) => {
    if (!groupIds?.size) return entry;
    const sorted = [...groupIds].sort((a, b) => a - b);
    return {
      ...entry,
      groups: { first: sorted[0], last: sorted[sorted.length - 1], distinct: sorted.length },
    };
  });
  return { peerSetup, entries, data: dataStreams };
}
