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
 * Dual Exponential Moving Average (DEWMA) goodput tracker.
 *
 * Maintains two simultaneous EMAs — fast (reactive) and slow (stable).
 * getBandwidthBps() returns min(fast, slow): the estimate drops immediately
 * when the fast EMA detects congestion, but only rises when both agree
 * bandwidth has recovered.
 *
 * Internal to Player — not re-exported from src/lib/index.ts.
 */
export class GoodputTracker {
  #emaFast: number = 0;
  #emaSlow: number = 0;
  #alphaFast: number;
  #alphaSlow: number;
  #hasData: boolean = false;
  #lastDeliveryTimeMs = 0;
  #lastObjectBytes = 0;
  #sampleCount = 0;

  // Windowed throughput: bytes received in a sliding window
  #windowBytes = 0;
  #windowStartMs = 0;
  #windowDurationMs = 3000; // 3-second sliding window

  constructor(alphaFast: number = 0.5, alphaSlow: number = 0.1) {
    this.#alphaFast = alphaFast;
    this.#alphaSlow = alphaSlow;
  }

  /**
   * Record one object delivery.
   * Uses a windowed byte counter for throughput — immune to MOQT's
   * bursty delivery pattern (unlike per-object or inter-arrival measurements).
   * @param bytes - payload size in bytes
   * @param _durationMs - ignored (kept for API compat), throughput is windowed
   */
  recordObject(bytes: number, _durationMs: number): void {
    const now = Date.now();
    this.#lastObjectBytes = bytes;
    this.#sampleCount++;

    // Initialize window on first sample
    if (this.#windowStartMs === 0) {
      this.#windowStartMs = now;
      this.#windowBytes = bytes;
      return;
    }

    this.#windowBytes += bytes;
    const elapsed = now - this.#windowStartMs;
    this.#lastDeliveryTimeMs = elapsed;

    // Only update EMA once we have enough window data (at least 100ms)
    if (elapsed < 100) return;

    const instantBps = (this.#windowBytes * 8 * 1000) / elapsed;

    if (!this.#hasData) {
      this.#emaFast = instantBps;
      this.#emaSlow = instantBps;
      this.#hasData = true;
    } else {
      this.#emaFast = this.#alphaFast * instantBps + (1 - this.#alphaFast) * this.#emaFast;
      this.#emaSlow = this.#alphaSlow * instantBps + (1 - this.#alphaSlow) * this.#emaSlow;
    }

    // Slide the window: reset every windowDuration
    if (elapsed >= this.#windowDurationMs) {
      this.#windowBytes = 0;
      this.#windowStartMs = now;
    }
  }

  /** Conservative bandwidth estimate: min(fast EMA, slow EMA). Returns 0 until first sample. */
  getBandwidthBps(): number {
    if (!this.#hasData) return 0;
    return Math.min(this.#emaFast, this.#emaSlow);
  }

  getFastEmaBps(): number {
    return this.#emaFast;
  }
  getSlowEmaBps(): number {
    return this.#emaSlow;
  }

  /** Reset both EMAs to 0. Call on track switch to avoid stale measurements. */
  reset(): void {
    this.#emaFast = 0;
    this.#emaSlow = 0;
    this.#hasData = false;
    this.#lastDeliveryTimeMs = 0;
    this.#lastObjectBytes = 0;
    this.#sampleCount = 0;
    this.#windowBytes = 0;
    this.#windowStartMs = 0;
  }

  /** Update smoothing factors without recreating the tracker. */
  setAlphas(alphaFast: number, alphaSlow: number): void {
    this.#alphaFast = alphaFast;
    this.#alphaSlow = alphaSlow;
  }

  getLastDeliveryTimeMs(): number {
    return this.#lastDeliveryTimeMs;
  }

  getLastObjectBytes(): number {
    return this.#lastObjectBytes;
  }

  getSampleCount(): number {
    return this.#sampleCount;
  }
}
