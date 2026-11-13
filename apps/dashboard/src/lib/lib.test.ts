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

import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import type { QlogEvent } from '../api/types';
import { bytes, duration, ms } from './format';
import { JsonSeqParser, SseParser } from './stream';
import { buildTimeline } from './timeline';
import { namespaceRows, underPrefix } from '../pages/Namespaces';

describe('JsonSeqParser', () => {
  it('returns records only once they are complete, across pieces', () => {
    const parser = new JsonSeqParser();
    expect(parser.push('\u001e{"a":1}\n\u001e{"b"')).toEqual([{ a: 1 }]);
    expect(parser.push(':2}\n')).toEqual([{ b: 2 }]);
  });

  it('skips a record that does not parse', () => {
    expect(new JsonSeqParser().push('\u001e{oops\n\u001e{"c":3}\n')).toEqual([{ c: 3 }]);
  });
});

describe('SseParser', () => {
  it('splits messages, joins data lines and ignores keep-alives', () => {
    const parser = new SseParser();
    expect(parser.push(':\n\nevent: qlog\ndata: {"x"')).toEqual([]);
    expect(parser.push(':1}\r\n\r\ndata: a\ndata: b\n\n')).toEqual([
      { event: 'qlog', data: '{"x":1}' },
      { event: 'message', data: 'a\nb' },
    ]);
  });
});

describe('format', () => {
  it('formats sizes, durations and milliseconds', () => {
    expect(bytes(999)).toBe('999 B');
    expect(bytes(1500)).toBe('1.5 KB');
    expect(duration(3725)).toBe('1h 2m');
    expect(ms(null)).toBe('–');
    expect(ms(0.256)).toBe('0.26 ms');
    expect(ms(250.4)).toBe('250 ms');
  });
});

describe('buildTimeline', () => {
  const control = (
    time: number,
    name: string,
    streamId: number,
    message: Record<string, unknown>,
  ): QlogEvent => ({
    time,
    name,
    data: { stream_id: streamId, message },
  });

  it('groups responses under their request and counts data streams', () => {
    const timeline = buildTimeline([
      control(1000, 'moqt:control_message_parsed', 0, { type: 'setup', setup_options: [] }),
      control(1100, 'moqt:control_message_parsed', 2, {
        type: 'subscribe',
        request_id: 0,
        track_namespace: [{ value: 'live' }],
        track_name: { value: 'video' },
      }),
      control(1200, 'moqt:control_message_created', 2, { type: 'subscribe_ok', track_alias: 5 }),
      control(1300, 'moqt:control_message_created', 0, { type: 'goaway' }),
      { time: 1400, name: 'moqt:subgroup_header_created', data: { track_alias: 5, group_id: 7 } },
      { time: 1500, name: 'moqt:subgroup_header_created', data: { track_alias: 5, group_id: 8 } },
    ]);

    expect(timeline.peerSetup?.text).toBe('SETUP (no options)');
    expect(timeline.entries).toHaveLength(2);
    const request = timeline.entries[0];
    expect(request.kind).toBe('request');
    if (request.kind === 'request') {
      expect(request.request.text).toBe('SUBSCRIBE live : video request_id=0');
      expect(request.request.at).toBeCloseTo(0.1);
      expect(request.responses.map(r => r.type)).toEqual(['subscribe_ok']);
    }
    expect(timeline.entries[1].kind).toBe('message');
    expect(timeline.data).toEqual([
      {
        direction: '->',
        label: 'track_alias=5',
        streams: 2,
        groups: { first: 7, last: 8, distinct: 2 },
      },
    ]);
  });
});

/** The timeline with snake_case keys, as the relay's Rust port writes it. */
export function snakeKeys(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(snakeKeys);
  if (value && typeof value === 'object') {
    return Object.fromEntries(
      Object.entries(value).map(([k, v]) => [
        k.replace(/[A-Z]/g, c => `_${c.toLowerCase()}`),
        snakeKeys(v),
      ]),
    );
  }
  return value;
}

describe('the trace shared with the relay', () => {
  it("gives the timeline the relay's port expects", () => {
    // The relay's Rust test reads the same two files, so the two stay in step.
    const text = readFileSync('../relay-api/testdata/timeline.sqlog', 'utf8');
    const events = new JsonSeqParser().push(text) as QlogEvent[];
    const expected: unknown = JSON.parse(
      readFileSync('../relay-api/testdata/timeline.expected.json', 'utf8'),
    );
    expect(snakeKeys(buildTimeline(events))).toEqual(expected);
  });
});

describe('namespaces', () => {
  it('matches prefixes field by field', () => {
    expect(underPrefix('live/sports', 'live')).toBe(true);
    expect(underPrefix('live', 'live')).toBe(true);
    expect(underPrefix('livestream', 'live')).toBe(false);
    expect(underPrefix('anything', '')).toBe(true);
  });

  it('joins announcements, tracks and namespace subscriptions', () => {
    const track = { namespace: 'live/a', name: 'v', track_id: 1, subscribers: 3 } as never;
    const rows = namespaceRows(
      {
        announced: [{ namespace: 'live/b', announced_by: [7] }],
        subscriptions: [{ prefix: 'live', conn_id: 9, kind: 'namespace' }],
      },
      [track],
    );
    expect(rows.map(r => [r.namespace, r.announcedBy, r.tracks.length, r.watchedBy])).toEqual([
      ['live/a', [], 1, [9]],
      ['live/b', [7], 0, [9]],
    ]);
  });
});
