import type { AbrRule, AbrSettings, RulesContext, SwitchRequest } from './types';
import { SwitchRequestPriority } from './types';
import { ThroughputRule } from './rules/ThroughputRule';
import { BolaRule } from './rules/BolaRule';
import { InsufficientBufferRule } from './rules/InsufficientBufferRule';
import { SwitchHistoryRule } from './rules/SwitchHistoryRule';
import { DroppedFramesRule } from './rules/DroppedFramesRule';
import { AbandonRequestsRule } from './rules/AbandonRequestsRule';
import { L2ARule } from './rules/L2ARule';
import { LoLpRule } from './rules/LoLpRule';

interface RuleEntry {
  rule: AbrRule;
  active: boolean;
}

export class AbrRulesCollection {
  readonly #rules: Map<string, RuleEntry>;
  #shouldUseBolaRule: boolean;

  constructor(settings: AbrSettings) {
    const allRules: AbrRule[] = [
      new ThroughputRule(),
      new BolaRule(),
      new InsufficientBufferRule(),
      new SwitchHistoryRule(),
      new DroppedFramesRule(),
      new AbandonRequestsRule(),
      new L2ARule(),
      new LoLpRule(),
    ];

    this.#rules = new Map();
    for (const rule of allRules) {
      const config = settings.rules[rule.name];
      this.#rules.set(rule.name, {
        rule,
        active: config?.active ?? false,
      });
    }

    this.#shouldUseBolaRule = false;
  }

  setRuleActive(ruleName: string, active: boolean): void {
    const entry = this.#rules.get(ruleName);
    if (!entry) return;

    if (active && !entry.active) {
      entry.rule.reset();
    }
    entry.active = active;
  }

  setShouldUseBolaRule(value: boolean): void {
    this.#shouldUseBolaRule = value;
  }

  isRuleActive(ruleName: string): boolean {
    return this.#rules.get(ruleName)?.active ?? false;
  }

  getBestPossibleSwitchRequest(context: RulesContext): SwitchRequest | null {
    const isLowLatencyMode =
      this.isRuleActive('L2ARule') || this.isRuleActive('LoLPRule');

    const requests: SwitchRequest[] = [];

    for (const [name, entry] of this.#rules) {
      if (!entry.active) continue;

      // Apply dynamic mutual exclusivity for BolaRule and ThroughputRule
      if (name === 'BolaRule') {
        // Skip BOLA in low-latency mode, or when !shouldUseBolaRule in normal mode
        if (isLowLatencyMode || !this.#shouldUseBolaRule) continue;
      } else if (name === 'ThroughputRule') {
        // Skip Throughput in low-latency mode, or when shouldUseBolaRule in normal mode
        if (isLowLatencyMode || this.#shouldUseBolaRule) continue;
      }

      const req = entry.rule.getMaxIndex(context);
      if (req !== null) {
        requests.push(req);
      }
    }

    return getMinSwitchRequest(requests);
  }
}

function getMinSwitchRequest(requests: SwitchRequest[]): SwitchRequest | null {
  if (requests.length === 0) return null;

  // Group by priority tier
  const tiers = new Map<SwitchRequestPriority, SwitchRequest[]>();

  for (const req of requests) {
    const bucket = tiers.get(req.priority);
    if (bucket) {
      bucket.push(req);
    } else {
      tiers.set(req.priority, [req]);
    }
  }

  // Evaluate from highest to lowest priority tier
  const priorityOrder: SwitchRequestPriority[] = [
    SwitchRequestPriority.STRONG,
    SwitchRequestPriority.DEFAULT,
    SwitchRequestPriority.WEAK,
  ];

  for (const priority of priorityOrder) {
    const bucket = tiers.get(priority);
    if (!bucket || bucket.length === 0) continue;

    // Pick the request with the lowest representationIndex within this tier
    let best = bucket[0]!;
    for (let i = 1; i < bucket.length; i++) {
      if (bucket[i]!.representationIndex < best.representationIndex) {
        best = bucket[i]!;
      }
    }
    return best;
  }

  return null;
}
