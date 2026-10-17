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

import type { DataDirection, DataEvent } from 'moqtail/client';
import { FullTrackName } from 'moqtail/model';
import { trackLabel } from './describe';

/** Rates are averaged over this much recent history. */
const RATE_WINDOW_MS = 2000;
/** The RFC 3550 jitter estimator moves 1/16 of the way to each new sample. */
const JITTER_GAIN = 1 / 16;
/**
 * A latency further off than this means the timestamp is not wall-clock ms (meet writes
 * `Date.now()` at capture, other senders may not), so it is left out rather than averaged in.
 */
const MAX_PLAUSIBLE_LATENCY_MS = 60_000;

/** avg/min/max over every sample since the last reset, in ms. */
export type Summary = { count: number; avg: number; min: number; max: number };

export type TimingStats = {
  /** Time between consecutive objects of the track: interarrival in, inter-send out. */
  interval?: Summary;
  jitterMs?: number;
  /**
   * `sender`: RFC 3550, comparing arrival spacing with the senders' LOC timestamps.
   * `arrival`: no sender timestamps, so how much consecutive intervals differ.
   */
  jitterSource?: 'sender' | 'arrival';
  /** From the LOC Timestamp to this client: capture → receive in, capture → send out. */
  latency?: Summary & { last: number };
};

export type TrackStats = {
  key: string;
  direction: DataDirection;
  stream: 'subgroup' | 'fetch';
  label: string;
  streams: number;
  openStreams: number;
  resets: number;
  errors: number;
  groups: number;
  subgroups: number;
  objects: number;
  bytes: number;
  statusObjects: number;
  lastGroup?: bigint;
  lastObject?: bigint;
  /** Group IDs jumped over between the highest group seen and a newer one. */
  skippedGroups: number;
  /** Object IDs jumped over within a stream. */
  skippedObjects: number;
  objectsPerSecond: number;
  kbps: number;
  timing: TimingStats;
};

type Accumulator = { count: number; sum: number; min: number; max: number };

function accumulate(acc: Accumulator | undefined, sample: number): Accumulator {
  if (!acc) return { count: 1, sum: sample, min: sample, max: sample };
  acc.count++;
  acc.sum += sample;
  if (sample < acc.min) acc.min = sample;
  if (sample > acc.max) acc.max = sample;
  return acc;
}

function summarize(acc: Accumulator | undefined): Summary | undefined {
  return acc && { count: acc.count, avg: acc.sum / acc.count, min: acc.min, max: acc.max };
}

type TimingState = {
  lastAt?: number;
  lastInterval?: number;
  interval?: Accumulator;
  /** The last object that carried a sender timestamp, for the RFC 3550 difference. */
  lastTimed?: { at: number; senderMs: number };
  jitterMs?: number;
  jitterSource?: 'sender' | 'arrival';
  latency?: Accumulator;
  lastLatency?: number;
};

type TrackState = Omit<
  TrackStats,
  'groups' | 'subgroups' | 'label' | 'objectsPerSecond' | 'kbps' | 'timing'
> & {
  timingState: TimingState;
  /** The id the track is known by until a name is found: its alias, or a fetch's request id. */
  id: bigint;
  fullTrackName?: FullTrackName;
  groups: Set<bigint>;
  subgroups: Set<string>;
  highestGroup?: bigint;
  history: Array<{ at: number; objects: number; bytes: number }>;
};

type StreamState = { track: TrackState; lastObjectId?: bigint; isOpen: boolean };

/**
 * Counts what the data streams carry, per track and direction. Fed one {@link DataEvent}
 * at a time and read with {@link snapshot}, so the cost per object stays a few map lookups.
 */
export class DataStats {
  #tracks = new Map<string, TrackState>();
  #streams = new Map<number, StreamState>();
  /** Names learned from PUBLISH messages, for tracks whose data arrives before the client knows them. */
  #aliasNames = new Map<string, FullTrackName>();

  reset() {
    this.#tracks.clear();
    this.#streams.clear();
    this.#aliasNames.clear();
  }

  /** Records which track an alias stands for, in the direction its data will flow. */
  nameAlias(direction: DataDirection, trackAlias: bigint, fullTrackName: FullTrackName) {
    this.#aliasNames.set(`${direction}|${trackAlias}`, fullTrackName);
  }

