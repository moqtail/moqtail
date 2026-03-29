import { SwitchRequestPriority, DEFAULT_ABR_SETTINGS } from '../types';
import type { AbrRule, RulesContext, SwitchRequest } from '../types';

export class AbandonRequestsRule implements AbrRule {
  readonly name = 'AbandonRequestsRule';

  #sampleCount = 0;

  getMaxIndex(context: RulesContext): SwitchRequest | null {
    const { tracks, activeTrackIndex, bufferSeconds, bandwidthBps, segmentDurationS, abrSettings } =
      context;

    const params =
      abrSettings.rules['AbandonRequestsRule']?.parameters ??
      DEFAULT_ABR_SETTINGS.rules['AbandonRequestsRule'].parameters;

    const abandonDurationMultiplier: number = (params['abandonDurationMultiplier'] as number) ?? 1.8;
    const minThroughputSamples: number =
      (params['minThroughputSamplesThreshold'] as number) ?? 6;

    const { stableBufferTime } = abrSettings;

    this.#sampleCount += 1;

    // Buffer is healthy — no need to abandon
    if (bufferSeconds >= stableBufferTime) {
      return null;
    }

    // Already at lowest quality — cannot go lower
    if (activeTrackIndex === 0) {
      return null;
    }

    // Not enough samples yet to make a reliable decision
    if (this.#sampleCount < minThroughputSamples) {
      return null;
    }

    // No bandwidth estimate yet
    if (bandwidthBps === 0) {
      return null;
    }

    const currentTrack = tracks[activeTrackIndex];
    const currentBitrate = currentTrack?.bitrate ?? 0;

    // Estimate how long the current segment would take to deliver
    const segmentSizeBits = currentBitrate * segmentDurationS;
    const estimatedDeliveryS = segmentSizeBits / bandwidthBps;

    // If delivery is projected to exceed the multiplier threshold, switch down
    if (estimatedDeliveryS > abandonDurationMultiplier * segmentDurationS) {
      // Find the lowest track index that can be sustained within current bandwidth
      let bestIndex = 0;
      for (let i = 0; i < tracks.length; i++) {
        const bitrate = tracks[i]?.bitrate ?? 0;
        const neededBps = bitrate > 0 ? bitrate / abandonDurationMultiplier : 0;
        if (neededBps <= bandwidthBps) {
          bestIndex = i;
        }
      }

      return {
        representationIndex: bestIndex,
        priority: SwitchRequestPriority.STRONG,
        reason: 'abandon-slow-delivery',
      };
    }

    return null;
  }

  reset(): void {
    this.#sampleCount = 0;
  }
}
