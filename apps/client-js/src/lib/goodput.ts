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
 * Dual Exponential Moving Average (DEWMA) goodput tracker backed by
 * WebTransport connection statistics.
 *
 * When a WebTransport session is bound via {@link setTransport}, bandwidth
 * estimation is derived from the QUIC stack's `bytesReceived` counter
 * (polled via {@link https://developer.mozilla.org/docs/Web/API/WebTransport/getStats | WebTransport.getStats()}).
 * This replaces the previous application-layer byte-counting window with
 * a transport-level measurement that accounts for retransmissions and
 * QUIC framing — giving a more accurate picture of link capacity.
 *
 * Falls back to manual application-layer byte counting when
 * WebTransport.getStats() is unavailable (e.g. Firefox, Safari).
 *
 * Internal to Player — not re-exported from src/lib/index.ts.
 */
/**
 * Extended WebTransport interface for browsers that support getStats()
 * (Chromium 114+). Not yet in TypeScript's lib.dom.d.ts.
 */
interface WebTransportWithStats extends WebTransport {
  getStats(): Promise<{ bytesReceived?: number; bytesSent?: number; smoothedRtt?: number }>;
}

export class GoodputTracker {
  #emaFast: number = 0;
  #emaSlow: number = 0;
  #alphaFast: number;
  #alphaSlow: number;
  #hasData: boolean = false;
  #lastDeliveryTimeMs = 0;
  #lastObjectBytes = 0;
  #sampleCount = 0;

  // WebTransport stats-based throughput
  #transport: WebTransportWithStats | null = null;
  #prevBytesReceived = 0;
  #prevTimestampMs = 0;
  #useTransportStats = false;

  // Fallback: windowed throughput (used when getStats() is unavailable)
  #windowBytes = 0;
  #windowStartMs = 0;
  #windowDurationMs = 3000; // 3-second sliding window

  constructor(alphaFast: number = 0.5, alphaSlow: number = 0.1) {
    this.#alphaFast = alphaFast;
    this.#alphaSlow = alphaSlow;
  }

  /**
   * Bind to the WebTransport session for transport-level throughput estimation.
   * Falls back to application-layer byte counting if getStats() is not supported.
   */
  setTransport(transport: WebTransport): void {
    const hasGetStats = typeof (transport as WebTransportWithStats).getStats === 'function';
    this.#transport = hasGetStats ? (transport as WebTransportWithStats) : null;
    this.#useTransportStats = hasGetStats;
  }

  /**
   * Poll WebTransport.getStats() and update EMAs from the bytesReceived delta.
   * Call this on the ABR tick interval (e.g. every 250ms).
   * No-op when transport stats are unavailable (fallback path uses recordObject).
   */
  async poll(): Promise<void> {
    if (!this.#useTransportStats || !this.#transport) return;

    const stats = await this.#transport.getStats();
    const now = Date.now();
    const bytesReceived = stats.bytesReceived ?? 0;

    if (this.#prevTimestampMs === 0) {
      // First poll — seed baseline, no EMA update yet
      this.#prevBytesReceived = bytesReceived;
      this.#prevTimestampMs = now;
      return;
    }

    const deltaBytes = bytesReceived - this.#prevBytesReceived;
    const deltaMs = now - this.#prevTimestampMs;
    this.#prevBytesReceived = bytesReceived;
    this.#prevTimestampMs = now;

    if (deltaMs < 50) return; // too short to be meaningful

    const instantBps = (deltaBytes * 8 * 1000) / deltaMs;

    this.#updateEma(instantBps);
  }

  /**
   * Record one object delivery.
   *
   * When transport stats are available this only tracks metadata
   * (lastObjectBytes, sampleCount). When transport stats are NOT
   * available this is the primary throughput source (fallback path).
   */
  recordObject(bytes: number, _durationMs: number): void {
    const now = Date.now();
    this.#lastObjectBytes = bytes;
    this.#sampleCount++;

    // If we're using transport stats, skip application-layer throughput
    if (this.#useTransportStats) return;

    // --- Fallback: windowed byte counting ---
    if (this.#windowStartMs === 0) {
      this.#windowStartMs = now;
      this.#windowBytes = bytes;
      return;
    }

    this.#windowBytes += bytes;
    const elapsed = now - this.#windowStartMs;
    this.#lastDeliveryTimeMs = elapsed;

    if (elapsed < 100) return;

    const instantBps = (this.#windowBytes * 8 * 1000) / elapsed;
    this.#updateEma(instantBps);

    // Slide the window
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

  /** Reset both EMAs to 0. */
  reset(): void {
    this.#emaFast = 0;
    this.#emaSlow = 0;
    this.#hasData = false;
    this.#lastDeliveryTimeMs = 0;
    this.#lastObjectBytes = 0;
    this.#sampleCount = 0;
    this.#windowBytes = 0;
    this.#windowStartMs = 0;
    this.#prevBytesReceived = 0;
    this.#prevTimestampMs = 0;
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

  #updateEma(instantBps: number): void {
    if (!this.#hasData) {
      this.#emaFast = instantBps;
      this.#emaSlow = instantBps;
      this.#hasData = true;
    } else {
      this.#emaFast = this.#alphaFast * instantBps + (1 - this.#alphaFast) * this.#emaFast;
      this.#emaSlow = this.#alphaSlow * instantBps + (1 - this.#alphaSlow) * this.#emaSlow;
    }
  }
}
