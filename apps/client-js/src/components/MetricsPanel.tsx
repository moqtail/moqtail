import type { AbrMetrics } from '@/lib/abr/AbrController';
import type { MetricsSnapshot } from '@/lib/metrics/types';
import type { Track } from '@/lib/abr/types';
import { MetricsChart } from './MetricsChart';

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

function StatRow({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex justify-between py-0.5">
      <span className="text-neutral-500">{label}</span>
      <span className="font-mono text-neutral-200 tabular-nums">{value}</span>
    </div>
  );
}

interface MetricsPanelProps {
  metrics: AbrMetrics | null;
  snapshot: MetricsSnapshot | null;
  tracks: Track[];
}

export function MetricsPanel({ metrics, snapshot, tracks }: MetricsPanelProps) {
  if (!metrics) return null;

  const sortedTracks = [...tracks].sort((a, b) => (a.bitrate ?? 0) - (b.bitrate ?? 0));
  const trackCount = sortedTracks.length;
  const activeIndex = metrics.activeTrackIndex;
  const activeTrackData = sortedTracks[activeIndex];

  const bufferState = metrics.bufferSeconds > 0 ? 'Loaded' : 'Empty';
  const goodputRatio = metrics.deliveryTimeMs > 0 && metrics.bandwidthBps > 0
    ? ((metrics.lastObjectBytes * 8 * 1000) / metrics.deliveryTimeMs / metrics.bandwidthBps).toFixed(2)
    : '-';

  const samples = snapshot?.samples ?? [];
  const bufferData = samples.map(s => s.bufferSeconds);
  const bitrateData = samples.map(s => s.bitrateKbps);
  const bwData = samples.map(s => s.bandwidthBps / 1000);
  const fastEmaData = samples.map(s => s.fastEmaBps / 1000);
  const slowEmaData = samples.map(s => s.slowEmaBps / 1000);

  return (
    <div className="space-y-3 text-[11px]">
      <div>
        <p className="mb-1 font-semibold tracking-widest text-neutral-500 uppercase">Video</p>
        <div className="space-y-0">
          <StatRow label="Buffer Level" value={fmt(metrics.bufferSeconds, 's')} />
          <StatRow label="Bitrate (downloading)" value={fmt((activeTrackData?.bitrate ?? 0) / 1000, 'kbps')} />
          <StatRow label="Index (downloading)" value={`${activeIndex} / ${trackCount}`} />
          <StatRow label="Index (playing)" value={`${activeIndex} / ${trackCount}`} />
          <StatRow label="Resolution" value={
            activeTrackData?.width && activeTrackData?.height
              ? `${activeTrackData.width}x${activeTrackData.height}`
              : '-'
          } />
          <StatRow label="Framerate" value={activeTrackData?.framerate ? `${activeTrackData.framerate} fps` : '-'} />
          <StatRow label="Codec" value={activeTrackData?.codec ?? '-'} />
          <StatRow label="Segment Duration" value="-" />
          <StatRow label="Buffer State" value={bufferState} />
        </div>
      </div>

      <div>
        <p className="mb-1 font-semibold tracking-widest text-neutral-500 uppercase">Playback Metrics</p>
        <div className="space-y-0">
          <StatRow label="Dropped Frames" value={`${metrics.droppedFrames}`} />
          <StatRow label="Avg Throughput" value={fmt(metrics.bandwidthBps / 1000, 'kbps')} />
          <StatRow label="Playback Rate" value={fmt(metrics.playbackRate, 'x')} />
        </div>
      </div>

      <div>
        <p className="mb-1 font-semibold tracking-widest text-neutral-500 uppercase">Transport Metrics</p>
        <div className="space-y-0">
          <StatRow label="QUIC RTT (ms)" value={metrics.deliveryTimeMs > 0 ? fmt(metrics.deliveryTimeMs, 'ms') : '-'} />
          <StatRow label="Delivery Time (ms)" value={metrics.deliveryTimeMs > 0 ? fmt(metrics.deliveryTimeMs, 'ms') : '-'} />
          <StatRow label="Goodput Ratio" value={goodputRatio} />
        </div>
      </div>

      {samples.length > 1 && (
        <div className="space-y-3">
          <MetricsChart
            title="Buffer Level"
            lines={[{ label: 'Buffer', color: '#3b82f6', data: bufferData }]}
            thresholds={[
              { value: 18, color: '#22c55e', label: '18s' },
              { value: 0.5, color: '#ef4444', label: '0.5s' },
            ]}
            yUnit="s"
          />
          <MetricsChart
            title="Bitrate"
            lines={[{ label: 'Active', color: '#a855f7', data: bitrateData }]}
            yUnit="kbps"
            step
          />
          <MetricsChart
            title="Throughput"
            lines={[
              { label: 'Estimate', color: '#3b82f6', data: bwData },
              { label: 'Fast EMA', color: '#22c55e', data: fastEmaData },
              { label: 'Slow EMA', color: '#f59e0b', data: slowEmaData },
            ]}
            yUnit="kbps"
          />
        </div>
      )}
    </div>
  );
}
