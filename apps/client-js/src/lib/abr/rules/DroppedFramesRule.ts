import { SwitchRequestPriority, DEFAULT_ABR_SETTINGS } from '../types';
import type { AbrRule, RulesContext, SwitchRequest } from '../types';

export class DroppedFramesRule implements AbrRule {
  readonly name = 'DroppedFramesRule';

  getMaxIndex(context: RulesContext): SwitchRequest | null {
    const { activeTrackIndex, droppedFrames, totalFrames, abrSettings } = context;

    const ruleConfig =
      abrSettings.rules['DroppedFramesRule'] ??
      DEFAULT_ABR_SETTINGS.rules['DroppedFramesRule'];

    const minimumSampleSize: number =
      ruleConfig.parameters['minimumSampleSize'] ??
      DEFAULT_ABR_SETTINGS.rules['DroppedFramesRule'].parameters['minimumSampleSize'];

    const droppedFramesPercentageThreshold: number =
      ruleConfig.parameters['droppedFramesPercentageThreshold'] ??
      DEFAULT_ABR_SETTINGS.rules['DroppedFramesRule'].parameters[
        'droppedFramesPercentageThreshold'
      ];

    // Not enough data yet
    if (totalFrames < minimumSampleSize) {
      return null;
    }

    const dropRatio = droppedFrames / totalFrames;

    // Drop rate is acceptable
    if (dropRatio <= droppedFramesPercentageThreshold) {
      return null;
    }

    // Drop rate exceeds threshold — downgrade one level, but not below 0
    const targetIndex = Math.max(0, activeTrackIndex - 1);

    const rulePriority =
      ruleConfig.priority ?? SwitchRequestPriority.DEFAULT;

    return {
      representationIndex: targetIndex,
      priority: rulePriority,
      reason: 'dropped-frames',
    };
  }

  reset(): void {
    // Stateless rule — dropped frame counts come from browser API via RulesContext
  }
}
