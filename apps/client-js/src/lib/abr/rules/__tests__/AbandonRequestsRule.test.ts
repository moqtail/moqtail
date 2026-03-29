import { describe, it, expect, beforeEach } from 'vitest';
import { AbandonRequestsRule } from '../AbandonRequestsRule';
import { DEFAULT_ABR_SETTINGS, SwitchRequestPriority } from '../../types';
import type { RulesContext } from '../../types';

function makeContext(overrides: Partial<RulesContext> = {}): RulesContext {
  return {
    tracks: [
      { name: '360p', bitrate: 500_000 },
      { name: '720p', bitrate: 1_500_000 },
      { name: '1080p', bitrate: 4_000_000 },
    ],
    activeTrackIndex: 1,
    bufferSeconds: 10,
    bandwidthBps: 2_000_000,
    fastEmaBps: 2_000_000,
    slowEmaBps: 2_000_000,
    droppedFrames: 0,
    totalFrames: 0,
    segmentDurationS: 4,
    isLowLatency: false,
    switchHistory: [],
    abrSettings: { ...DEFAULT_ABR_SETTINGS },
    ...overrides,
  };
}

describe('AbandonRequestsRule', () => {
  let rule: AbandonRequestsRule;

  beforeEach(() => {
    rule = new AbandonRequestsRule();
  });

  it('has the correct name', () => {
    expect(rule.name).toBe('AbandonRequestsRule');
  });

  it('returns null when buffer is above stable level', () => {
    // bufferSeconds (20) >= stableBufferTime (18) → no action
    const ctx = makeContext({
      bufferSeconds: 20,
      bandwidthBps: 100_000, // bandwidth very low, but buffer is fine
    });
    // Call enough times to exceed minThroughputSamples
    for (let i = 0; i < 5; i++) rule.getMaxIndex(ctx);
    expect(rule.getMaxIndex(ctx)).toBeNull();
  });

  it('returns null when already at lowest quality', () => {
    // activeTrackIndex === 0 → cannot go lower
    const ctx = makeContext({
      activeTrackIndex: 0,
      bufferSeconds: 5,
      bandwidthBps: 100_000, // too slow to sustain current quality
    });
    for (let i = 0; i < 5; i++) rule.getMaxIndex(ctx);
    expect(rule.getMaxIndex(ctx)).toBeNull();
  });

  it('returns null when bandwidth is sufficient for current quality', () => {
    // activeTrackIndex=1, bitrate=1_500_000, segmentDurationS=4
    // segmentSizeBits = 1_500_000 * 4 = 6_000_000
    // estimatedDeliveryS = 6_000_000 / 5_000_000 = 1.2s
    // abandonDurationMultiplier * segmentDurationS = 1.8 * 4 = 7.2s
    // 1.2 <= 7.2 → no abandon
    const ctx = makeContext({
      activeTrackIndex: 1,
      bufferSeconds: 5,
      bandwidthBps: 5_000_000,
    });
    for (let i = 0; i < 5; i++) rule.getMaxIndex(ctx);
    expect(rule.getMaxIndex(ctx)).toBeNull();
  });

  it('recommends lower quality when delivery is too slow (after enough samples)', () => {
    // activeTrackIndex=2, bitrate=4_000_000, segmentDurationS=4
    // segmentSizeBits = 4_000_000 * 4 = 16_000_000
    // estimatedDeliveryS = 16_000_000 / 500_000 = 32s
    // abandonDurationMultiplier * segmentDurationS = 1.8 * 4 = 7.2s
    // 32 > 7.2 → should abandon
    const ctx = makeContext({
      activeTrackIndex: 2,
      bufferSeconds: 5,
      bandwidthBps: 500_000,
    });

    // First 5 calls should return null (sampleCount < minThroughputSamples = 6)
    for (let i = 0; i < 5; i++) {
      expect(rule.getMaxIndex(ctx)).toBeNull();
    }

    // 6th call should trigger abandon
    const result = rule.getMaxIndex(ctx);
    expect(result).not.toBeNull();
    expect(result!.priority).toBe(SwitchRequestPriority.STRONG);
    expect(result!.reason).toBe('abandon-slow-delivery');
    // representationIndex should be a lower track than current (2)
    expect(result!.representationIndex).toBeLessThan(2);
  });

  it('returns null before reaching minThroughputSamples', () => {
    const ctx = makeContext({
      activeTrackIndex: 2,
      bufferSeconds: 5,
      bandwidthBps: 100_000, // very slow
    });
    // With minThroughputSamplesThreshold=6, first 5 calls should be null
    for (let i = 0; i < 5; i++) {
      expect(rule.getMaxIndex(ctx)).toBeNull();
    }
  });

  it('returns null when bandwidthBps is 0', () => {
    const ctx = makeContext({
      activeTrackIndex: 2,
      bufferSeconds: 5,
      bandwidthBps: 0,
    });
    for (let i = 0; i < 10; i++) rule.getMaxIndex(ctx);
    expect(rule.getMaxIndex(ctx)).toBeNull();
  });

  it('reset() clears sampleCount so early-sample guard applies again', () => {
    const ctx = makeContext({
      activeTrackIndex: 2,
      bufferSeconds: 5,
      bandwidthBps: 500_000,
    });

    // Get past the sample threshold
    for (let i = 0; i < 6; i++) rule.getMaxIndex(ctx);
    expect(rule.getMaxIndex(ctx)).not.toBeNull();

    // After reset, sample count is cleared — first call is back to null
    rule.reset();
    expect(rule.getMaxIndex(ctx)).toBeNull();
  });

  it('returns STRONG priority on abandon', () => {
    const ctx = makeContext({
      activeTrackIndex: 2,
      bufferSeconds: 5,
      bandwidthBps: 500_000,
    });
    for (let i = 0; i < 6; i++) rule.getMaxIndex(ctx);
    const result = rule.getMaxIndex(ctx);
    expect(result!.priority).toBe(SwitchRequestPriority.STRONG);
  });
});
