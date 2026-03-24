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
 * Dual Exponential Weighted Moving Average (DEWMA) goodput tracker.
 *
 * Maintains two simultaneous EMAs of per-object download bandwidth:
 *   - fast (default α=0.5): reacts quickly to sudden drops
 *   - slow (default α=0.1): stable historical baseline
 *
 * getBandwidthBps() returns min(fast, slow) — conservative, safe estimate.
 * The estimate only rises when BOTH EMAs agree bandwidth has recovered.
 */
export class GoodputTracker {
  private emaFast = 0
  private emaSlow = 0
  private hasData = false

  constructor(
    private alphaFast = 0.5,
    private alphaSlow = 0.1,
  ) {}

  /**
   * Record one object delivery. Call from inside the WritableStream write handler
   * with the number of bytes in the payload and the milliseconds elapsed
   * from when the write began to when appendBuffer completed.
   */
  recordObject(bytes: number, durationMs: number): void {
    const instantaneous = (bytes * 8 * 1000) / Math.max(durationMs, 0.001)
    if (!this.hasData) {
      this.emaFast = instantaneous
      this.emaSlow = instantaneous
      this.hasData = true
    } else {
      this.emaFast = this.alphaFast * instantaneous + (1 - this.alphaFast) * this.emaFast
      this.emaSlow = this.alphaSlow * instantaneous + (1 - this.alphaSlow) * this.emaSlow
    }
  }

  /** Returns min(emaFast, emaSlow) in bps. Returns 0 until first sample recorded. */
  getBandwidthBps(): number {
    if (!this.hasData) return 0
    return Math.min(this.emaFast, this.emaSlow)
  }

  /** Returns the fast EMA value in bps (for dashboard display). */
  getFastEmaBps(): number {
    return this.emaFast
  }

  /** Returns the slow EMA value in bps (for dashboard display). */
  getSlowEmaBps(): number {
    return this.emaSlow
  }

  /** Resets both EMAs to 0. Call after a track switch. */
  reset(): void {
    this.emaFast = 0
    this.emaSlow = 0
    this.hasData = false
  }

  /** Updates alpha values without recreating the tracker. */
  setAlphas(alphaFast: number, alphaSlow: number): void {
    this.alphaFast = alphaFast
    this.alphaSlow = alphaSlow
  }
}
