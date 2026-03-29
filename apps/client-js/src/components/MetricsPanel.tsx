import { useState } from 'preact/hooks';
import type { AbrMetrics } from '@/lib/abr/AbrController';
import type { MetricsSnapshot, MetricsSample } from '@/lib/metrics/types';
import type { Track } from '@/lib/abr/types';
import { MetricsChart } from './MetricsChart';
import type { ChartLine } from './MetricsChart';

// ── Metric definitions ──

interface MetricDef {
  key: string;
  label: string;
  unit: string;
  unitLabel: string;
  color: string;
  extract: (s: MetricsSample) => number;
  step?: boolean;
}

const METRIC_DEFS: MetricDef[] = [
  {
    key: 'buffer',
    label: 'Buffer Level',
    unit: 's',
    unitLabel: 'Seconds',
    color: '#3b82f6',
    extract: s => s.bufferSeconds,
  },
  {
    key: 'bitrate',
    label: 'Bitrate',
    unit: 'kbps',
    unitLabel: 'kbps',
    color: '#a855f7',
    extract: s => s.bitrateKbps,
    step: true,
  },
  {
    key: 'bandwidth',
    label: 'Avg Throughput',
    unit: 'kbps',
    unitLabel: 'kbit/s',
    color: '#06b6d4',
    extract: s => s.bandwidthBps / 1000,
  },
  {
    key: 'fastEma',
    label: 'Fast EMA',
    unit: 'kbps',
    unitLabel: 'kbit/s',
    color: '#22c55e',
    extract: s => s.fastEmaBps / 1000,
  },
  {
    key: 'slowEma',
    label: 'Slow EMA',
    unit: 'kbps',
    unitLabel: 'kbit/s',
    color: '#f59e0b',
    extract: s => s.slowEmaBps / 1000,
  },
  {
    key: 'dropped',
    label: 'Dropped Frames',
    unit: 'count',
    unitLabel: 'Count',
    color: '#ef4444',
    extract: s => s.droppedFrames,
  },
  {
    key: 'playbackRate',
    label: 'Playback Rate',
    unit: 'x',
    unitLabel: 'Ratio',
    color: '#ec4899',
    extract: s => s.playbackRate,
  },
  {
    key: 'deliveryTime',
    label: 'Delivery Time',
    unit: 'ms',
    unitLabel: 'ms',
    color: '#14b8a6',
    extract: s => s.deliveryTimeMs,
  },
];

const METRIC_MAP = new Map(METRIC_DEFS.map(d => [d.key, d]));
const MAX_ACTIVE = 5;
const DEFAULT_ACTIVE = new Set(['buffer', 'bitrate']);

// ── Helpers ──

function fmt(value: number, unit: string): string {
  if (unit === 'kbps' || unit === 'kbit/s') {
    if (value >= 1_000_000) return `${(value / 1_000_000).toFixed(1)} Mbit/s`;
    if (value >= 1_000) return `${(value / 1_000).toFixed(0)} kbit/s`;
    return `${value.toFixed(0)} bit/s`;
  }
  if (unit === 's') return `${value.toFixed(2)} s`;
  if (unit === 'ms') return `${value.toFixed(0)} ms`;
  if (unit === 'x') return `${value.toFixed(2)}x`;
  return `${value} ${unit}`;
}

// ── Toggle dot next to plottable stat rows ──

function ToggleDot({
  metricKey,
  active,
  activeCount,
  onToggle,
}: {
  metricKey: string;
  active: boolean;
  activeCount: number;
  onToggle: (key: string) => void;
}) {
  const def = METRIC_MAP.get(metricKey);
  if (!def) return null;
  const disabled = !active && activeCount >= MAX_ACTIVE;
  return (
    <button
      onClick={() => !disabled && onToggle(metricKey)}
      className="mr-1 h-2.5 w-2.5 flex-shrink-0 rounded-full border transition-colors"
      style={{
        borderColor: active ? def.color : '#525252',
        backgroundColor: active ? def.color : 'transparent',
        opacity: disabled ? 0.35 : 1,
        cursor: disabled ? 'not-allowed' : 'pointer',
      }}
      title={
        disabled
          ? `Max ${MAX_ACTIVE} metrics — disable one first`
          : active
            ? 'Remove from chart'
            : 'Add to chart'
      }
    />
  );
}

function StatRow({
  label,
  value,
  metricKey,
  activeMetrics,
  onToggle,
}: {
  label: string;
  value: string;
  metricKey?: string;
  activeMetrics?: Set<string>;
  onToggle?: (key: string) => void;
}) {
  return (
    <div className="flex justify-between py-0.5">
      <span className="flex items-center text-neutral-500">
        {metricKey && activeMetrics && onToggle && (
          <ToggleDot
            metricKey={metricKey}
            active={activeMetrics.has(metricKey)}
            activeCount={activeMetrics.size}
            onToggle={onToggle}
          />
        )}
        {label}
      </span>
      <span className="font-mono text-neutral-200 tabular-nums">{value}</span>
    </div>
  );
}

// ── Panel ──

interface MetricsPanelProps {
  metrics: AbrMetrics | null;
  snapshot: MetricsSnapshot | null;
  tracks: Track[];
}

