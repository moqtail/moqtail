import { SwitchRequestPriority, DEFAULT_ABR_SETTINGS } from '../types';
import type { AbrRule, RulesContext, SwitchRequest } from '../types';

const MIN_BUFFER_S = 0.5;
const DEFAULT_WEIGHTS = [0.4, 0.4, 0.4, 0.4]; // throughput, latency, rebuffer, switch
const ALPHA = 0.1; // SOM learning rate

interface Neuron {
  throughput: number;
  bufferLevel: number;
  rebufferCount: number;
  switchCount: number;
  qualityIndex: number;
}

export class LoLpRule implements AbrRule {
  readonly name = 'LoLPRule';

  private neurons: Neuron[] = [];
  private weights: number[] = [...DEFAULT_WEIGHTS];
  private prevActiveTrackIndex: number | null = null;
  private rebufferCount = 0;
  private switchCount = 0;

  getMaxIndex(context: RulesContext): SwitchRequest | null {
    const { tracks, activeTrackIndex, bufferSeconds, bandwidthBps, abrSettings } = context;

    // Emergency: critically low buffer
    if (bufferSeconds < MIN_BUFFER_S) {
      return {
        representationIndex: 0,
        priority: SwitchRequestPriority.STRONG,
        reason: 'lolp-emergency-low-buffer',
      };
    }

    // Cold start: no bandwidth estimate yet
    if (bandwidthBps === 0) {
      return null;
    }

    // Single track: nothing to decide
    if (tracks.length <= 1) {
      return null;
    }

    const { bandwidthSafetyFactor, stableBufferTime } = abrSettings;

    // Initialise neurons if needed (one per track)
    if (this.neurons.length !== tracks.length) {
      this.neurons = tracks.map((_, i) => ({
        throughput: (tracks[i]?.bitrate ?? 0),
        bufferLevel: stableBufferTime,
        rebufferCount: 0,
        switchCount: 0,
        qualityIndex: i,
      }));
    }

    // Track rebuffers (buffer empty) and switches
    if (bufferSeconds === 0) {
      this.rebufferCount += 1;
    }
    if (this.prevActiveTrackIndex !== null && this.prevActiveTrackIndex !== activeTrackIndex) {
      this.switchCount += 1;
    }
    this.prevActiveTrackIndex = activeTrackIndex;

    // Update current neuron with SOM learning
    const currentNeuron = this.neurons[activeTrackIndex];
    if (currentNeuron) {
      currentNeuron.throughput += ALPHA * (bandwidthBps - currentNeuron.throughput);
      currentNeuron.bufferLevel += ALPHA * (bufferSeconds - currentNeuron.bufferLevel);
      currentNeuron.rebufferCount += ALPHA * (this.rebufferCount - currentNeuron.rebufferCount);
      currentNeuron.switchCount += ALPHA * (this.switchCount - currentNeuron.switchCount);
    }

    // Target state: ideal conditions
    const target = {
      throughput: bandwidthBps * bandwidthSafetyFactor,
      bufferLevel: stableBufferTime,
      rebufferCount: 0,
      switchCount: 0,
    };

    const effectiveBandwidth = bandwidthBps * bandwidthSafetyFactor;

    // Find neurons with acceptable bitrate and compute weighted Euclidean distance to target
    let bestIndex = -1;
    let bestDistance = Infinity;

    for (let i = 0; i < tracks.length; i++) {
      const bitrate = tracks[i]?.bitrate ?? 0;

      // Skip if bitrate exceeds available bandwidth
      if (bitrate > effectiveBandwidth) {
        continue;
      }

      const neuron = this.neurons[i];
      if (!neuron) continue;

      const w = this.weights;
      const d =
        w[0]! * Math.pow(neuron.throughput - target.throughput, 2) +
        w[1]! * Math.pow(neuron.bufferLevel - target.bufferLevel, 2) +
        w[2]! * Math.pow(neuron.rebufferCount - target.rebufferCount, 2) +
        w[3]! * Math.pow(neuron.switchCount - target.switchCount, 2);

      if (d < bestDistance) {
        bestDistance = d;
        bestIndex = i;
      }
    }

    if (bestIndex === -1) {
      return null;
    }

    const rulePriority =
      abrSettings.rules['LoLPRule']?.priority ??
      DEFAULT_ABR_SETTINGS.rules['LoLPRule'].priority;

    return {
      representationIndex: bestIndex,
      priority: rulePriority ?? SwitchRequestPriority.DEFAULT,
      reason: 'lolp',
    };
  }

  reset(): void {
    this.neurons = [];
    this.weights = [...DEFAULT_WEIGHTS];
    this.prevActiveTrackIndex = null;
    this.rebufferCount = 0;
    this.switchCount = 0;
  }
}
