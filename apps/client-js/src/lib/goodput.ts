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

  constructor(alphaFast: number = 0.5, alphaSlow: number = 0.1) {
    this.#alphaFast = alphaFast;
    this.#alphaSlow = alphaSlow;
  }

  /**
   * Record one object delivery measurement.
   * @param bytes - payload size in bytes
   * @param durationMs - wall-clock time elapsed from send to SourceBuffer append completion, in ms
   */
  recordObject(bytes: number, durationMs: number): void {
    if (durationMs <= 0) return;
    this.#lastDeliveryTimeMs = durationMs;
    this.#lastObjectBytes = bytes;
    this.#sampleCount++;
    const instantBps = (bytes * 8 * 1000) / durationMs;
    if (!this.#hasData) {
      // Cold start: seed both EMAs to the first sample so there is no ramp-up lag
      this.#emaFast = instantBps;
      this.#emaSlow = instantBps;
      this.#hasData = true;
    } else {
      this.#emaFast = this.#alphaFast * instantBps + (1 - this.#alphaFast) * this.#emaFast;
      this.#emaSlow = this.#alphaSlow * instantBps + (1 - this.#alphaSlow) * this.#emaSlow;
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
