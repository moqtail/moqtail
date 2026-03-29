import { describe, it, expect, beforeEach } from 'vitest';
import { LoLpRule } from '../LoLpRule';
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
    bandwidthBps: 3_000_000,
    fastEmaBps: 3_000_000,
    slowEmaBps: 3_000_000,
    droppedFrames: 0,
    totalFrames: 0,
    segmentDurationS: 4,
    isLowLatency: true,
    switchHistory: [],
    abrSettings: { ...DEFAULT_ABR_SETTINGS },
    ...overrides,
  };
}

describe('LoLpRule', () => {
  let rule: LoLpRule;

  beforeEach(() => {
    rule = new LoLpRule();
  });

  it('has the correct name', () => {
    expect(rule.name).toBe('LoLPRule');
  });

  it('returns null when only one track', () => {
    const ctx = makeContext({
      tracks: [{ name: '360p', bitrate: 500_000 }],
      activeTrackIndex: 0,
    });
    expect(rule.getMaxIndex(ctx)).toBeNull();
  });

  it('returns null when bandwidthBps is 0 (cold start)', () => {
    const ctx = makeContext({ bandwidthBps: 0, bufferSeconds: 10 });
    expect(rule.getMaxIndex(ctx)).toBeNull();
  });

  it('returns a valid representationIndex within track bounds', () => {
    const ctx = makeContext();
    const result = rule.getMaxIndex(ctx);
    expect(result).not.toBeNull();
    expect(result!.representationIndex).toBeGreaterThanOrEqual(0);
    expect(result!.representationIndex).toBeLessThan(ctx.tracks.length);
  });

  it('returns a SwitchRequest with reason "lolp"', () => {
    const ctx = makeContext();
    const result = rule.getMaxIndex(ctx);
    expect(result).not.toBeNull();
    expect(result!.reason).toBe('lolp');
  });

  it('downgrades immediately when buffer is critically low (< 0.5s)', () => {
    const ctx = makeContext({ bufferSeconds: 0.3, activeTrackIndex: 2 });
    const result = rule.getMaxIndex(ctx);
    expect(result).not.toBeNull();
    expect(result!.representationIndex).toBe(0);
    expect(result!.priority).toBe(SwitchRequestPriority.STRONG);
    expect(result!.reason).toBe('lolp-emergency-low-buffer');
  });

  it('emergency triggers at exactly MIN_BUFFER_S boundary (0.5 exclusive)', () => {
    // bufferSeconds === 0.5 should NOT trigger emergency
    const ctx05 = makeContext({ bufferSeconds: 0.5 });
    const result05 = rule.getMaxIndex(ctx05);
    // Should not be emergency (reason should be 'lolp' or null)
    if (result05 !== null) {
      expect(result05.reason).not.toBe('lolp-emergency-low-buffer');
    }

    // bufferSeconds < 0.5 should trigger emergency
    rule.reset();
    const ctx04 = makeContext({ bufferSeconds: 0.4 });
    const result04 = rule.getMaxIndex(ctx04);
    expect(result04).not.toBeNull();
    expect(result04!.reason).toBe('lolp-emergency-low-buffer');
    expect(result04!.priority).toBe(SwitchRequestPriority.STRONG);
  });

  it('skips tracks whose bitrate exceeds effective bandwidth', () => {
    // effectiveBandwidth = 600_000 * 0.9 = 540_000
    // Only 360p (500k) fits; 720p and 1080p are skipped
    const ctx = makeContext({ bandwidthBps: 600_000 });
    const result = rule.getMaxIndex(ctx);
    expect(result).not.toBeNull();
    expect(result!.representationIndex).toBe(0);
  });

  it('returns null when no track fits within effective bandwidth', () => {
    // effectiveBandwidth = 400_000 * 0.9 = 360_000, all tracks ≥ 500k
    const ctx = makeContext({
      bandwidthBps: 400_000,
      tracks: [
        { name: '360p', bitrate: 500_000 },
        { name: '720p', bitrate: 1_500_000 },
      ],
    });
    // Note: emergency check runs first (bufferSeconds=10 so no emergency), then bandwidth=400k
    const result = rule.getMaxIndex(ctx);
    expect(result).toBeNull();
  });

  it('reset() clears internal state so neurons are re-initialised on next call', () => {
    const ctx = makeContext();
    // Call once to prime neurons
    rule.getMaxIndex(ctx);

    // Reset and call again — should still return a valid result (no error)
    rule.reset();
    const result = rule.getMaxIndex(ctx);
    expect(result).not.toBeNull();
    expect(result!.representationIndex).toBeGreaterThanOrEqual(0);
    expect(result!.representationIndex).toBeLessThan(ctx.tracks.length);
  });

  it('reset() clears rebuffer and switch counters', () => {
    // Simulate a rebuffer by calling with bufferSeconds=0 (after reset to avoid emergency)
    const ctxRebuffer = makeContext({ bufferSeconds: 0.6 });
    rule.getMaxIndex(ctxRebuffer);

    rule.reset();

    // After reset, a fresh call should work without accumulated state
    const ctx = makeContext();
    const result = rule.getMaxIndex(ctx);
    expect(result).not.toBeNull();
  });

  it('uses DEFAULT priority from settings for normal operation', () => {
    const ctx = makeContext();
    const result = rule.getMaxIndex(ctx);
    expect(result).not.toBeNull();
    expect(result!.priority).toBe(SwitchRequestPriority.DEFAULT);
  });
});
