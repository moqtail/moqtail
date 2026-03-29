import { SwitchRequestPriority, DEFAULT_ABR_SETTINGS } from '../types';
import type { AbrRule, RulesContext, SwitchRequest } from '../types';

export class ThroughputRule implements AbrRule {
  readonly name = 'ThroughputRule';

  getMaxIndex(context: RulesContext): SwitchRequest | null {
    const { tracks, bandwidthBps, abrSettings } = context;

    // Cold start: no bandwidth estimate yet
    if (bandwidthBps === 0) {
      return null;
    }

    const { bandwidthSafetyFactor, minBitrate, maxBitrate } = abrSettings;
    const effectiveBandwidth = bandwidthBps * bandwidthSafetyFactor;

    // Find the highest track index whose bitrate fits within effective bandwidth
    // and respects minBitrate/maxBitrate clamps (-1 means unconstrained)
    let bestIndex = -1;

    for (let i = 0; i < tracks.length; i++) {
      const track = tracks[i];
      const bitrate = track.bitrate ?? 0;

      // Must fit within effective bandwidth
      if (bitrate > effectiveBandwidth) {
        continue;
      }

      // Respect minBitrate (-1 = no minimum)
      if (minBitrate !== -1 && bitrate < minBitrate) {
        continue;
      }

      // Respect maxBitrate (-1 = no maximum)
      if (maxBitrate !== -1 && bitrate > maxBitrate) {
        continue;
      }

      bestIndex = i;
    }

    if (bestIndex === -1) {
      return null;
    }

    const rulePriority =
      abrSettings.rules['ThroughputRule']?.priority ??
      DEFAULT_ABR_SETTINGS.rules['ThroughputRule'].priority;

    return {
      representationIndex: bestIndex,
      priority: rulePriority ?? SwitchRequestPriority.DEFAULT,
      reason: 'throughput',
    };
  }

  reset(): void {
    // Stateless rule — nothing to reset
  }
}