  add(event: DataEvent) {
    if (event.kind === 'header') {
      const id = event.stream === 'fetch' ? event.requestId! : event.trackAlias!;
      const key = `${event.direction}|${event.stream}|${id}`;
      let track = this.#tracks.get(key);
      if (!track) {
        track = {
          key,
          id,
          direction: event.direction,
          stream: event.stream,
          streams: 0,
          openStreams: 0,
          resets: 0,
          errors: 0,
          groups: new Set(),
          subgroups: new Set(),
          objects: 0,
          bytes: 0,
          statusObjects: 0,
          skippedGroups: 0,
          skippedObjects: 0,
          history: [],
          timingState: {},
        };
        this.#tracks.set(key, track);
      }
      if (event.fullTrackName) track.fullTrackName = event.fullTrackName;
      track.streams++;
      track.openStreams++;
      this.#streams.set(event.streamId, { track, isOpen: true });
      return;
    }

    const stream = this.#streams.get(event.streamId);
    if (!stream) return;
    const track = stream.track;

    if (event.kind === 'object') {
      track.objects++;
      track.bytes += event.payloadLength;
      if (event.objectStatus !== undefined) track.statusObjects++;
      if (!track.groups.has(event.groupId)) {
        track.groups.add(event.groupId);
        if (track.highestGroup !== undefined && event.groupId > track.highestGroup + 1n)
          track.skippedGroups += Number(event.groupId - track.highestGroup - 1n);
        if (track.highestGroup === undefined || event.groupId > track.highestGroup)
          track.highestGroup = event.groupId;
      }
      track.subgroups.add(`${event.groupId}:${event.subgroupId ?? '?'}`);
      if (stream.lastObjectId !== undefined && event.objectId > stream.lastObjectId + 1n)
        track.skippedObjects += Number(event.objectId - stream.lastObjectId - 1n);
      stream.lastObjectId = event.objectId;
      track.lastGroup = event.groupId;
      track.lastObject = event.objectId;
      this.#time(track.timingState, event.at, event.locTimestamp);
      return;
    }

    if (stream.isOpen) {
      stream.isOpen = false;
      track.openStreams--;
    }
    if (event.reason === 'reset') track.resets++;
    else if (event.reason === 'error') track.errors++;
    this.#streams.delete(event.streamId);
  }

  #time(timing: TimingState, at: number, locTimestamp: bigint | undefined) {
    if (timing.lastAt !== undefined) {
      const interval = at - timing.lastAt;
      timing.interval = accumulate(timing.interval, interval);
      // Without sender timestamps, fall back to how much one interval differs from the last.
      if (timing.jitterSource !== 'sender' && timing.lastInterval !== undefined) {
        const d = Math.abs(interval - timing.lastInterval);
        timing.jitterMs = (timing.jitterMs ?? 0) + (d - (timing.jitterMs ?? 0)) * JITTER_GAIN;
        timing.jitterSource = 'arrival';
      }
      timing.lastInterval = interval;
    }
    timing.lastAt = at;

    if (locTimestamp === undefined) return;
    const senderMs = Number(locTimestamp);
    const latency = performance.timeOrigin + at - senderMs;
    if (Math.abs(latency) <= MAX_PLAUSIBLE_LATENCY_MS) {
      timing.latency = accumulate(timing.latency, latency);
      timing.lastLatency = latency;
    }
    if (timing.lastTimed) {
      // RFC 3550: D = (Rj - Ri) - (Sj - Si), J += (|D| - J) / 16.
      const d = at - timing.lastTimed.at - (senderMs - timing.lastTimed.senderMs);
      if (timing.jitterSource !== 'sender') timing.jitterMs = 0;
      timing.jitterMs! += (Math.abs(d) - timing.jitterMs!) * JITTER_GAIN;
      timing.jitterSource = 'sender';
    }
    timing.lastTimed = { at, senderMs };
  }

  /** The counters as of `now`, with rates over the last {@link RATE_WINDOW_MS}. */
  snapshot(now: number): TrackStats[] {
    const result: TrackStats[] = [];
    for (const track of this.#tracks.values()) {
      track.history.push({ at: now, objects: track.objects, bytes: track.bytes });
      while (track.history.length > 2 && now - track.history[1]!.at >= RATE_WINDOW_MS)
        track.history.shift();
      const oldest = track.history[0]!;
      const seconds = (now - oldest.at) / 1000;

      const name =
        track.fullTrackName ??
        (track.stream === 'subgroup'
          ? this.#aliasNames.get(`${track.direction}|${track.id}`)
          : undefined);
      const fallback = track.stream === 'fetch' ? `fetch ${track.id}` : `alias ${track.id}`;

      const {
        history: _history,
        groups,
        subgroups,
        fullTrackName: _name,
        id: _id,
        highestGroup: _highestGroup,
        timingState: timing,
        ...counts
      } = track;
      result.push({
        ...counts,
        label: name ? trackLabel(name) : fallback,
        groups: groups.size,
        subgroups: subgroups.size,
        objectsPerSecond: seconds > 0 ? (track.objects - oldest.objects) / seconds : 0,
        kbps: seconds > 0 ? ((track.bytes - oldest.bytes) * 8) / 1000 / seconds : 0,
        timing: {
          interval: summarize(timing.interval),
          jitterMs: timing.jitterMs,
          jitterSource: timing.jitterSource,
          latency: timing.latency && { ...summarize(timing.latency)!, last: timing.lastLatency! },
        },
      });
    }
    return result.sort((a, b) => a.label.localeCompare(b.label));
  }
}
