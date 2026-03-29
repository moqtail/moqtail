import { describe, it, expect, beforeEach } from 'vitest';
import { InsufficientBufferRule } from '../InsufficientBufferRule';
import { SwitchRequestPriority, DEFAULT_ABR_SETTINGS } from '../../types';
import type { RulesContext } from '../../types';

const tracks = [
  { name: 'low', bitrate: 500_000 },
  { name: 'mid', bitrate: 1_500_000 },
  { name: 'high', bitrate: 4_000_000 },
];

function makeContext(overrides: Partial<RulesContext> = {}): RulesContext {
  return {
    tracks,
    activeTrackIndex: 1,
    bufferSeconds: 20,
    bandwidthBps: 5_000_000,
    fastEmaBps: 5_000_000,
    slowEmaBps: 5_000_000,
    droppedFrames: 0,
    totalFrames: 1000,
    segmentDurationS: 4,
    isLowLatency: false,
    switchHistory: [],
    abrSettings: DEFAULT_ABR_SETTINGS,
    ...overrides,
  };
}

describe('InsufficientBufferRule', () => {
  let rule: InsufficientBufferRule;

  beforeEach(() => {
    rule = new InsufficientBufferRule();
  });

  it('ignores first segmentIgnoreCount (2) calls and returns null', () => {
    const ctx = makeContext({ bufferSeconds: 0 });

    // Call 1 — should be ignored
    expect(rule.getMaxIndex(ctx)).toBeNull();
    // Call 2 — should also be ignored
    expect(rule.getMaxIndex(ctx)).toBeNull();
    // Call 3 — now past ignore count, buffer = 0, should intervene
    const result = rule.getMaxIndex(ctx);
    expect(result).not.toBeNull();
    expect(result!.representationIndex).toBe(0);
    expect(result!.priority).toBe(SwitchRequestPriority.STRONG);
  });

  it('returns STRONG priority index 0 when buffer is empty (after ignore count)', () => {
    const ctx = makeContext({ bufferSeconds: 0 });

    // Burn through ignore count
    rule.getMaxIndex(ctx);
    rule.getMaxIndex(ctx);

    const result = rule.getMaxIndex(ctx);
    expect(result).not.toBeNull();
    expect(result!.representationIndex).toBe(0);
    expect(result!.priority).toBe(SwitchRequestPriority.STRONG);
    expect(result!.reason).toBe('insufficient-buffer-empty');
  });

  it('returns null when buffer is healthy (>= stableBufferTime)', () => {
    // DEFAULT_ABR_SETTINGS.stableBufferTime = 18
    const ctx = makeContext({ bufferSeconds: 20 });

    // Burn through ignore count
    rule.getMaxIndex(ctx);
    rule.getMaxIndex(ctx);

    const result = rule.getMaxIndex(ctx);
    expect(result).toBeNull();
  });

  it('returns null when buffer exactly equals stableBufferTime', () => {
    const ctx = makeContext({ bufferSeconds: DEFAULT_ABR_SETTINGS.stableBufferTime });

    rule.getMaxIndex(ctx);
    rule.getMaxIndex(ctx);

    expect(rule.getMaxIndex(ctx)).toBeNull();
  });

  it('caps bitrate proportional to buffer level when buffer is low', () => {
    // bufferSeconds=8, segmentDurationS=4, bandwidthBps=5_000_000, throughputSafetyFactor=0.7
    // cappedBps = 5_000_000 * 0.7 * (8 / 4) = 5_000_000 * 0.7 * 2 = 7_000_000
    // All three tracks (500k, 1.5M, 4M) fit within 7M → bestIndex = 2
    const ctx = makeContext({ bufferSeconds: 8, bandwidthBps: 5_000_000, segmentDurationS: 4 });

    rule.getMaxIndex(ctx);
    rule.getMaxIndex(ctx);

    const result = rule.getMaxIndex(ctx);
    expect(result).not.toBeNull();
    expect(result!.representationIndex).toBe(2);
    expect(result!.priority).toBe(SwitchRequestPriority.DEFAULT);
    expect(result!.reason).toBe('insufficient-buffer-low');
  });

  it('caps to lower track when buffer is very low', () => {
    // bufferSeconds=1, segmentDurationS=4, bandwidthBps=5_000_000, throughputSafetyFactor=0.7
    // cappedBps = 5_000_000 * 0.7 * (1 / 4) = 875_000
    // tracks: 500k fits, 1.5M does not → bestIndex = 0
    const ctx = makeContext({ bufferSeconds: 1, bandwidthBps: 5_000_000, segmentDurationS: 4 });

    rule.getMaxIndex(ctx);
    rule.getMaxIndex(ctx);

    const result = rule.getMaxIndex(ctx);
    expect(result).not.toBeNull();
    expect(result!.representationIndex).toBe(0);
    expect(result!.priority).toBe(SwitchRequestPriority.DEFAULT);
  });

  it('uses STRONG priority when buffer is below 0.5s', () => {
    const ctx = makeContext({ bufferSeconds: 0.3, bandwidthBps: 5_000_000, segmentDurationS: 4 });

    rule.getMaxIndex(ctx);
    rule.getMaxIndex(ctx);

    const result = rule.getMaxIndex(ctx);
    expect(result).not.toBeNull();
    expect(result!.priority).toBe(SwitchRequestPriority.STRONG);
    expect(result!.reason).toBe('insufficient-buffer-low');
  });

  it('uses DEFAULT priority when buffer is between 0.5s and stableBufferTime', () => {
    // bufferSeconds=2, segmentDurationS=4, bandwidthBps=10_000_000
    // cappedBps = 10_000_000 * 0.7 * (2/4) = 3_500_000 → tracks 500k and 1.5M fit → bestIndex = 1
    const ctx = makeContext({ bufferSeconds: 2, bandwidthBps: 10_000_000, segmentDurationS: 4 });

    rule.getMaxIndex(ctx);
    rule.getMaxIndex(ctx);

    const result = rule.getMaxIndex(ctx);
    expect(result).not.toBeNull();
    expect(result!.priority).toBe(SwitchRequestPriority.DEFAULT);
  });

  it('reset() clears callCount so ignoring restarts', () => {
    const ctx = makeContext({ bufferSeconds: 0 });

    // Burn through ignore count and confirm intervention
    rule.getMaxIndex(ctx);
    rule.getMaxIndex(ctx);
    expect(rule.getMaxIndex(ctx)).not.toBeNull();

    // Reset — ignore count starts over
    rule.reset();
    expect(rule.getMaxIndex(ctx)).toBeNull(); // call 1 after reset
    expect(rule.getMaxIndex(ctx)).toBeNull(); // call 2 after reset
    expect(rule.getMaxIndex(ctx)).not.toBeNull(); // call 3 — now active again
  });

  it('respects custom segmentIgnoreCount from abrSettings', () => {
    const customSettings = {
      ...DEFAULT_ABR_SETTINGS,
      rules: {
        ...DEFAULT_ABR_SETTINGS.rules,
        InsufficientBufferRule: {
          ...DEFAULT_ABR_SETTINGS.rules['InsufficientBufferRule']!,
          parameters: { throughputSafetyFactor: 0.7, segmentIgnoreCount: 4 },
        },
      },
    };
    const ctx = makeContext({ bufferSeconds: 0, abrSettings: customSettings });

    // Calls 1-4 should be ignored
    expect(rule.getMaxIndex(ctx)).toBeNull();
    expect(rule.getMaxIndex(ctx)).toBeNull();
    expect(rule.getMaxIndex(ctx)).toBeNull();
    expect(rule.getMaxIndex(ctx)).toBeNull();
    // Call 5 should intervene
    const result = rule.getMaxIndex(ctx);
    expect(result).not.toBeNull();
    expect(result!.representationIndex).toBe(0);
  });
});
