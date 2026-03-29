import { describe, it, expect, beforeEach } from 'vitest';
import { DroppedFramesRule } from '../DroppedFramesRule';
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

describe('DroppedFramesRule', () => {
  let rule: DroppedFramesRule;

  beforeEach(() => {
    rule = new DroppedFramesRule();
  });

  it('has the correct name', () => {
    expect(rule.name).toBe('DroppedFramesRule');
  });

  it('returns null when totalFrames is below minimumSampleSize', () => {
    // minimumSampleSize default = 375; totalFrames = 374 is not enough
    const ctx = makeContext({ totalFrames: 374, droppedFrames: 374 });
    expect(rule.getMaxIndex(ctx)).toBeNull();
  });

  it('returns null when totalFrames is 0 (cold start)', () => {
    const ctx = makeContext({ totalFrames: 0, droppedFrames: 0 });
    expect(rule.getMaxIndex(ctx)).toBeNull();
  });

  it('returns null when drop ratio is at exactly the threshold', () => {
    // ratio = 56 / 375 ≈ 0.1493... < 0.15 → null
    const ctx = makeContext({ totalFrames: 375, droppedFrames: 56 });
    expect(rule.getMaxIndex(ctx)).toBeNull();
  });

  it('returns null when drop ratio is below threshold', () => {
    // ratio = 10 / 400 = 0.025 < 0.15
    const ctx = makeContext({ totalFrames: 400, droppedFrames: 10 });
    expect(rule.getMaxIndex(ctx)).toBeNull();
  });

  it('downgrades by one when drop ratio exceeds threshold', () => {
    // ratio = 80 / 400 = 0.20 > 0.15, activeTrackIndex = 1 → target = 0
    const ctx = makeContext({
      totalFrames: 400,
      droppedFrames: 80,
      activeTrackIndex: 1,
    });
    const result = rule.getMaxIndex(ctx);
    expect(result).not.toBeNull();
    expect(result!.representationIndex).toBe(0);
  });

  it('downgrades from a higher index correctly', () => {
    // ratio = 100 / 400 = 0.25 > 0.15, activeTrackIndex = 2 → target = 1
    const ctx = makeContext({
      totalFrames: 400,
      droppedFrames: 100,
      activeTrackIndex: 2,
    });
    const result = rule.getMaxIndex(ctx);
    expect(result!.representationIndex).toBe(1);
  });

  it('does not downgrade below index 0', () => {
    // ratio = 100 / 400 = 0.25 > 0.15, activeTrackIndex = 0 → target = max(0, -1) = 0
    const ctx = makeContext({
      totalFrames: 400,
      droppedFrames: 100,
      activeTrackIndex: 0,
    });
    const result = rule.getMaxIndex(ctx);
    expect(result).not.toBeNull();
    expect(result!.representationIndex).toBe(0);
  });

  it('returns a SwitchRequest with DEFAULT priority and dropped-frames reason', () => {
    const ctx = makeContext({
      totalFrames: 400,
      droppedFrames: 80,
      activeTrackIndex: 2,
    });
    const result = rule.getMaxIndex(ctx);
    expect(result!.priority).toBe(SwitchRequestPriority.DEFAULT);
    expect(result!.reason).toBe('dropped-frames');
  });

  it('respects custom minimumSampleSize from abrSettings', () => {
    const customSettings = {
      ...DEFAULT_ABR_SETTINGS,
      rules: {
        ...DEFAULT_ABR_SETTINGS.rules,
        DroppedFramesRule: {
          ...DEFAULT_ABR_SETTINGS.rules['DroppedFramesRule'],
          parameters: {
            minimumSampleSize: 100,
            droppedFramesPercentageThreshold: 0.15,
          },
        },
      },
    };
    // totalFrames = 99 < 100 → null
    const ctx = makeContext({
      totalFrames: 99,
      droppedFrames: 99,
      abrSettings: customSettings,
    });
    expect(rule.getMaxIndex(ctx)).toBeNull();
  });

  it('respects custom droppedFramesPercentageThreshold from abrSettings', () => {
    const customSettings = {
      ...DEFAULT_ABR_SETTINGS,
      rules: {
        ...DEFAULT_ABR_SETTINGS.rules,
        DroppedFramesRule: {
          ...DEFAULT_ABR_SETTINGS.rules['DroppedFramesRule'],
          parameters: {
            minimumSampleSize: 375,
            droppedFramesPercentageThreshold: 0.05,
          },
        },
      },
    };
    // ratio = 30 / 400 = 0.075 > 0.05 (custom threshold) → downgrade
    const ctx = makeContext({
      totalFrames: 400,
      droppedFrames: 30,
      activeTrackIndex: 2,
      abrSettings: customSettings,
    });
    const result = rule.getMaxIndex(ctx);
    expect(result).not.toBeNull();
    expect(result!.representationIndex).toBe(1);
  });

  it('reset() does not throw (stateless)', () => {
    expect(() => rule.reset()).not.toThrow();
  });
});
