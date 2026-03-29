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

import { describe, it, expect, beforeEach } from 'vitest';
import { BolaRule } from '../BolaRule';
import { DEFAULT_ABR_SETTINGS } from '../../types';
import type { RulesContext } from '../../types';

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

const TRACKS_MULTI = [
  { name: '360p', bitrate: 500_000 },
  { name: '720p', bitrate: 2_000_000 },
  { name: '1080p', bitrate: 4_000_000 },
];

const TRACKS_SINGLE = [{ name: '360p', bitrate: 500_000 }];

function makeContext(overrides: Partial<RulesContext> = {}): RulesContext {
  return {
    tracks: TRACKS_MULTI,
    activeTrackIndex: 0,
    bufferSeconds: 15,
    bandwidthBps: 10_000_000,
    fastEmaBps: 10_000_000,
    slowEmaBps: 10_000_000,
    droppedFrames: 0,
    totalFrames: 0,
    segmentDurationS: 2,
    isLowLatency: false,
    switchHistory: [],
    abrSettings: DEFAULT_ABR_SETTINGS,
    ...overrides,
  };
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

describe('BolaRule', () => {
  let rule: BolaRule;

  beforeEach(() => {
    rule = new BolaRule();
  });

  // -------------------------------------------------------------------------
  // 1. Returns null when only one track (ONE_BITRATE state)
  // -------------------------------------------------------------------------
  describe('ONE_BITRATE state', () => {
    it('returns null when tracks array has exactly one entry', () => {
      const ctx = makeContext({ tracks: TRACKS_SINGLE });
      expect(rule.getMaxIndex(ctx)).toBeNull();
    });

    it('returns null when tracks array is empty', () => {
      const ctx = makeContext({ tracks: [] });
      expect(rule.getMaxIndex(ctx)).toBeNull();
    });
  });

  // -------------------------------------------------------------------------
  // 2. Throughput-based selection during STARTUP (low buffer)
  // -------------------------------------------------------------------------
  describe('STARTUP state', () => {
    it('uses throughput-based selection when buffer < segmentDurationS', () => {
      // Buffer is 0.5s, segment duration is 2s → STARTUP
      const bw = 2_500_000; // fits 720p (2 Mbps) with 0.9 safety factor (cap = 2.25 Mbps)
      const ctx = makeContext({ bufferSeconds: 0.5, segmentDurationS: 2, bandwidthBps: bw });
      const result = rule.getMaxIndex(ctx);

      expect(result).not.toBeNull();
      // throughput cap = 2.25 Mbps, so 720p (2 Mbps) qualifies but 1080p (4 Mbps) doesn't
      const chosenTrack = TRACKS_MULTI[result!.representationIndex];
      expect(chosenTrack?.bitrate).toBeLessThanOrEqual(bw * DEFAULT_ABR_SETTINGS.bandwidthSafetyFactor);
    });

    it('returns lowest quality when no throughput data (bandwidthBps === 0)', () => {
      const ctx = makeContext({ bufferSeconds: 0, segmentDurationS: 2, bandwidthBps: 0 });
      const result = rule.getMaxIndex(ctx);

      expect(result).not.toBeNull();
      expect(result!.representationIndex).toBe(0); // lowest quality track (360p is at index 0)
    });

    it('transitions to STEADY after buffer >= segmentDurationS', () => {
      // First call: STARTUP
      rule.getMaxIndex(makeContext({ bufferSeconds: 0.5, segmentDurationS: 2, bandwidthBps: 5_000_000 }));

      // Second call with sufficient buffer: should enter STEADY and pick based on Lyapunov score
      const highBufferCtx = makeContext({
        bufferSeconds: 15,
        segmentDurationS: 2,
        bandwidthBps: 10_000_000,
        activeTrackIndex: 0,
      });
      const result = rule.getMaxIndex(highBufferCtx);
      expect(result).not.toBeNull();
      expect(result!.reason).toContain('BOLA steady');
    });
  });

  // -------------------------------------------------------------------------
  // 3. In STEADY state, higher buffer favors higher quality
  // -------------------------------------------------------------------------
  describe('STEADY state', () => {
    function getQualityAtBuffer(bufferSeconds: number): number {
      const r = new BolaRule();
      // First call with high buffer to jump directly into STEADY
      r.getMaxIndex(makeContext({ bufferSeconds, segmentDurationS: 2, bandwidthBps: 10_000_000 }));
      // Second call to evaluate Lyapunov scores in STEADY
      const result = r.getMaxIndex(
        makeContext({ bufferSeconds, segmentDurationS: 2, bandwidthBps: 10_000_000 }),
      );
      return result?.representationIndex ?? -1;
    }

    it('higher buffer level selects equal or higher quality than lower buffer', () => {
      // With enough buffer to pass startup on first call, steady-state BOLA should prefer
      // higher quality when buffer is high (larger feasible set)
      const lowBufIdx = getQualityAtBuffer(10);
      const highBufIdx = getQualityAtBuffer(16);

      // Higher buffer should yield equal or higher quality index
      expect(highBufIdx).toBeGreaterThanOrEqual(lowBufIdx);
    });

    it('selects highest quality with very high buffer and ample bandwidth', () => {
      const r = new BolaRule();
      // Transition through startup with high buffer
      r.getMaxIndex(makeContext({ bufferSeconds: 15, segmentDurationS: 2, bandwidthBps: 20_000_000 }));
      const result = r.getMaxIndex(
        makeContext({
          bufferSeconds: 17,
          segmentDurationS: 2,
          bandwidthBps: 20_000_000,
          activeTrackIndex: 2,
        }),
      );
      expect(result).not.toBeNull();
      // With very high buffer and unlimited bandwidth, 1080p (highest, origIdx 2) should win
      const chosen = TRACKS_MULTI[result!.representationIndex];
      expect(chosen?.name).toBe('1080p');
    });

    it('reason field is "BOLA steady" in steady state', () => {
      const r = new BolaRule();
      r.getMaxIndex(makeContext({ bufferSeconds: 15, segmentDurationS: 2, bandwidthBps: 10_000_000 }));
      const result = r.getMaxIndex(makeContext({ bufferSeconds: 15, segmentDurationS: 2, bandwidthBps: 10_000_000 }));
      expect(result?.reason).toBe('BOLA steady');
    });
  });

  // -------------------------------------------------------------------------
  // 4. BOLA-O: does not increase above throughput-sustainable level
  // -------------------------------------------------------------------------
  describe('BOLA-O oscillation cap', () => {
    it('does not select quality above what throughput can sustain', () => {
      const r = new BolaRule();

      // Low bandwidth: only 360p (500 kbps) is sustainable with safety factor 0.9
      // cap = 600_000 * 0.9 = 540_000 bps → only 360p fits
      const lowBw = 600_000;

      // Transition to STEADY via high buffer
      r.getMaxIndex(makeContext({ bufferSeconds: 15, segmentDurationS: 2, bandwidthBps: lowBw }));

      // In steady state, Lyapunov might want higher quality, but BOLA-O should cap it
      const result = r.getMaxIndex(
        makeContext({
          bufferSeconds: 17, // high buffer could push Lyapunov score toward higher quality
          segmentDurationS: 2,
          bandwidthBps: lowBw,
          activeTrackIndex: 0, // currently on 360p
        }),
      );

      expect(result).not.toBeNull();
      const chosenBitrate = TRACKS_MULTI[result!.representationIndex]?.bitrate ?? 0;
      const throughputCap = lowBw * DEFAULT_ABR_SETTINGS.bandwidthSafetyFactor;
      expect(chosenBitrate).toBeLessThanOrEqual(throughputCap);
    });

    it('BOLA-O: will not jump above current track when throughput is only marginal', () => {
      const r = new BolaRule();

      // Set bandwidth such that 720p (2 Mbps) is borderline.
      // cap = 2_200_000 * 0.9 = 1_980_000 — just below 720p (2 Mbps)
      const marginalBw = 2_200_000;

      // Transition to STEADY
      r.getMaxIndex(makeContext({ bufferSeconds: 15, segmentDurationS: 2, bandwidthBps: marginalBw }));

      // Suppose we're on 360p (index 0 in original array). With high buffer, Lyapunov
      // might score 1080p highest, but BOLA-O should clamp to throughputIndex.
      const result = r.getMaxIndex(
        makeContext({
          bufferSeconds: 17,
          segmentDurationS: 2,
          bandwidthBps: marginalBw,
          activeTrackIndex: 0,
        }),
      );

      expect(result).not.toBeNull();
      const chosenBitrate = TRACKS_MULTI[result!.representationIndex]?.bitrate ?? 0;
      // throughput cap = 1.98 Mbps → only 360p (500 kbps) fits
      expect(chosenBitrate).toBeLessThanOrEqual(marginalBw * DEFAULT_ABR_SETTINGS.bandwidthSafetyFactor);
    });
  });

  // -------------------------------------------------------------------------
  // 5. reset() returns to STARTUP state
  // -------------------------------------------------------------------------
  describe('reset()', () => {
    it('returns to STARTUP after reset', () => {
      const r = new BolaRule();

      // Push into STEADY
      r.getMaxIndex(makeContext({ bufferSeconds: 15, segmentDurationS: 2, bandwidthBps: 10_000_000 }));
      r.getMaxIndex(makeContext({ bufferSeconds: 15, segmentDurationS: 2, bandwidthBps: 10_000_000 }));

      // Verify it is in STEADY (reason = 'BOLA steady')
      const beforeReset = r.getMaxIndex(
        makeContext({ bufferSeconds: 15, segmentDurationS: 2, bandwidthBps: 10_000_000 }),
      );
      expect(beforeReset?.reason).toBe('BOLA steady');

      // Reset
      r.reset();

      // Now with low buffer it should be in STARTUP
      const afterReset = r.getMaxIndex(
        makeContext({ bufferSeconds: 0.5, segmentDurationS: 2, bandwidthBps: 10_000_000 }),
      );
      expect(afterReset?.reason).toContain('BOLA startup');
    });

    it('reset() clears placeholder buffer', () => {
      const r = new BolaRule();

      // Run a few STARTUP iterations to build up placeholder buffer
      for (let i = 0; i < 5; i++) {
        r.getMaxIndex(
          makeContext({ bufferSeconds: 0.5, segmentDurationS: 2, bandwidthBps: 10_000_000 }),
        );
      }

      r.reset();

      // After reset with zero bandwidth, should return lowest quality (placeholder = 0)
      const result = r.getMaxIndex(
        makeContext({ bufferSeconds: 0, segmentDurationS: 2, bandwidthBps: 0 }),
      );
      expect(result?.representationIndex).toBe(0);
    });
  });
});
