import type { Player } from '@/lib/player';
import type { MetricsSample, MetricsSnapshot } from './types';

const MAX_SAMPLES = 240;
const INTERVAL_MS = 250;

export class MetricsCollector {
  readonly #player: Player;
  readonly #bitrateMap: Record<string, number>;
  readonly #onSnapshot: (snapshot: MetricsSnapshot) => void;
  readonly #samples: MetricsSample[] = [];
  #intervalId: ReturnType<typeof setInterval> | null = null;

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

    this.#onSnapshot(this.getSnapshot());
  }
}
