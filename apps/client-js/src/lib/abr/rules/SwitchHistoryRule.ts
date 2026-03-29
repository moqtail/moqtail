import { SwitchRequestPriority, DEFAULT_ABR_SETTINGS } from '../types';
import type { AbrRule, RulesContext, SwitchRequest } from '../types';

interface TrackStats {
  drops: number;
  noDrops: number;
}

export class SwitchHistoryRule implements AbrRule {
  readonly name = 'SwitchHistoryRule';

  private trackStats: Map<string, TrackStats> = new Map();

  getMaxIndex(context: RulesContext): SwitchRequest | null {
    const { tracks, activeTrackIndex, switchHistory, abrSettings } = context;

    const ruleConfig = abrSettings.rules['SwitchHistoryRule'];
    const defaultConfig = DEFAULT_ABR_SETTINGS.rules['SwitchHistoryRule'];
    const sampleSize: number =
      ruleConfig?.parameters?.['sampleSize'] ?? defaultConfig.parameters['sampleSize'];
    const switchPercentageThreshold: number =
      ruleConfig?.parameters?.['switchPercentageThreshold'] ??
      defaultConfig.parameters['switchPercentageThreshold'];

    // Rebuild per-track stats from the full switch history on each call
    this.trackStats = new Map();

    for (const event of switchHistory) {
      if (event.reason === 'auto-downgrade' || event.reason === 'auto-emergency') {
        // Downgrade/emergency: record a drop against the track being left
        const stats = this.trackStats.get(event.fromTrack) ?? { drops: 0, noDrops: 0 };
        stats.drops += 1;
        this.trackStats.set(event.fromTrack, stats);
      } else if (event.reason === 'auto-upgrade') {
        // Upgrade: record a successful "no-drop" visit on the target track
        const stats = this.trackStats.get(event.toTrack) ?? { drops: 0, noDrops: 0 };
        stats.noDrops += 1;
        this.trackStats.set(event.toTrack, stats);
      }
    }

    const isUnsafe = (trackName: string): boolean => {
      const stats = this.trackStats.get(trackName);
      if (!stats) return false;
      const total = stats.drops + stats.noDrops;
      return (
        total >= sampleSize &&
        stats.noDrops > 0 &&
        stats.drops / stats.noDrops > switchPercentageThreshold
      );
    };

    // Walk from activeTrackIndex down, looking for the highest safe index
    let highestSafeIndex: number | null = null;

    for (let i = activeTrackIndex; i >= 0; i--) {
      const trackName = tracks[i]?.name;
      if (trackName && !isUnsafe(trackName)) {
        highestSafeIndex = i;
        break;
      }
    }

    // If the current active track is already safe, no constraint is needed
    if (highestSafeIndex === activeTrackIndex) {
      return null;
    }

    // If no safe index was found, or nothing is lower — return null
    if (highestSafeIndex === null) {
      return null;
    }

    const rulePriority = ruleConfig?.priority ?? defaultConfig.priority;

    return {
      representationIndex: highestSafeIndex,
      priority: rulePriority ?? SwitchRequestPriority.DEFAULT,
      reason: 'switch-history',
    };
  }

  reset(): void {
    this.trackStats = new Map();
  }
}
