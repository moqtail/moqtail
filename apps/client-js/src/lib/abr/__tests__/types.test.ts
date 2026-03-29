import { describe, it, expect } from 'vitest';
import { DEFAULT_ABR_SETTINGS, SwitchRequestPriority } from '../types';

describe('DEFAULT_ABR_SETTINGS', () => {
  it('has all 8 rules defined', () => {
    const ruleNames = Object.keys(DEFAULT_ABR_SETTINGS.rules);
    expect(ruleNames).toHaveLength(8);
    expect(ruleNames).toContain('ThroughputRule');
    expect(ruleNames).toContain('BolaRule');
    expect(ruleNames).toContain('InsufficientBufferRule');
    expect(ruleNames).toContain('SwitchHistoryRule');
    expect(ruleNames).toContain('DroppedFramesRule');
    expect(ruleNames).toContain('AbandonRequestsRule');
    expect(ruleNames).toContain('L2ARule');
    expect(ruleNames).toContain('LoLPRule');
  });

  it('DroppedFramesRule, L2ARule, LoLPRule are inactive by default', () => {
    expect(DEFAULT_ABR_SETTINGS.rules.DroppedFramesRule.active).toBe(false);
    expect(DEFAULT_ABR_SETTINGS.rules.L2ARule.active).toBe(false);
    expect(DEFAULT_ABR_SETTINGS.rules.LoLPRule.active).toBe(false);
  });

  it('all rules have DEFAULT priority', () => {
    for (const rule of Object.values(DEFAULT_ABR_SETTINGS.rules)) {
      expect(rule.priority).toBe(SwitchRequestPriority.DEFAULT);
    }
  });

  it('bufferTimeDefault is 18', () => {
    expect(DEFAULT_ABR_SETTINGS.bufferTimeDefault).toBe(18);
  });
});
