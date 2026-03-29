import { describe, it, expect, beforeEach, vi } from 'vitest';
import { AbrRulesCollection } from '../AbrRulesCollection';
import {
  DEFAULT_ABR_SETTINGS,
  SwitchRequestPriority,
} from '../types';
import type { AbrSettings, RulesContext, SwitchRequest } from '../types';

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function makeSettings(overrides: Partial<AbrSettings> = {}): AbrSettings {
  return {
    ...DEFAULT_ABR_SETTINGS,
    rules: { ...DEFAULT_ABR_SETTINGS.rules },
    ...overrides,
  };
}

function makeContext(overrides: Partial<RulesContext> = {}): RulesContext {
  return {
    tracks: [
      { name: '360p', bitrate: 500_000 },
      { name: '720p', bitrate: 1_500_000 },
      { name: '1080p', bitrate: 4_000_000 },
    ],
    activeTrackIndex: 0,
    bufferSeconds: 20, // healthy buffer — avoids InsufficientBufferRule triggering
    bandwidthBps: 10_000_000,
    fastEmaBps: 10_000_000,
    slowEmaBps: 10_000_000,
    droppedFrames: 0,
    totalFrames: 0,
    segmentDurationS: 4,
    isLowLatency: false,
    switchHistory: [],
    abrSettings: makeSettings(),
    ...overrides,
  };
}

