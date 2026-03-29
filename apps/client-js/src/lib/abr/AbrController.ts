import type { Player } from '@/lib/player';
import type { AbrRulesCollection } from './AbrRulesCollection';
import {
  type AbrSettings,
  type RulesContext,
  type SwitchEvent,
  type SwitchReason,
  type Track,
} from './types';

export interface AbrMetrics {
  bandwidthBps: number;
  fastEmaBps: number;
  slowEmaBps: number;
  bufferSeconds: number;
  activeTrack: string | null;
  activeTrackIndex: number;
  droppedFrames: number;
  totalFrames: number;
  playbackRate: number;
  deliveryTimeMs: number;
  lastObjectBytes: number;
  switchHistory: SwitchEvent[];
  mode: 'auto' | 'manual';
}

const MAX_HISTORY = 60;

export class AbrController {
  #player: Pick<Player, 'getMetrics' | 'switchTrack'>;
  #rulesCollection: AbrRulesCollection;
  #tracks: Track[];
  #settings: AbrSettings;
  #onMetricsUpdate: (m: AbrMetrics) => void;

  #intervalId: ReturnType<typeof setInterval> | null = null;
  #switching = false;
  #usingBolaRule = false;
  #switchHistory: SwitchEvent[] = [];

  constructor(
    player: Pick<Player, 'getMetrics' | 'switchTrack'>,
    rulesCollection: AbrRulesCollection,
    tracks: Track[],
    settings: AbrSettings,
    onMetricsUpdate: (m: AbrMetrics) => void,
  ) {
    this.#player = player;
    this.#rulesCollection = rulesCollection;
    // Sort ascending by bitrate — index 0 = lowest quality, last = highest
    this.#tracks = [...tracks].sort((a, b) => (a.bitrate ?? 0) - (b.bitrate ?? 0));
    this.#settings = settings;
    this.#onMetricsUpdate = onMetricsUpdate;
  }

  start(): void {
    if (this.#intervalId !== null) return;
    this.#intervalId = setInterval(() => this._tick(), 250);
  }

  stop(): void {
    if (this.#intervalId !== null) {
      clearInterval(this.#intervalId);
      this.#intervalId = null;
    }
  }

  updateSettings(settings: AbrSettings): void {
    this.#settings = settings;
  }

  releaseSwitchingGuard(): void {
    this.#switching = false;
  }

  manualSwitch(trackName: string): void {
    const fromTrack = this.#player.getMetrics().activeTrack ?? '';
    this.#recordHistory(fromTrack, trackName, 'manual', 0, 0);
    void this.#player.switchTrack(trackName);
  }

  getHistory(): SwitchEvent[] {
    return [...this.#switchHistory];
  }

  _tick(): void {
    const raw = this.#player.getMetrics();
    const {
      bandwidthBps,
      fastEmaBps,
      slowEmaBps,
      bufferSeconds,
      activeTrack,
      droppedFrames,
      totalFrames,
      playbackRate,
      deliveryTimeMs,
      lastObjectBytes,
    } = raw;

    // Find the active track index in the sorted tracks array
    const activeTrackIndex = activeTrack ? this.#tracks.findIndex(t => t.name === activeTrack) : -1;

    const mode: 'auto' | 'manual' = this.#settings.videoAutoSwitch ? 'auto' : 'manual';

    const metrics: AbrMetrics = {
      bandwidthBps,
      fastEmaBps,
      slowEmaBps,
      bufferSeconds,
      activeTrack,
      activeTrackIndex,
      droppedFrames,
      totalFrames,
      playbackRate,
      deliveryTimeMs,
      lastObjectBytes,
      switchHistory: [...this.#switchHistory],
      mode,
    };

    this.#onMetricsUpdate(metrics);

    // Manual mode — don't make automatic decisions
    if (!this.#settings.videoAutoSwitch) return;

    // Switching guard — wait for previous switch to complete
    if (this.#switching) return;

    // Update DYNAMIC strategy based on buffer level
    this.#updateDynamicStrategy(bufferSeconds);

    // Build context for rules
    const context: RulesContext = {
      tracks: this.#tracks,
      activeTrackIndex: activeTrackIndex >= 0 ? activeTrackIndex : 0,
      bufferSeconds,
      bandwidthBps,
      fastEmaBps,
      slowEmaBps,
      droppedFrames,
      totalFrames,
      segmentDurationS: 1,
      isLowLatency: false,
      switchHistory: [...this.#switchHistory],
      abrSettings: this.#settings,
    };

    const switchRequest = this.#rulesCollection.getBestPossibleSwitchRequest(context);
    if (switchRequest === null) return;

    const targetIndex = switchRequest.representationIndex;
    const currentIndex = activeTrackIndex >= 0 ? activeTrackIndex : 0;

    // Only switch if the target differs from current
    if (targetIndex === currentIndex) return;

    const targetTrack = this.#tracks[targetIndex];
    if (!targetTrack) return;

    // Determine switch reason
    const currentBitrate =
      activeTrackIndex >= 0 ? (this.#tracks[activeTrackIndex]?.bitrate ?? 0) : 0;
    const targetBitrate = targetTrack.bitrate ?? 0;
    let reason: SwitchReason;
    if (targetBitrate < currentBitrate) {
      reason = switchRequest.reason.toLowerCase().includes('emergency')
        ? 'auto-emergency'
        : 'auto-downgrade';
    } else {
      reason = 'auto-upgrade';
    }

    // Activate switching guard, record history, and switch
    this.#switching = true;
    this.#recordHistory(activeTrack ?? '', targetTrack.name, reason, bufferSeconds, fastEmaBps);
    void this.#player.switchTrack(targetTrack.name);
  }

  #updateDynamicStrategy(bufferLevel: number): void {
    // Skip if L2A or LoLP is active — they manage strategy themselves
    if (
      this.#rulesCollection.isRuleActive('L2ARule') ||
      this.#rulesCollection.isRuleActive('LoLPRule')
    ) {
      return;
    }

    const switchOnThreshold = this.#settings.bufferTimeDefault; // 18s by default
    const switchOffThreshold = 0.5 * this.#settings.bufferTimeDefault; // 9s by default

    // Hysteresis: use the current state to pick which threshold to compare against
    this.#usingBolaRule =
      bufferLevel >= (this.#usingBolaRule ? switchOffThreshold : switchOnThreshold);

    this.#rulesCollection.setShouldUseBolaRule(this.#usingBolaRule);
  }

  #recordHistory(
    fromTrack: string,
    toTrack: string,
    reason: SwitchReason,
    bufferAtSwitch: number,
    emaBwAtSwitch: number,
  ): void {
    const event: SwitchEvent = {
      ts: Date.now(),
      fromTrack,
      toTrack,
      reason,
      bufferAtSwitch,
      emaBwAtSwitch,
    };

    this.#switchHistory.push(event);

    if (this.#switchHistory.length > MAX_HISTORY) {
      this.#switchHistory.shift();
    }
  }
}
