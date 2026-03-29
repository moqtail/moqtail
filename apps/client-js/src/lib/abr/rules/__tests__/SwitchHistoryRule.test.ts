import { describe, it, expect, beforeEach } from 'vitest';
import { SwitchHistoryRule } from '../SwitchHistoryRule';
import { DEFAULT_ABR_SETTINGS, SwitchRequestPriority } from '../../types';
import type { RulesContext, SwitchEvent } from '../../types';

function makeContext(overrides: Partial<RulesContext> = {}): RulesContext {
  return {
    tracks: [
      { name: '360p', bitrate: 500_000 },
      { name: '720p', bitrate: 1_500_000 },
      { name: '1080p', bitrate: 4_000_000 },
    ],
    activeTrackIndex: 2,
    bufferSeconds: 10,
    bandwidthBps: 5_000_000,
    fastEmaBps: 5_000_000,
    slowEmaBps: 5_000_000,
    droppedFrames: 0,
    totalFrames: 1000,
    segmentDurationS: 4,
    isLowLatency: false,
    switchHistory: [],
    abrSettings: { ...DEFAULT_ABR_SETTINGS },
    ...overrides,
  };
}

/**
 * Build a switch history that records `drops` downgrade events on a given track
 * and `noDrops` upgrade events to the same track.
 */
function makeHistory(
  trackName: string,
  drops: number,
  noDrops: number,
  fromTrack = '360p',
): SwitchEvent[] {
  const events: SwitchEvent[] = [];
  // Downgrade events: fromTrack = trackName (the higher quality being left)
  for (let i = 0; i < drops; i++) {
    events.push({
      ts: i,
      fromTrack: trackName,
      toTrack: fromTrack,
      reason: 'auto-downgrade',
      bufferAtSwitch: 5,
      emaBwAtSwitch: 1_000_000,
    });
  }
  // Upgrade events: toTrack = trackName (the quality being upgraded to)
  for (let i = 0; i < noDrops; i++) {
    events.push({
      ts: drops + i,
      fromTrack: fromTrack,
      toTrack: trackName,
      reason: 'auto-upgrade',
      bufferAtSwitch: 15,
      emaBwAtSwitch: 5_000_000,
    });
  }
  return events;
}

describe('SwitchHistoryRule', () => {
  let rule: SwitchHistoryRule;

  beforeEach(() => {
    rule = new SwitchHistoryRule();
  });

  it('has the correct name', () => {
    expect(rule.name).toBe('SwitchHistoryRule');
  });

  it('returns null when switch history is empty', () => {
    const ctx = makeContext({ switchHistory: [] });
    expect(rule.getMaxIndex(ctx)).toBeNull();
  });

  it('returns null when no representation has excessive drops', () => {
    // sampleSize=8, threshold=0.075
    // 1080p: 0 drops, 8 noDrops → ratio 0/8 = 0 → safe
    const history = makeHistory('1080p', 0, 8);
    const ctx = makeContext({ switchHistory: history, activeTrackIndex: 2 });
    expect(rule.getMaxIndex(ctx)).toBeNull();
  });

  it('returns null when there are fewer samples than sampleSize', () => {
    // Only 3 downgrade events on 1080p — below sampleSize of 8, rule should not trigger
    const history = makeHistory('1080p', 3, 40);
    // total samples = 43 >= 8, but drops/noDrops = 3/40 = 0.075 which equals threshold (not >, so safe)
    const ctx = makeContext({ switchHistory: history, activeTrackIndex: 2 });
    expect(rule.getMaxIndex(ctx)).toBeNull();
  });

  it('downgrades when a representation has too many drops', () => {
    // 1080p: 8 drops, 100 noDrops → ratio = 8/100 = 0.08 > 0.075 → unsafe
    // Should recommend index 1 (720p) as highest safe
    const history = makeHistory('1080p', 8, 100);
    const ctx = makeContext({ switchHistory: history, activeTrackIndex: 2 });
    const result = rule.getMaxIndex(ctx);
    expect(result).not.toBeNull();
    expect(result!.representationIndex).toBe(1);
    expect(result!.priority).toBe(SwitchRequestPriority.DEFAULT);
    expect(result!.reason).toBe('switch-history');
  });

  it('returns lowest safe index when multiple tracks are unsafe', () => {
    // Both 1080p and 720p have excessive drops → safe index should be 0 (360p)
    const history = [
      ...makeHistory('1080p', 8, 100),
      ...makeHistory('720p', 8, 100, '360p'),
    ];
    const ctx = makeContext({ switchHistory: history, activeTrackIndex: 2 });
    const result = rule.getMaxIndex(ctx);
    expect(result).not.toBeNull();
    expect(result!.representationIndex).toBe(0);
  });

  it('handles auto-emergency events as drops', () => {
    // auto-emergency increments drops on fromTrack
    const emergencyEvents: SwitchEvent[] = [];
    for (let i = 0; i < 8; i++) {
      emergencyEvents.push({
        ts: i,
        fromTrack: '1080p',
        toTrack: '360p',
        reason: 'auto-emergency',
        bufferAtSwitch: 1,
        emaBwAtSwitch: 500_000,
      });
    }
    // add upgrade events so noDrops > 0 for ratio check
    for (let i = 0; i < 100; i++) {
      emergencyEvents.push({
        ts: 8 + i,
        fromTrack: '360p',
        toTrack: '1080p',
        reason: 'auto-upgrade',
        bufferAtSwitch: 15,
        emaBwAtSwitch: 5_000_000,
      });
    }
    const ctx = makeContext({ switchHistory: emergencyEvents, activeTrackIndex: 2 });
    const result = rule.getMaxIndex(ctx);
    expect(result).not.toBeNull();
    expect(result!.representationIndex).toBe(1);
  });

  it('does not constrain when active track has no history', () => {
    // Only 720p has drops; active track 1080p (index 2) has no data → safe
    const history = makeHistory('720p', 8, 100, '360p');
    const ctx = makeContext({ switchHistory: history, activeTrackIndex: 2 });
    // 1080p has no stats — it's safe, so rule returns null (no constraint)
    expect(rule.getMaxIndex(ctx)).toBeNull();
  });

  it('reset() clears internal tracking', () => {
    // First call: set up unsafe 1080p
    const history = makeHistory('1080p', 8, 100);
    const ctx = makeContext({ switchHistory: history, activeTrackIndex: 2 });
    const before = rule.getMaxIndex(ctx);
    expect(before).not.toBeNull();

    // reset, then call with empty history — should return null
    rule.reset();
    const ctxEmpty = makeContext({ switchHistory: [], activeTrackIndex: 2 });
    expect(rule.getMaxIndex(ctxEmpty)).toBeNull();
  });
});
