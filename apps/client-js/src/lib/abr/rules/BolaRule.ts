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

/**
 * BolaRule — Buffer Occupancy based Lyapunov Algorithm (BOLA)
 *
 * Implements a 3-state machine:
 *   BOLA_STATE_ONE_BITRATE (0): Only one representation available → no switch needed.
 *   BOLA_STATE_STARTUP     (1): Buffer below one segment duration → throughput-based selection.
 *   BOLA_STATE_STEADY      (2): Full Lyapunov optimisation with BOLA-O oscillation cap.
 *
 * Reference: Spiteri et al., "BOLA: Near-optimal bitrate adaptation for online videos"
 */

import type { AbrRule, RulesContext, SwitchRequest } from '../types';
import { SwitchRequestPriority } from '../types';

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const MINIMUM_BUFFER_S = 10;
const PLACEHOLDER_BUFFER_DECAY = 0.99;

// Internal state enum
const enum BolaState {
  ONE_BITRATE = 0,
  STARTUP = 1,
  STEADY = 2,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/** Log-based utility function. Returns 1 for the lowest bitrate, >1 for higher tiers. */
function utility(bitrate: number, bitrateMin: number): number {
  return Math.log(bitrate / bitrateMin) + 1;
}

/**
 * Compute Vp and gp from BOLA parameters.
 *
 * gp = (utilityMax - 1) / (bufferTime / MINIMUM_BUFFER_S - 1)
 * Vp = MINIMUM_BUFFER_S / gp
 */
function computeBolaParams(
  bufferTime: number,
  utilityMax: number,
): { Vp: number; gp: number } {
  const gp = (utilityMax - 1) / (bufferTime / MINIMUM_BUFFER_S - 1);
  const Vp = MINIMUM_BUFFER_S / gp;
  return { Vp, gp };
}

/**
 * Select a representation index from bandwidth, applying a safety factor.
 * Returns the highest index whose bitrate is <= bandwidth * safetyFactor.
 * Falls back to 0 if none qualifies.
 */
function throughputIndex(
  bitrates: number[],
  bandwidthBps: number,
  safetyFactor: number = 0.9,
): number {
  const cap = bandwidthBps * safetyFactor;
  let best = 0;
  for (let i = 0; i < bitrates.length; i++) {
    if ((bitrates[i] ?? 0) <= cap) {
      best = i;
    }
  }
  return best;
}

// ---------------------------------------------------------------------------
// BolaRule
// ---------------------------------------------------------------------------

export class BolaRule implements AbrRule {
  readonly name = 'BolaRule';

  #state: BolaState = BolaState.STARTUP;
  /** Virtual placeholder buffer used during STARTUP to avoid cold-start oscillation. */
  #placeholderBuffer: number = 0;

  getMaxIndex(context: RulesContext): SwitchRequest | null {
    const { tracks, activeTrackIndex, bufferSeconds, bandwidthBps, segmentDurationS, abrSettings } =
      context;

    // -----------------------------------------------------------------------
    // State 0: only one representation — nothing to decide
    // -----------------------------------------------------------------------
    if (tracks.length <= 1) {
      this.#state = BolaState.ONE_BITRATE;
      return null;
    }

    // Sort tracks by bitrate ascending and build parallel arrays.
    // We work with indices into the *sorted* array throughout.
    const sorted = [...tracks]
      .map((t, origIdx) => ({ t, origIdx }))
      .sort((a, b) => (a.t.bitrate ?? 0) - (b.t.bitrate ?? 0));

    const bitrates = sorted.map(({ t }) => t.bitrate ?? 1);
    const bitrateMin = bitrates[0] ?? 1;
    const utilities = bitrates.map(br => utility(br, bitrateMin));
    const utilityMax = utilities[utilities.length - 1] ?? 1;

    // Map activeTrackIndex (original order) → sorted index
    const sortedActiveIndex = sorted.findIndex(({ origIdx }) => origIdx === activeTrackIndex);
    const currentRepIndex = sortedActiveIndex >= 0 ? sortedActiveIndex : 0;

    const bufferTime = abrSettings.bufferTimeDefault;
    const { Vp, gp } = computeBolaParams(bufferTime, utilityMax);

    // -----------------------------------------------------------------------
    // State 1: STARTUP — buffer below one segment duration
    // -----------------------------------------------------------------------
    if (this.#state !== BolaState.STEADY) {
      this.#state = BolaState.STARTUP;
    }

    if (this.#state === BolaState.STARTUP) {
      // Decay placeholder buffer on each call to prevent it from stalling startup
      this.#placeholderBuffer = Math.max(0, this.#placeholderBuffer * PLACEHOLDER_BUFFER_DECAY);

      // Effective buffer is the real buffer plus the placeholder
      const effectiveBuffer = bufferSeconds + this.#placeholderBuffer;

      if (bufferSeconds >= segmentDurationS) {
        // Enough real buffer accumulated — transition to steady state
        this.#state = BolaState.STEADY;
        this.#placeholderBuffer = 0;
      } else {
        // Still in startup: use throughput-based selection
        if (bandwidthBps === 0) {
          // No throughput measurement yet — stay at lowest quality
          return {
            representationIndex: sorted[0]!.origIdx,
            priority: SwitchRequestPriority.DEFAULT,
            reason: 'BOLA startup: no throughput data',
          };
        }

        const tpIdx = throughputIndex(
          bitrates,
          bandwidthBps,
          abrSettings.bandwidthSafetyFactor,
        );

        // Grow placeholder buffer toward what STEADY state would need
        const tpBitrate = bitrates[tpIdx] ?? 1;
        const increment = (effectiveBuffer * tpBitrate) / (bitrates[0] ?? 1);
        this.#placeholderBuffer = Math.min(
          this.#placeholderBuffer + increment * PLACEHOLDER_BUFFER_DECAY,
          bufferTime,
        );

        const representationIndex = sorted[tpIdx]!.origIdx;
        return {
          representationIndex,
          priority: SwitchRequestPriority.DEFAULT,
          reason: 'BOLA startup: throughput-based',
        };
      }
    }

    // -----------------------------------------------------------------------
    // State 2: STEADY — Lyapunov optimisation
    // -----------------------------------------------------------------------

    // Compute score for each representation: score[i] = (Vp*(utility[i]+gp) - bufferLevel) / bitrate[i]
    let bestIndex = 0;
    let bestScore = -Infinity;
    for (let i = 0; i < bitrates.length; i++) {
      const score = (Vp * ((utilities[i] ?? 0) + gp) - bufferSeconds) / (bitrates[i] ?? 1);
      if (score > bestScore) {
        bestScore = score;
        bestIndex = i;
      }
    }

    // BOLA-O: oscillation prevention cap.
    // If the algorithm wants to go higher than both the throughput-sustainable level
    // AND the current level, cap it at max(throughputIndex, currentRepIndex).
    const tpIdx = throughputIndex(bitrates, bandwidthBps, abrSettings.bandwidthSafetyFactor);
    if (bestIndex > currentRepIndex && bestIndex > tpIdx) {
      bestIndex = Math.max(tpIdx, currentRepIndex);
    }

    const representationIndex = sorted[bestIndex]!.origIdx;
    return {
      representationIndex,
      priority: SwitchRequestPriority.DEFAULT,
      reason: 'BOLA steady',
    };
  }

  reset(): void {
    this.#state = BolaState.STARTUP;
    this.#placeholderBuffer = 0;
  }
}
