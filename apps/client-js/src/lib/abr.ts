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

import type { Player } from '@/lib/player';

type Track = {
  name: string;
  bitrate?: number;
  role?: string;
  codec?: string;
  width?: number;
  height?: number;
};

export type SwitchReason = 'auto-upgrade' | 'auto-downgrade' | 'auto-emergency' | 'manual';

export interface SwitchEvent {
  ts: number;
  fromTrack: string;
  toTrack: string;
  reason: SwitchReason;
  bufferAtSwitch: number;
  emaBwAtSwitch: number;
}

export interface AbrThresholds {
  bolaV: number;
  bolaGp: number;
  bufferMax: number;
  segmentDurationS: number;
  emergencyBufferFloor: number;
  emaFactor: number;
  emaAlphaFast: number;
  emaAlphaSlow: number;
  pollIntervalMs: number;
}

export interface AbrMetrics {
  bandwidthBps: number;
  fastEmaBps: number;
  slowEmaBps: number;
  bufferSeconds: number;
  activeTrack: string | null;
  mode: 'auto' | 'manual';
  bolaScores: Record<string, number>;
  history: SwitchEvent[];
}

const HISTORY_MAX = 60;

const DEFAULT_THRESHOLDS: Omit<AbrThresholds, 'bolaV'> = {
  bolaGp: 1.0,
  bufferMax: 5.0,
  segmentDurationS: 1.0,
  emergencyBufferFloor: 0.5,
  emaFactor: 0.85,
  emaAlphaFast: 0.5,
  emaAlphaSlow: 0.1,
  pollIntervalMs: 250,
};

function computeV(
  bufferMax: number,
  segmentDurationS: number,
  utilityMax: number,
  bolaGp: number,
): number {
  return (bufferMax - segmentDurationS) / (utilityMax + bolaGp);
}

function utility(bitrate: number, bitrateMin: number): number {
  return Math.log(bitrate / bitrateMin) + 1;
}

// ---------------------------------------------------------------------------
// Standalone exported functions (used by tests and external callers)
// ---------------------------------------------------------------------------

/** Compute the BOLA V control parameter from high-level inputs. */
export function computeBolaV(
  bufferMax: number,
  bolaGp: number,
  segmentDurationS: number,
  tracks: { name: string; bitrate?: number }[],
): number {
  if (tracks.length === 0) return 1;
  const sorted = [...tracks].sort((a, b) => (a.bitrate ?? 0) - (b.bitrate ?? 0));
  const minBitrate = sorted[0]!.bitrate ?? 1;
  const maxBitrate = sorted[sorted.length - 1]!.bitrate ?? 1;
  const utilityMax = utility(maxBitrate, minBitrate);
  return computeV(bufferMax, segmentDurationS, utilityMax, bolaGp);
}

interface BolaScoreParams {
  bufferSeconds: number;
  bufferMax: number;
  bolaGp: number;
  bolaV: number;
  segmentDurationS: number;
  tracks: { name: string; bitrate?: number }[];
}

/** Compute BOLA score for every track — positive score = buffer supports that quality. */
export function computeBolaScores(params: BolaScoreParams): Record<string, number> {
  const { bufferSeconds, bolaGp, bolaV, tracks } = params;
  if (tracks.length === 0) return {};
  const sorted = [...tracks].sort((a, b) => (a.bitrate ?? 0) - (b.bitrate ?? 0));
  const minBitrate = sorted[0]!.bitrate ?? 1;
  const scores: Record<string, number> = {};
  for (const track of tracks) {
    const bitrate = track.bitrate ?? 1;
    const u = utility(bitrate, minBitrate);
    scores[track.name] = (bolaV * (u + bolaGp) - bufferSeconds) / bitrate;
  }
  return scores;
}

/** Select highest tier where BOLA score > 0 AND bitrate ≤ bandwidthBps * emaFactor. */
export function selectBestTier(
  scores: Record<string, number>,
  tracks: { name: string; bitrate?: number }[],
  bandwidthBps: number,
  emaFactor: number,
): string | null {
  if (bandwidthBps === 0) return null;
  const cap = bandwidthBps * emaFactor;
  const sorted = [...tracks].sort((a, b) => (b.bitrate ?? 0) - (a.bitrate ?? 0)); // descending
  for (const track of sorted) {
    const bitrate = track.bitrate ?? 0;
    if ((scores[track.name] ?? -Infinity) > 0 && bitrate <= cap) {
      return track.name;
    }
  }
  return null;
}

export class AbrController {
  #thresholds: AbrThresholds;
  #mode: 'auto' | 'manual' = 'manual';
  #switching = false;
  #intervalId: ReturnType<typeof setInterval> | null = null;
  #history: SwitchEvent[] = [];
  #tracks: Track[]; // sorted by bitrate ascending
  #bitrateMin: number;
  #utilityMax: number;

  constructor(
    private readonly player: Pick<Player, 'getMetrics' | 'switchTrack' | 'setEmaAlphas'>,
    tracks: Track[],
    private readonly onMetricsUpdate: (m: AbrMetrics) => void,
  ) {
    // Sort tracks by bitrate ascending
    this.#tracks = [...tracks].sort((a, b) => (a.bitrate ?? 0) - (b.bitrate ?? 0));
    this.#bitrateMin = this.#tracks[0]?.bitrate ?? 1;
    const bitrateMax = this.#tracks[this.#tracks.length - 1]?.bitrate ?? 1;
    this.#utilityMax = utility(bitrateMax, this.#bitrateMin);

    const v = computeV(
      DEFAULT_THRESHOLDS.bufferMax,
      DEFAULT_THRESHOLDS.segmentDurationS,
      this.#utilityMax,
      DEFAULT_THRESHOLDS.bolaGp,
    );
    this.#thresholds = { ...DEFAULT_THRESHOLDS, bolaV: v };
  }

