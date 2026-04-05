import type { Player } from '@/lib/player';
import type { MetricsSample, MetricsSnapshot } from './types';

const MAX_SAMPLES = 240;
const INTERVAL_MS = 250;
/** Flush accumulated CSV rows to the server every N samples. */
const LOG_FLUSH_INTERVAL = 4; // every 1 second (4 * 250ms)

const CSV_HEADER =
  'timestamp,elapsed_s,buffer_s,bitrate_kbps,bandwidth_kbps,fast_ema_kbps,slow_ema_kbps,dropped_frames,total_frames,playback_rate,delivery_time_ms';

export class MetricsCollector {
  readonly #player: Player;
  readonly #bitrateMap: Record<string, number>;
  readonly #onSnapshot: (snapshot: MetricsSnapshot) => void;
  readonly #samples: MetricsSample[] = [];
  readonly #allSamples: MetricsSample[] = [];
  /** Pending CSV rows not yet flushed to the log file. */
  readonly #pendingLogRows: string[] = [];
  #intervalId: ReturnType<typeof setInterval> | null = null;
  #sessionStartTs: number = 0;
  #sampleCount: number = 0;
  #headerSent: boolean = false;

  constructor(
    player: Player,
    bitrateMap: Record<string, number>,
    onSnapshot: (snapshot: MetricsSnapshot) => void,
  ) {
    this.#player = player;
    this.#bitrateMap = bitrateMap;
    this.#onSnapshot = onSnapshot;
  }

  start(): void {
    if (this.#intervalId !== null) return;
    this.#sessionStartTs = Date.now();
    this.#intervalId = setInterval(() => this.#sample(), INTERVAL_MS);
  }

  stop(): void {
    if (this.#intervalId === null) return;
    clearInterval(this.#intervalId);
    this.#intervalId = null;
  }

  getSnapshot(): MetricsSnapshot {
    const samples = [...this.#samples];
    return {
      samples,
      latest: samples.length > 0 ? (samples[samples.length - 1] ?? null) : null,
    };
  }

  #sample(): void {
    const m = this.#player.getMetrics();
    const sample: MetricsSample = {
      ts: Date.now(),
      bufferSeconds: m.bufferSeconds,
      bitrateKbps: m.activeTrack !== null ? (this.#bitrateMap[m.activeTrack] ?? 0) : 0,
      bandwidthBps: m.bandwidthBps,
      fastEmaBps: m.fastEmaBps,
      slowEmaBps: m.slowEmaBps,
      droppedFrames: m.droppedFrames,
      totalFrames: m.totalFrames,
      playbackRate: m.playbackRate,
      deliveryTimeMs: m.deliveryTimeMs,
    };

    this.#samples.push(sample);
    if (this.#samples.length > MAX_SAMPLES) {
      this.#samples.shift();
    }

    this.#allSamples.push(sample);
    this.#pendingLogRows.push(this.#sampleToCsvRow(sample));
    this.#sampleCount++;

    if (this.#sampleCount % LOG_FLUSH_INTERVAL === 0) {
      this.#flushToServer();
    }

    this.#onSnapshot(this.getSnapshot());
  }

  #sampleToCsvRow(s: MetricsSample): string {
    const elapsed = ((s.ts - this.#sessionStartTs) / 1000).toFixed(3);
    return [
      new Date(s.ts).toISOString(),
      elapsed,
      s.bufferSeconds.toFixed(3),
      s.bitrateKbps.toFixed(0),
      (s.bandwidthBps / 1000).toFixed(1),
      (s.fastEmaBps / 1000).toFixed(1),
      (s.slowEmaBps / 1000).toFixed(1),
      s.droppedFrames,
      s.totalFrames,
      s.playbackRate.toFixed(4),
      s.deliveryTimeMs.toFixed(1),
    ].join(',');
  }

  #flushToServer(): void {
    if (this.#pendingLogRows.length === 0) return;

    const rows = this.#pendingLogRows.splice(0);
    const body = this.#headerSent
      ? rows.join('\n')
      : [CSV_HEADER, ...rows].join('\n');
    this.#headerSent = true;

    // Fire-and-forget — don't block the sampling loop
    fetch('/__metrics', { method: 'POST', body }).catch(() => {
      // Dev server not available (e.g. production build) — silently ignore
    });
  }

  exportCsv(): string {
    const rows = this.#allSamples.map(s => this.#sampleToCsvRow(s));
    return [CSV_HEADER, ...rows].join('\n');
  }
}
