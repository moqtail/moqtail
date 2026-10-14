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

import { useEffect, useState } from 'react';
import { ArrowDownLeft, ArrowUpRight, ChevronRight } from 'lucide-react';
import { DataStats, Summary, TrackStats } from './moq/dataStats';

/** How often the counters are redrawn: objects arrive far faster than is worth rendering. */
const REFRESH_MS = 500;

type DataStatsPanelProps = {
  stats: DataStats;
  /** Only polls while visible. */
  isActive: boolean;
};

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(2)} MB`;
}

function formatRate(t: Pick<TrackStats, 'objectsPerSecond' | 'kbps'>): string {
  return `${t.objectsPerSecond.toFixed(1)} obj/s · ${t.kbps.toFixed(0)} kbps`;
}

const ms = (value: number) => `${value.toFixed(1)} ms`;

function TimingRow({
  label,
  summary,
  extra,
}: {
  label: string;
  summary?: Summary;
  extra?: string;
}) {
  return (
    <>
      <dt className="font-sans text-[10px] text-neutral-500">{label}</dt>
      {summary ? (
        <dd className="text-neutral-200 tabular-nums">
          {ms(summary.avg)}
          <span className="text-neutral-500">
            {' '}
            · {ms(summary.min)} – {ms(summary.max)}
            {extra && ` · ${extra}`}
          </span>
        </dd>
      ) : (
        <dd className="text-neutral-600">—</dd>
      )}
    </>
  );
}

/** Interval, jitter and latency, labelled for the direction the data flows. */
function Timing({ track }: { track: TrackStats }) {
  const { interval, jitterMs, jitterSource, latency } = track.timing;
  const isIncoming = track.direction === 'in';
  return (
    <dl className="grid grid-cols-[auto_1fr] items-baseline gap-x-3 gap-y-1 border-t border-white/4 px-2.5 py-2">
      <TimingRow label={isIncoming ? 'Interarrival' : 'Inter-send'} summary={interval} />
      <dt className="font-sans text-[10px] text-neutral-500">Jitter</dt>
      <dd
        className="text-neutral-200 tabular-nums"
        title={
          jitterSource === 'sender'
            ? 'RFC 3550: arrival spacing compared with the senders’ capture timestamps'
            : 'No capture timestamps: how much consecutive intervals differ'
        }
      >
        {jitterMs === undefined ? (
          <span className="text-neutral-600">—</span>
        ) : (
          <>
            {ms(jitterMs)}
            <span className="text-neutral-500">
              {' '}
              · {jitterSource === 'sender' ? 'RFC 3550' : 'interval variation'}
            </span>
          </>
        )}
      </dd>
      <TimingRow
        label={isIncoming ? 'Latency (capture → receive)' : 'Latency (capture → send)'}
        summary={latency}
        extra={latency && `last ${ms(latency.last)}`}
      />
    </dl>
  );
}

function Stat({ label, value, warn }: { label: string; value: string | number; warn?: boolean }) {
  return (
    <div className="flex flex-col">
      <span className="font-sans text-[10px] text-neutral-500">{label}</span>
      <span className={`tabular-nums ${warn ? 'text-amber-300' : 'text-neutral-200'}`}>
        {value}
      </span>
    </div>
  );
}

function TrackCard({ track }: { track: TrackStats }) {
  const [isExpanded, setIsExpanded] = useState(true);
  return (
    <div className="shrink-0 rounded-lg border border-white/6 bg-neutral-900/60">
      <button
        onClick={() => setIsExpanded(open => !open)}
        aria-expanded={isExpanded}
        className="flex w-full min-w-0 items-center gap-2 px-2.5 py-1.5 text-left"
      >
        <ChevronRight
          className={`h-3 w-3 shrink-0 text-neutral-600 transition-transform ${isExpanded ? 'rotate-90' : ''}`}
          aria-hidden="true"
        />
        <span className="min-w-0 flex-1 truncate text-neutral-200" title={track.label}>
          {track.label}
        </span>
        {track.stream === 'fetch' && (
          <span className="shrink-0 rounded bg-white/8 px-1 font-sans text-[10px] text-neutral-400">
            fetch
          </span>
        )}
        <span className="shrink-0 text-neutral-500 tabular-nums">{formatRate(track)}</span>
      </button>
      {isExpanded && (
        <div className="grid grid-cols-4 gap-x-3 gap-y-2 border-t border-white/4 px-2.5 py-2">
          <Stat label="Streams" value={`${track.streams} (${track.openStreams} open)`} />
          <Stat label="Groups" value={track.groups} />
          <Stat label="Subgroups" value={track.subgroups} />
          <Stat label="Objects" value={track.objects} />
          <Stat label="Payload" value={formatBytes(track.bytes)} />
          <Stat label="Status objects" value={track.statusObjects} />
          <Stat
            label="Last group/object"
            value={track.lastGroup === undefined ? '—' : `${track.lastGroup}/${track.lastObject}`}
          />
          <Stat label="Resets" value={track.resets} warn={track.resets > 0} />
          <Stat label="Errors" value={track.errors} warn={track.errors > 0} />
          <Stat label="Skipped groups" value={track.skippedGroups} warn={track.skippedGroups > 0} />
          <Stat
            label="Skipped objects"
            value={track.skippedObjects}
            warn={track.skippedObjects > 0}
          />
        </div>
      )}
      {isExpanded && <Timing track={track} />}
    </div>
  );
}

function Section({ title, tracks }: { title: 'Incoming' | 'Outgoing'; tracks: TrackStats[] }) {
  const totals = tracks.reduce(
    (sum, t) => ({
      openStreams: sum.openStreams + t.openStreams,
      objects: sum.objects + t.objects,
      objectsPerSecond: sum.objectsPerSecond + t.objectsPerSecond,
      kbps: sum.kbps + t.kbps,
    }),
    { openStreams: 0, objects: 0, objectsPerSecond: 0, kbps: 0 },
  );
  const Icon = title === 'Incoming' ? ArrowDownLeft : ArrowUpRight;
  return (
    <section className="flex shrink-0 flex-col gap-2">
      <div className="flex items-center gap-1.5 font-sans text-xs">
        <Icon
          className={`h-3.5 w-3.5 ${title === 'Incoming' ? 'text-emerald-400' : 'text-blue-400'}`}
          aria-hidden="true"
        />
        <span className="font-semibold text-neutral-200">{title}</span>
        <span className="ml-auto font-mono text-[11px] text-neutral-500 tabular-nums">
          {totals.objects} obj · {totals.openStreams} open · {formatRate(totals)}
        </span>
      </div>
      {tracks.length === 0 ? (
        <p className="font-sans text-xs text-neutral-600">No data streams yet.</p>
      ) : (
        tracks.map(track => <TrackCard key={track.key} track={track} />)
      )}
    </section>
  );
}

/** Per-track counters for the data streams, from the headers and object sizes the client reports. */
export function DataStatsPanel({ stats, isActive }: DataStatsPanelProps) {
  const [tracks, setTracks] = useState<TrackStats[]>([]);

  useEffect(() => {
    if (!isActive) return;
    const refresh = () => setTracks(stats.snapshot(Date.now()));
    refresh();
    const timer = setInterval(refresh, REFRESH_MS);
    return () => clearInterval(timer);
  }, [stats, isActive]);

  return (
    <div className="pane-scroll flex min-h-0 flex-1 flex-col gap-4 p-3 font-mono text-[11px]">
      <Section title="Incoming" tracks={tracks.filter(t => t.direction === 'in')} />
      <Section title="Outgoing" tracks={tracks.filter(t => t.direction === 'out')} />
    </div>
  );
}