  start(): void {
    this.#intervalId = setInterval(() => this._tick(), this.#thresholds.pollIntervalMs);
  }

  stop(): void {
    if (this.#intervalId !== null) {
      clearInterval(this.#intervalId);
      this.#intervalId = null;
    }
  }

  setMode(mode: 'auto' | 'manual'): void {
    this.#mode = mode;
  }

  setThresholds(partial: Partial<AbrThresholds>): void {
    const prev = this.#thresholds;
    this.#thresholds = { ...prev, ...partial };

    // Recompute bolaV if any of its inputs changed (unless caller explicitly set bolaV)
    const bolaInputChanged =
      partial.bufferMax !== undefined ||
      partial.segmentDurationS !== undefined ||
      partial.bolaGp !== undefined;
    if (bolaInputChanged && partial.bolaV === undefined) {
      this.#thresholds.bolaV = computeV(
        this.#thresholds.bufferMax,
        this.#thresholds.segmentDurationS,
        this.#utilityMax,
        this.#thresholds.bolaGp,
      );
    }

    // Propagate alpha changes to the player's tracker
    if (partial.emaAlphaFast !== undefined || partial.emaAlphaSlow !== undefined) {
      this.player.setEmaAlphas(this.#thresholds.emaAlphaFast, this.#thresholds.emaAlphaSlow);
    }
  }

  getThresholds(): AbrThresholds {
    return { ...this.#thresholds };
  }

  manualSwitch(trackName: string): void {
    if (this.#switching) return;
    const metrics = this.player.getMetrics();
    const fromTrack = metrics.activeTrack ?? '';
    if (fromTrack === trackName) return;
    this.#addHistory({
      ts: performance.now(),
      fromTrack,
      toTrack: trackName,
      reason: 'manual',
      bufferAtSwitch: metrics.bufferSeconds,
      emaBwAtSwitch: metrics.bandwidthBps,
    });
    this.player.switchTrack(trackName);
  }

  getHistory(): SwitchEvent[] {
    return [...this.#history];
  }

  /** Compute BOLA score for every track at the given buffer level. Public for testing. */
  computeBolaScores(bufferLevel: number): Record<string, number> {
    const { bolaV, bolaGp } = this.#thresholds;
    const scores: Record<string, number> = {};
    for (const track of this.#tracks) {
      const bitrate = track.bitrate ?? 1;
      const u = utility(bitrate, this.#bitrateMin);
      scores[track.name] = (bolaV * (u + bolaGp) - bufferLevel) / bitrate;
    }
    return scores;
  }

  /** Called by Player.onTrackSwitched callback to release the switching guard. */
  releaseSwitchingGuard(_trackName?: string): void {
    this.#switching = false;
  }

  /** Alias for releaseSwitchingGuard — called by onTrackSwitched in app.tsx. */
  onSwitchComplete(trackName: string): void {
    this.releaseSwitchingGuard(trackName);
  }

  /** Exposed for testing — runs one decision tick synchronously. */
  _tick(): void {
    if (this.#switching) return;

    const metrics = this.player.getMetrics();
    const scores = this.computeBolaScores(metrics.bufferSeconds);

    // Emit metrics to dashboard on every tick (regardless of mode)
    this.onMetricsUpdate({
      ...metrics,
      mode: this.#mode,
      bolaScores: scores,
      history: this.getHistory(),
    });

    if (this.#mode !== 'auto') return;

    // Emergency: buffer critically low
    if (metrics.bufferSeconds < this.#thresholds.emergencyBufferFloor) {
      const lowest = this.#tracks[0];
      if (lowest && lowest.name !== metrics.activeTrack) {
        this.#doSwitch(lowest.name, metrics, 'auto-emergency');
      }
      return;
    }

    // DEWMA cold start: skip until we have a sample
    if (metrics.bandwidthBps === 0) return;

    // BOLA + EMA cap: find best tier
    const cap = metrics.bandwidthBps * this.#thresholds.emaFactor;
    let best: Track | null = null;
    for (const track of this.#tracks) {
      const bitrate = track.bitrate ?? 0;
      if (scores[track.name] > 0 && bitrate <= cap) {
        best = track;
      }
    }

    if (best && best.name !== metrics.activeTrack) {
      const reason =
        (best.bitrate ?? 0) > (this.#tracks.find(t => t.name === metrics.activeTrack)?.bitrate ?? 0)
          ? 'auto-upgrade'
          : 'auto-downgrade';
      this.#doSwitch(best.name, metrics, reason);
    }
  }

  #doSwitch(
    trackName: string,
    metrics: ReturnType<typeof this.player.getMetrics>,
    reason: SwitchReason,
  ): void {
    this.#switching = true; // synchronous — before any async work
    this.#addHistory({
      ts: performance.now(),
      fromTrack: metrics.activeTrack ?? '',
      toTrack: trackName,
      reason,
      bufferAtSwitch: metrics.bufferSeconds,
      emaBwAtSwitch: metrics.bandwidthBps,
    });
    // Fire-and-forget — never await in the decision loop
    this.player.switchTrack(trackName);
  }

  #addHistory(event: SwitchEvent): void {
    this.#history.push(event);
    if (this.#history.length > HISTORY_MAX) {
      this.#history.shift();
    }
  }
}
