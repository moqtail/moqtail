export type Track = {
  name: string;
  bitrate?: number;
  role?: string;
  codec?: string;
  width?: number;
  height?: number;
  framerate?: number;
};

export type SwitchReason =
  | 'auto-upgrade'
  | 'auto-downgrade'
  | 'auto-emergency'
  | 'manual';

export interface SwitchEvent {
  ts: number;
  fromTrack: string;
  toTrack: string;
  reason: SwitchReason;
  bufferAtSwitch: number;
  emaBwAtSwitch: number;
}

export enum SwitchRequestPriority {
  WEAK = 0,
  DEFAULT = 0.5,
  STRONG = 1,
}

export interface SwitchRequest {
  representationIndex: number;
  priority: SwitchRequestPriority;
  reason: string;
}

export interface RuleConfig {
  active: boolean;
  priority: SwitchRequestPriority;
  parameters: Record<string, number>;
}

export interface AbrSettings {
  fastSwitching: boolean;
  videoAutoSwitch: boolean;
  bufferTimeDefault: number;
  stableBufferTime: number;
  bandwidthSafetyFactor: number;
  ewma: {
    throughputFastHalfLifeSeconds: number;
    throughputSlowHalfLifeSeconds: number;
  };
  initialBitrate: number;
  minBitrate: number;
  maxBitrate: number;
  rules: Record<string, RuleConfig>;
}

export const DEFAULT_ABR_SETTINGS: AbrSettings = {
  fastSwitching: false,
  videoAutoSwitch: true,
  bufferTimeDefault: 18,
  stableBufferTime: 18,
  bandwidthSafetyFactor: 0.9,
  ewma: {
    throughputFastHalfLifeSeconds: 3,
    throughputSlowHalfLifeSeconds: 8,
  },
  initialBitrate: -1,
  minBitrate: -1,
  maxBitrate: -1,
  rules: {
    ThroughputRule: { active: true, priority: SwitchRequestPriority.DEFAULT, parameters: {} },
    BolaRule: { active: true, priority: SwitchRequestPriority.DEFAULT, parameters: {} },
    InsufficientBufferRule: {
      active: true,
      priority: SwitchRequestPriority.DEFAULT,
      parameters: { throughputSafetyFactor: 0.7, segmentIgnoreCount: 2 },
    },
    SwitchHistoryRule: {
      active: true,
      priority: SwitchRequestPriority.DEFAULT,
      parameters: { sampleSize: 8, switchPercentageThreshold: 0.075 },
    },
    DroppedFramesRule: {
      active: false,
      priority: SwitchRequestPriority.DEFAULT,
      parameters: { minimumSampleSize: 375, droppedFramesPercentageThreshold: 0.15 },
    },
    AbandonRequestsRule: {
      active: true,
      priority: SwitchRequestPriority.DEFAULT,
      parameters: {
        abandonDurationMultiplier: 1.8,
        minThroughputSamplesThreshold: 6,
        minSegmentDownloadTimeThresholdInMs: 500,
      },
    },
    L2ARule: { active: false, priority: SwitchRequestPriority.DEFAULT, parameters: {} },
    LoLPRule: { active: false, priority: SwitchRequestPriority.DEFAULT, parameters: {} },
  },
};

export interface RulesContext {
  tracks: Track[];
  activeTrackIndex: number;
  bufferSeconds: number;
  bandwidthBps: number;
  fastEmaBps: number;
  slowEmaBps: number;
  droppedFrames: number;
  totalFrames: number;
  segmentDurationS: number;
  isLowLatency: boolean;
  switchHistory: SwitchEvent[];
  abrSettings: AbrSettings;
}

export interface AbrRule {
  readonly name: string;
  getMaxIndex(context: RulesContext): SwitchRequest | null;
  reset(): void;
}