export function MetricsPanel({ metrics, snapshot, tracks }: MetricsPanelProps) {
  const [activeMetrics, setActiveMetrics] = useState<Set<string>>(DEFAULT_ACTIVE);

  if (!metrics) return null;

  const toggle = (key: string) => {
    setActiveMetrics(prev => {
      const next = new Set(prev);
      if (next.has(key)) {
        next.delete(key);
      } else if (next.size < MAX_ACTIVE) {
        next.add(key);
      }
      return next;
    });
  };

  const sortedTracks = [...tracks].sort((a, b) => (a.bitrate ?? 0) - (b.bitrate ?? 0));
  const trackCount = sortedTracks.length;
  const activeIndex = metrics.activeTrackIndex;
  const activeTrackData = sortedTracks[activeIndex];

  const bufferState = metrics.bufferSeconds > 0 ? 'Loaded' : 'Empty';
  const goodputRatio =
    metrics.deliveryTimeMs > 0 && metrics.bandwidthBps > 0
      ? (
          (metrics.lastObjectBytes * 8 * 1000) /
          metrics.deliveryTimeMs /
          metrics.bandwidthBps
        ).toFixed(2)
      : '-';

  const samples = snapshot?.samples ?? [];

  // Build chart lines for active metrics
  const chartLines: ChartLine[] = [];
  for (const key of activeMetrics) {
    const def = METRIC_MAP.get(key);
    if (!def || samples.length < 2) continue;
    chartLines.push({
      label: def.label,
      color: def.color,
      data: samples.map(def.extract),
      unit: def.unit,
      unitLabel: def.unitLabel,
      step: def.step,
    });
  }

  return (
    <div className="space-y-3 text-[11px]">
      {/* ── Video stats ── */}
      <div>
        <p className="mb-1 font-semibold tracking-widest text-neutral-500 uppercase">Video</p>
        <div className="space-y-0">
          <StatRow
            label="Buffer Level"
            value={fmt(metrics.bufferSeconds, 's')}
            metricKey="buffer"
            activeMetrics={activeMetrics}
            onToggle={toggle}
          />
          <StatRow
            label="Bitrate (downloading)"
            value={fmt(activeTrackData?.bitrate ?? 0, 'kbps')}
            metricKey="bitrate"
            activeMetrics={activeMetrics}
            onToggle={toggle}
          />
          <StatRow label="Index (downloading)" value={`${activeIndex} / ${trackCount}`} />
          <StatRow label="Index (playing)" value={`${activeIndex} / ${trackCount}`} />
          <StatRow
            label="Resolution"
            value={
              activeTrackData?.width && activeTrackData?.height
                ? `${activeTrackData.width}x${activeTrackData.height}`
                : '-'
            }
          />
          <StatRow
            label="Framerate"
            value={activeTrackData?.framerate ? `${activeTrackData.framerate} fps` : '-'}
          />
          <StatRow label="Codec" value={activeTrackData?.codec ?? '-'} />
          <StatRow label="Segment Duration" value="-" />
          <StatRow label="Buffer State" value={bufferState} />
        </div>
      </div>

      {/* ── Playback Metrics ── */}
      <div>
        <p className="mb-1 font-semibold tracking-widest text-neutral-500 uppercase">
          Playback Metrics
        </p>
        <div className="space-y-0">
          <StatRow
            label="Dropped Frames"
            value={`${metrics.droppedFrames}`}
            metricKey="dropped"
            activeMetrics={activeMetrics}
            onToggle={toggle}
          />
          <StatRow
            label="Avg Throughput"
            value={fmt(metrics.bandwidthBps, 'kbps')}
            metricKey="bandwidth"
            activeMetrics={activeMetrics}
            onToggle={toggle}
          />
          <StatRow
            label="Fast EMA"
            value={fmt(metrics.fastEmaBps, 'kbps')}
            metricKey="fastEma"
            activeMetrics={activeMetrics}
            onToggle={toggle}
          />
          <StatRow
            label="Slow EMA"
            value={fmt(metrics.slowEmaBps, 'kbps')}
            metricKey="slowEma"
            activeMetrics={activeMetrics}
            onToggle={toggle}
          />
          <StatRow
            label="Playback Rate"
            value={fmt(metrics.playbackRate, 'x')}
            metricKey="playbackRate"
            activeMetrics={activeMetrics}
            onToggle={toggle}
          />
        </div>
      </div>

      {/* ── Transport Metrics ── */}
      <div>
        <p className="mb-1 font-semibold tracking-widest text-neutral-500 uppercase">
          Transport Metrics
        </p>
        <div className="space-y-0">
          <StatRow
            label="QUIC RTT (ms)"
            value={metrics.deliveryTimeMs > 0 ? fmt(metrics.deliveryTimeMs, 'ms') : '-'}
          />
          <StatRow
            label="Delivery Time (ms)"
            value={metrics.deliveryTimeMs > 0 ? fmt(metrics.deliveryTimeMs, 'ms') : '-'}
            metricKey="deliveryTime"
            activeMetrics={activeMetrics}
            onToggle={toggle}
          />
          <StatRow label="Goodput Ratio" value={goodputRatio} />
        </div>
      </div>

      {/* ── Unified chart ── */}
      {chartLines.length > 0 && (
        <div>
          <MetricsChart lines={chartLines} />
          <p className="mt-1 text-[9px] text-neutral-600">
            Toggle metrics with the dots above (max {MAX_ACTIVE})
          </p>
        </div>
      )}
    </div>
  );
}
