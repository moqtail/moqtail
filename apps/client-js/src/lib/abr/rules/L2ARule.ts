import { SwitchRequestPriority, DEFAULT_ABR_SETTINGS } from '../types';
import type { AbrRule, RulesContext, SwitchRequest } from '../types';

// L2A states
const enum L2AState {
  ONE_BITRATE = 0,
  STARTUP = 1,
  STEADY = 2,
}

const HORIZON = 4;
const VL = Math.pow(HORIZON, 0.99); // ~3.98
const REACT = 2;
const B_TARGET = 1.5; // seconds

/**
 * Projects weight vector onto the probability simplex using Euclidean projection.
 * Modifies the array in place.
 */
function projectOntoProbabilitySimplex(w: number[]): void {
  const n = w.length;
  // Sort descending (keep original indices by operating on a sorted copy)
  const sorted = [...w].sort((a, b) => b - a);

  let tMax = 0;
  let tmpSum = 0;
  for (let i = 0; i < n; i++) {
    tmpSum += sorted[i]!;
    const tCandidate = (tmpSum - 1) / (i + 1);
    if (sorted[i]! - tCandidate > 0) {
      tMax = tCandidate;
    }
  }

  for (let i = 0; i < n; i++) {
    w[i] = Math.max(w[i]! - tMax, 0);
  }
}

export class L2ARule implements AbrRule {
  readonly name = 'L2ARule';

  #state: L2AState = L2AState.STARTUP;
  #w: number[] = [];
  #startupCount = 0;

  getMaxIndex(context: RulesContext): SwitchRequest | null {
    const { tracks, bandwidthBps, bufferSeconds, activeTrackIndex, abrSettings } = context;
    const n = tracks.length;

    // Single track — nothing to decide
    if (n <= 1) {
      return null;
    }

    // No bandwidth estimate yet
    if (bandwidthBps === 0) {
      return null;
    }

    const rulePriority =
      abrSettings.rules['L2ARule']?.priority ??
      DEFAULT_ABR_SETTINGS.rules['L2ARule']?.priority ??
      SwitchRequestPriority.DEFAULT;

    // Initialize weight vector if needed (e.g. after reset or first call)
    if (this.#w.length !== n) {
      this.#w = Array(n).fill(1 / n);
      this.#state = L2AState.STARTUP;
      this.#startupCount = 0;
    }

    const { bandwidthSafetyFactor } = abrSettings;
    const safeThroughput = bandwidthBps * bandwidthSafetyFactor;

    // --- STARTUP state: throughput-based selection ---
    if (this.#state === L2AState.STARTUP) {
      this.#startupCount += 1;

      // Find highest track fitting within safe throughput
      let bestIndex = -1;
      for (let i = 0; i < n; i++) {
        const bitrate = tracks[i]!.bitrate ?? 0;
        if (bitrate <= safeThroughput) {
          bestIndex = i;
        }
      }

      if (bestIndex === -1) {
        return null;
      }

      // Transition to STEADY after HORIZON ticks
      if (this.#startupCount >= HORIZON) {
        this.#state = L2AState.STEADY;
      }

      return {
        representationIndex: bestIndex,
        priority: rulePriority,
        reason: 'l2a-startup',
      };
    }

    // --- STEADY state: Lagrangian gradient update ---

    // REACT recalibration: if current bitrate > safeThroughput * REACT, reset weights
    const currentBitrate = tracks[activeTrackIndex]?.bitrate ?? 0;
    if (currentBitrate > safeThroughput * REACT) {
      this.#w = Array(n).fill(0);
      this.#w[0] = 1;
    } else {
      // Compute gradient and update weights using Lagrangian descent
      // L2A gradient: g_i = VL * (bitrate_i / max_bitrate) - (buffer - B_TARGET) * bitrate_i
      const maxBitrate = Math.max(...tracks.map((t) => t.bitrate ?? 0));

      for (let i = 0; i < n; i++) {
        const bitrate_i = (tracks[i]!.bitrate ?? 0) / (maxBitrate || 1);
        const bufferTerm = (bufferSeconds - B_TARGET) * bitrate_i;
        const gradient = VL * bitrate_i - bufferTerm;
        // Gradient ascent: w[i] += gradient / VL (step size = 1/VL)
        this.#w[i] = (this.#w[i] ?? 0) + gradient / VL;
      }

      // Project onto probability simplex
      projectOntoProbabilitySimplex(this.#w);
    }

    // Pick the index with the highest weight
    let bestIndex = 0;
    let bestWeight = this.#w[0]!;
    for (let i = 1; i < n; i++) {
      if (this.#w[i]! > bestWeight) {
        bestWeight = this.#w[i]!;
        bestIndex = i;
      }
    }

    return {
      representationIndex: bestIndex,
      priority: rulePriority,
      reason: 'l2a-steady',
    };
  }

  reset(): void {
    this.#state = L2AState.STARTUP;
    this.#w = [];
    this.#startupCount = 0;
  }
}