// Build a settings object with only the specified rule(s) active
function settingsWithOnlyRules(ruleNames: string[]): AbrSettings {
  const rules: AbrSettings['rules'] = {};
  for (const [name, config] of Object.entries(DEFAULT_ABR_SETTINGS.rules)) {
    rules[name] = { ...config, active: ruleNames.includes(name) };
  }
  return { ...DEFAULT_ABR_SETTINGS, rules };
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

describe('AbrRulesCollection', () => {
  describe('constructor', () => {
    it('initialises all 8 rules from settings', () => {
      const collection = new AbrRulesCollection(makeSettings());
      const ruleNames = [
        'ThroughputRule',
        'BolaRule',
        'InsufficientBufferRule',
        'SwitchHistoryRule',
        'DroppedFramesRule',
        'AbandonRequestsRule',
        'L2ARule',
        'LoLPRule',
      ];
      for (const name of ruleNames) {
        // isRuleActive reflects the settings.rules[name].active flag
        expect(collection.isRuleActive(name)).toBe(
          DEFAULT_ABR_SETTINGS.rules[name]!.active,
        );
      }
    });
  });

  describe('isRuleActive', () => {
    it('returns false for unknown rule names', () => {
      const collection = new AbrRulesCollection(makeSettings());
      expect(collection.isRuleActive('NonExistentRule')).toBe(false);
    });
  });

  describe('setRuleActive', () => {
    it('enables a disabled rule', () => {
      const collection = new AbrRulesCollection(makeSettings());
      expect(collection.isRuleActive('L2ARule')).toBe(false); // off by default
      collection.setRuleActive('L2ARule', true);
      expect(collection.isRuleActive('L2ARule')).toBe(true);
    });

    it('disables an active rule', () => {
      const collection = new AbrRulesCollection(makeSettings());
      expect(collection.isRuleActive('ThroughputRule')).toBe(true);
      collection.setRuleActive('ThroughputRule', false);
      expect(collection.isRuleActive('ThroughputRule')).toBe(false);
    });

    it('disabling all rules causes getBestPossibleSwitchRequest to return null', () => {
      const settings = makeSettings();
      const allNames = Object.keys(settings.rules);
      const collection = new AbrRulesCollection(settings);
      for (const name of allNames) {
        collection.setRuleActive(name, false);
      }
      const ctx = makeContext({ abrSettings: settings });
      expect(collection.getBestPossibleSwitchRequest(ctx)).toBeNull();
    });

    it('does not throw for unknown rule names', () => {
      const collection = new AbrRulesCollection(makeSettings());
      expect(() => collection.setRuleActive('NoSuchRule', true)).not.toThrow();
    });
  });

  describe('getBestPossibleSwitchRequest — returns a result from active rules', () => {
    it('returns a SwitchRequest when at least one rule is active and fires', () => {
      // Only ThroughputRule active; shouldUseBolaRule=false (default) → ThroughputRule runs
      const settings = settingsWithOnlyRules(['ThroughputRule']);
      const collection = new AbrRulesCollection(settings);
      const ctx = makeContext({ abrSettings: settings, bandwidthBps: 10_000_000 });
      const result = collection.getBestPossibleSwitchRequest(ctx);
      expect(result).not.toBeNull();
      expect(result!.representationIndex).toBeGreaterThanOrEqual(0);
    });
  });

  describe('getBestPossibleSwitchRequest — DYNAMIC mutual exclusivity', () => {
    it('only runs ThroughputRule when shouldUseBolaRule is false', () => {
      const settings = settingsWithOnlyRules(['ThroughputRule', 'BolaRule']);
      const collection = new AbrRulesCollection(settings);
      collection.setShouldUseBolaRule(false); // ThroughputRule should run, BolaRule should not

      const ctx = makeContext({ abrSettings: settings, bandwidthBps: 10_000_000 });

      // Spy on both rules through getMaxIndex to confirm only Throughput fires
      // We verify indirectly: ThroughputRule returns index=2 at 10Mbps (all tracks fit),
      // BolaRule in startup with buffer=20 > segmentDuration transitions to STEADY.
      // If only Throughput runs we get a valid result; if BOLA also runs it still
      // picks the LOWEST (min arbitration), so we verify the call pattern instead.
      const result = collection.getBestPossibleSwitchRequest(ctx);
      expect(result).not.toBeNull();
      // ThroughputRule result has reason 'throughput'
      expect(result!.reason).toBe('throughput');
    });

    it('only runs BolaRule when shouldUseBolaRule is true', () => {
      const settings = settingsWithOnlyRules(['ThroughputRule', 'BolaRule']);
      const collection = new AbrRulesCollection(settings);
      collection.setShouldUseBolaRule(true); // BolaRule should run, ThroughputRule should not

      const ctx = makeContext({ abrSettings: settings, bandwidthBps: 10_000_000 });
      const result = collection.getBestPossibleSwitchRequest(ctx);
      expect(result).not.toBeNull();
      // BolaRule reasons start with 'BOLA'
      expect(result!.reason).toMatch(/^BOLA/);
    });

    it('skips both BolaRule and ThroughputRule when L2A is active (low-latency mode)', () => {
      // Activate L2A and have ThroughputRule + BolaRule active too
      const settings = settingsWithOnlyRules(['ThroughputRule', 'BolaRule', 'L2ARule']);
      const collection = new AbrRulesCollection(settings);
      // shouldUseBolaRule doesn't matter — low-latency bypasses both

      const ctx = makeContext({ abrSettings: settings, bandwidthBps: 10_000_000 });
      const result = collection.getBestPossibleSwitchRequest(ctx);
      // L2ARule should fire; result should come from L2A (reason contains 'l2a')
      expect(result).not.toBeNull();
      expect(result!.reason).toMatch(/l2a/i);
    });

    it('skips both BolaRule and ThroughputRule when LoLP is active (low-latency mode)', () => {
      const settings = settingsWithOnlyRules(['ThroughputRule', 'BolaRule', 'LoLPRule']);
      const collection = new AbrRulesCollection(settings);

      // Give enough buffer to avoid LoLp's emergency path (bufferSeconds >= 0.5)
      const ctx = makeContext({ abrSettings: settings, bandwidthBps: 10_000_000, bufferSeconds: 20 });
      const result = collection.getBestPossibleSwitchRequest(ctx);
      expect(result).not.toBeNull();
      // LoLPRule has reason 'lolp'
      expect(result!.reason).toBe('lolp');
    });
  });

  describe('getMinSwitchRequest arbitration', () => {
    it('STRONG priority overrides DEFAULT priority', () => {
      // We need two active rules that produce different priorities.
      // InsufficientBufferRule with bufferSeconds=0 (after warm-up) returns STRONG.
      // ThroughputRule returns DEFAULT.
      // Wire up InsufficientBufferRule to fire with STRONG by setting bufferSeconds=0
      // but we need to pass the warm-up (segmentIgnoreCount=2 calls).

      const settings = settingsWithOnlyRules(['ThroughputRule', 'InsufficientBufferRule']);
      const collection = new AbrRulesCollection(settings);

      // Warm up InsufficientBufferRule (needs >2 calls before it fires)
      const warmupCtx = makeContext({ abrSettings: settings, bufferSeconds: 0, bandwidthBps: 10_000_000 });
      collection.getBestPossibleSwitchRequest(warmupCtx);
      collection.getBestPossibleSwitchRequest(warmupCtx);

      // Now on the 3rd call, InsufficientBufferRule fires with STRONG (bufferSeconds=0)
      const ctx = makeContext({ abrSettings: settings, bufferSeconds: 0, bandwidthBps: 10_000_000 });
      const result = collection.getBestPossibleSwitchRequest(ctx);
      expect(result).not.toBeNull();
      expect(result!.priority).toBe(SwitchRequestPriority.STRONG);
      expect(result!.representationIndex).toBe(0);
    });

    it('takes minimum representationIndex within the same priority tier', () => {
      // Use two rules at DEFAULT priority that return different representationIndex values,
      // then verify we get back the lower one.
      // ThroughputRule at 10Mbps → index 2 (all tracks fit)
      // We can craft a scenario where SwitchHistoryRule returns a lower index.
      // Alternatively, use a mock approach by monkey-patching via spies.
      //
      // Simplest: enable only ThroughputRule (returns index=2) and InsufficientBufferRule
      // with a low buffer. After warm-up, InsufficientBufferRule returns a lower index
      // at DEFAULT priority (bufferSeconds between 0 and stableBufferTime).

      const settings = settingsWithOnlyRules(['ThroughputRule', 'InsufficientBufferRule']);
      const collection = new AbrRulesCollection(settings);

      // Warm up InsufficientBufferRule
      const warmupCtx = makeContext({
        abrSettings: settings,
        bufferSeconds: 5, // low buffer → triggers insufficient buffer rule
        bandwidthBps: 10_000_000,
        segmentDurationS: 4,
      });
      collection.getBestPossibleSwitchRequest(warmupCtx); // call 1
      collection.getBestPossibleSwitchRequest(warmupCtx); // call 2

      // Call 3: InsufficientBufferRule fires with DEFAULT priority (bufferSeconds ≥ 0.5)
      // cappedBps = 10M * 0.7 * (5/4) = 8.75M → bestIndex = 2 (4M fits)
      // ThroughputRule: effectiveBandwidth = 10M * 0.9 = 9M → bestIndex = 2
      // Both return index 2 at DEFAULT — min is 2.
      // Let's use a lower bandwidth so they diverge:
      // bandwidthBps = 1_600_000:
      //   ThroughputRule: effective = 1.44M → index 1 (1.5M doesn't fit, 500k does) → index 0
      //   Wait, 1.44M > 500k but < 1.5M → index 0
      //   InsufficientBufferRule: cappedBps = 1.6M * 0.7 * (5/4) = 1.4M → index 0 (500k fits)
      // Both would return index 0. Let's try bandwidthBps = 2_000_000:
      //   ThroughputRule: effective = 1.8M → 500k fits, 1.5M fits, 4M no → index 1
      //   InsufficientBufferRule: cappedBps = 2M * 0.7 * (5/4) = 1.75M → 500k fits, 1.5M fits → index 1
      // Hmm, still the same. Try a different buffer level to make them diverge:
      //   bufferSeconds = 2, bandwidthBps = 5_000_000:
      //   ThroughputRule: effective = 4.5M → all fit → index 2
      //   InsufficientBufferRule: cappedBps = 5M * 0.7 * (2/4) = 1.75M → index 1 (1.5M fits)
      // They differ: ThroughputRule=2, InsufficientBufferRule=1 → min should be 1
      const ctx = makeContext({
        abrSettings: settings,
        bufferSeconds: 2,
        bandwidthBps: 5_000_000,
        segmentDurationS: 4,
      });
      const result = collection.getBestPossibleSwitchRequest(ctx);
      expect(result).not.toBeNull();
      // Min of index 2 (throughput) and index 1 (insufficient buffer) = 1
      expect(result!.representationIndex).toBe(1);
      expect(result!.priority).toBe(SwitchRequestPriority.DEFAULT);
    });
  });

  describe('integration — SwitchRequest from active rules', () => {
    it('returns null when no active rules return a request', () => {
      // Only SwitchHistoryRule active with no problematic history — returns null
      const settings = settingsWithOnlyRules(['SwitchHistoryRule']);
      const collection = new AbrRulesCollection(settings);
      // SwitchHistoryRule returns null when activeTrack is already the best safe index
      const ctx = makeContext({ abrSettings: settings });
      const result = collection.getBestPossibleSwitchRequest(ctx);
      expect(result).toBeNull();
    });
  });
});
