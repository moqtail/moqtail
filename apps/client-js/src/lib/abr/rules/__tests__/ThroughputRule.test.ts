import { describe, it, expect, beforeEach } from 'vitest';
import { ThroughputRule } from '../ThroughputRule';
import { DEFAULT_ABR_SETTINGS, SwitchRequestPriority } from '../../types';
import type { RulesContext } from '../../types';

function makeContext(overrides: Partial<RulesContext> = {}): RulesContext {
  return {
    tracks: [
      { name: '360p', bitrate: 500_000 },
      { name: '720p', bitrate: 1_500_000 },
      { name: '1080p', bitrate: 4_000_000 },
    ],
    activeTrackIndex: 0,
    bufferSeconds: 10,
    bandwidthBps: 0,
    fastEmaBps: 0,
    slowEmaBps: 0,
    droppedFrames: 0,
    totalFrames: 0,
    segmentDurationS: 4,
    isLowLatency: false,
    switchHistory: [],
    abrSettings: { ...DEFAULT_ABR_SETTINGS },
    ...overrides,
  };
}

describe('ThroughputRule', () => {
  let rule: ThroughputRule;

  beforeEach(() => {
    rule = new ThroughputRule();
  });

  it('has the correct name', () => {
    expect(rule.name).toBe('ThroughputRule');
  });

  it('returns null when bandwidthBps is 0 (cold start)', () => {
    const ctx = makeContext({ bandwidthBps: 0 });
    expect(rule.getMaxIndex(ctx)).toBeNull();
  });

  it('selects the highest track that fits within effective bandwidth', () => {
    // effectiveBandwidth = 2_000_000 * 0.9 = 1_800_000
    // tracks: 500k fits, 1.5M fits, 4M does not → bestIndex = 1
    const ctx = makeContext({ bandwidthBps: 2_000_000 });
    const result = rule.getMaxIndex(ctx);
    expect(result).not.toBeNull();
    expect(result!.representationIndex).toBe(1);
  });

  it('selects highest track when all tracks fit', () => {
    // effectiveBandwidth = 10_000_000 * 0.9 = 9_000_000 → all tracks fit → index 2
    const ctx = makeContext({ bandwidthBps: 10_000_000 });
    const result = rule.getMaxIndex(ctx);
    expect(result!.representationIndex).toBe(2);
  });

  it('selects the lowest track when only it fits', () => {
    // effectiveBandwidth = 600_000 * 0.9 = 540_000 → only 500k track fits
    const ctx = makeContext({ bandwidthBps: 600_000 });
    const result = rule.getMaxIndex(ctx);
    expect(result!.representationIndex).toBe(0);
  });

  it('returns null when no track fits within effective bandwidth', () => {
    // effectiveBandwidth = 400_000 * 0.9 = 360_000 → nothing fits
    const ctx = makeContext({ bandwidthBps: 400_000 });
    expect(rule.getMaxIndex(ctx)).toBeNull();
  });

  it('respects bandwidthSafetyFactor from settings', () => {
    // With safetyFactor = 1.0, effectiveBandwidth = 1_500_000 → index 1
    const ctx = makeContext({
      bandwidthBps: 1_500_000,
      abrSettings: { ...DEFAULT_ABR_SETTINGS, bandwidthSafetyFactor: 1.0 },
    });
    const result = rule.getMaxIndex(ctx);
    expect(result!.representationIndex).toBe(1);
  });

  it('respects maxBitrate setting', () => {
    // effectiveBandwidth = 10M * 0.9 = 9M, but maxBitrate = 1_600_000 → capped at index 1
    const ctx = makeContext({
      bandwidthBps: 10_000_000,
      abrSettings: { ...DEFAULT_ABR_SETTINGS, maxBitrate: 1_600_000 },
    });
    const result = rule.getMaxIndex(ctx);
    expect(result!.representationIndex).toBe(1);
  });

  it('respects minBitrate setting', () => {
    // effectiveBandwidth = 600_000 * 0.9 = 540_000; only 500k fits bandwidth,
    // but minBitrate = 600_000 excludes it → null
    const ctx = makeContext({
      bandwidthBps: 600_000,
      abrSettings: { ...DEFAULT_ABR_SETTINGS, minBitrate: 600_000 },
    });
    expect(rule.getMaxIndex(ctx)).toBeNull();
  });

  it('returns a SwitchRequest with DEFAULT priority', () => {
    const ctx = makeContext({ bandwidthBps: 2_000_000 });
    const result = rule.getMaxIndex(ctx);
    expect(result!.priority).toBe(SwitchRequestPriority.DEFAULT);
    expect(result!.reason).toBe('throughput');
  });

  it('reset() does not throw (stateless)', () => {
    expect(() => rule.reset()).not.toThrow();
  });

  it('handles tracks with no bitrate (treated as 0)', () => {
    const ctx = makeContext({
      bandwidthBps: 500_000,
      tracks: [{ name: 'no-bitrate' }, { name: '360p', bitrate: 400_000 }],
    });
    // effectiveBandwidth = 500_000 * 0.9 = 450_000
    // track 0: bitrate=0 fits, track 1: 400k fits → bestIndex = 1
    const result = rule.getMaxIndex(ctx);
    expect(result!.representationIndex).toBe(1);
  });
});
