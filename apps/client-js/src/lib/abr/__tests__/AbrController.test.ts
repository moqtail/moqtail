import { describe, it, expect, vi } from 'vitest';
import { AbrController } from '../AbrController';
import type { AbrMetrics } from '../AbrController';
import { AbrRulesCollection } from '../AbrRulesCollection';
import { DEFAULT_ABR_SETTINGS } from '../types';
import type { AbrSettings, Track } from '../types';

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

type MockPlayer = {
  getMetrics: ReturnType<typeof vi.fn>;
  switchTrack: ReturnType<typeof vi.fn>;
};

function makeTracks(): Track[] {
  return [
    { name: '360p', bitrate: 500_000 },
    { name: '720p', bitrate: 1_500_000 },
    { name: '1080p', bitrate: 4_000_000 },
  ];
}

function makeSettings(overrides: Partial<AbrSettings> = {}): AbrSettings {
  return {
    ...DEFAULT_ABR_SETTINGS,
    rules: { ...DEFAULT_ABR_SETTINGS.rules },
    ...overrides,
  };
}

function makePlayerMetrics(overrides: Partial<ReturnType<MockPlayer['getMetrics']>> = {}) {
  return {
    bandwidthBps: 10_000_000,
    fastEmaBps: 10_000_000,
    slowEmaBps: 10_000_000,
    bufferSeconds: 5,
    activeTrack: '360p',
    droppedFrames: 0,
    totalFrames: 1000,
    playbackRate: 1,
    deliveryTimeMs: 50,
    lastObjectBytes: 8000,
    ...overrides,
  };
}

function makeController(
  playerOverrides: Partial<ReturnType<typeof makePlayerMetrics>> = {},
  settingsOverrides: Partial<AbrSettings> = {},
): {
  controller: AbrController;
  player: MockPlayer;
  collection: AbrRulesCollection;
  metrics: AbrMetrics[];
} {
  const tracks = makeTracks();
  const settings = makeSettings(settingsOverrides);
  const collection = new AbrRulesCollection(settings);
  const player: MockPlayer = {
    getMetrics: vi.fn().mockReturnValue(makePlayerMetrics(playerOverrides)),
    switchTrack: vi.fn().mockResolvedValue(undefined),
  };
  const capturedMetrics: AbrMetrics[] = [];
  const controller = new AbrController(player, collection, tracks, settings, m =>
    capturedMetrics.push(m),
  );
  return { controller, player, collection, metrics: capturedMetrics };
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

describe('AbrController', () => {
  describe('metrics emission', () => {
    it('emits metrics on every tick regardless of mode', () => {
      const { controller, metrics } = makeController(
        { bufferSeconds: 5, activeTrack: '360p' },
        { videoAutoSwitch: false },
      );

      controller._tick();
      expect(metrics).toHaveLength(1);
      expect(metrics[0]!.bandwidthBps).toBe(10_000_000);
      expect(metrics[0]!.activeTrack).toBe('360p');
      expect(metrics[0]!.activeTrackIndex).toBe(0);
      expect(metrics[0]!.mode).toBe('manual');

      controller._tick();
      expect(metrics).toHaveLength(2);
    });

    it('emits mode=auto when videoAutoSwitch is true', () => {
      const { controller, metrics } = makeController(
        { bufferSeconds: 5 },
        { videoAutoSwitch: true },
      );
      controller._tick();
      expect(metrics[0]!.mode).toBe('auto');
    });

    it('activeTrackIndex is -1 when activeTrack is undefined', () => {
      const { controller, metrics } = makeController({ activeTrack: undefined });
      controller._tick();
      expect(metrics[0]!.activeTrackIndex).toBe(-1);
    });

    it('includes a copy of switch history in emitted metrics', () => {
      const { controller, metrics } = makeController(
        { bufferSeconds: 5, activeTrack: '360p' },
        { videoAutoSwitch: false },
      );
      controller.manualSwitch('720p');
      controller._tick();
      expect(metrics[0]!.switchHistory).toHaveLength(1);
      expect(metrics[0]!.switchHistory[0]!.reason).toBe('manual');
    });
  });

  describe('manual mode (videoAutoSwitch=false)', () => {
    it('does not switch when videoAutoSwitch is false', () => {
      const { controller, player } = makeController(
        { bufferSeconds: 5, activeTrack: '360p', bandwidthBps: 10_000_000 },
        { videoAutoSwitch: false },
      );

      controller._tick();

      // switchTrack should NOT have been called by the tick
      expect(player.switchTrack).not.toHaveBeenCalled();
    });

    it('still emits metrics in manual mode', () => {
      const { controller, metrics } = makeController(
        { bufferSeconds: 5 },
        { videoAutoSwitch: false },
      );
      controller._tick();
      expect(metrics).toHaveLength(1);
    });
  });

  describe('switching guard', () => {
    it('does not switch when switching guard is active', () => {
      // High bandwidth, buffer=5s → ThroughputRule would want to upgrade
      const { controller, player } = makeController(
        { bufferSeconds: 5, activeTrack: '360p', bandwidthBps: 10_000_000 },
        { videoAutoSwitch: true },
      );

      // First tick — should switch (guard not set yet)
      controller._tick();
      expect(player.switchTrack).toHaveBeenCalledTimes(1);
      player.switchTrack.mockClear();

      // Guard is now active; second tick should not switch
      controller._tick();
      expect(player.switchTrack).not.toHaveBeenCalled();
    });

    it('releaseSwitchingGuard allows the next switch', () => {
      const { controller, player } = makeController(
        { bufferSeconds: 5, activeTrack: '360p', bandwidthBps: 10_000_000 },
        { videoAutoSwitch: true },
      );

      controller._tick(); // first switch fires
      expect(player.switchTrack).toHaveBeenCalledTimes(1);
      player.switchTrack.mockClear();

      // Guard still active
      controller._tick();
      expect(player.switchTrack).not.toHaveBeenCalled();

      // Release guard
      controller.releaseSwitchingGuard();

      // Next tick should be able to switch again
      controller._tick();
      expect(player.switchTrack).toHaveBeenCalledTimes(1);
    });
  });

  describe('DYNAMIC strategy — ThroughputRule at low buffer', () => {
    it('uses ThroughputRule when buffer is below switchOnThreshold', () => {
      // buffer=5s < switchOnThreshold=18s → shouldUseBolaRule=false → ThroughputRule active
      const { controller, player } = makeController(
        { bufferSeconds: 5, activeTrack: '360p', bandwidthBps: 10_000_000 },
        { videoAutoSwitch: true },
      );

      controller._tick();

      // ThroughputRule at 10Mbps with safety factor 0.9 → effectiveBw=9Mbps
      // All tracks fit (max 4Mbps), so it would want index=2 (1080p), which differs from 360p (index=0)
      expect(player.switchTrack).toHaveBeenCalledWith('1080p');
    });
  });

  describe('DYNAMIC strategy — BolaRule at high buffer', () => {
    it('switches to BolaRule when buffer exceeds switchOnThreshold', () => {
      // buffer=20s >= switchOnThreshold=18s → shouldUseBolaRule=true → BolaRule active
      const { controller, player } = makeController(
        { bufferSeconds: 20, activeTrack: '360p', bandwidthBps: 10_000_000 },
        { videoAutoSwitch: true },
      );

      controller._tick();

      // With BolaRule active and high bandwidth/buffer, it should upgrade
      expect(player.switchTrack).toHaveBeenCalled();
      // Verify we didn't call with 360p (current track) — it should upgrade
      const calledWith = (player.switchTrack.mock.calls[0] as string[])[0];
      expect(calledWith).not.toBe('360p');
    });
  });

  describe('no switch when already on best track', () => {
    it('does not switch when already on the best track', () => {
      // Active track is 1080p (index=2) with high bandwidth — ThroughputRule also wants index=2
      const { controller, player } = makeController(
        { bufferSeconds: 5, activeTrack: '1080p', bandwidthBps: 10_000_000 },
        { videoAutoSwitch: true },
      );

      controller._tick();

      expect(player.switchTrack).not.toHaveBeenCalled();
    });
  });

  describe('manualSwitch', () => {
    it('calls switchTrack regardless of videoAutoSwitch setting', () => {
      const { controller, player } = makeController(
        { activeTrack: '360p' },
        { videoAutoSwitch: false },
      );

      controller.manualSwitch('1080p');

      expect(player.switchTrack).toHaveBeenCalledWith('1080p');
    });

    it('records a manual switch event in history', () => {
      const { controller } = makeController({ activeTrack: '360p' }, { videoAutoSwitch: false });

      controller.manualSwitch('720p');

      const history = controller.getHistory();
      expect(history).toHaveLength(1);
      expect(history[0]!.fromTrack).toBe('360p');
      expect(history[0]!.toTrack).toBe('720p');
      expect(history[0]!.reason).toBe('manual');
    });

    it('works even when videoAutoSwitch is true', () => {
      const { controller, player } = makeController(
        { activeTrack: '1080p' },
        { videoAutoSwitch: true },
      );

      controller.manualSwitch('360p');

      expect(player.switchTrack).toHaveBeenCalledWith('360p');
    });
  });

  describe('getHistory', () => {
    it('returns a copy of the switch history', () => {
      const { controller } = makeController({ activeTrack: '360p' }, { videoAutoSwitch: false });

      controller.manualSwitch('720p');
      const h1 = controller.getHistory();
      const h2 = controller.getHistory();

      expect(h1).toEqual(h2);
      expect(h1).not.toBe(h2); // different array instances
    });

    it('caps history at 60 entries', () => {
      const { controller } = makeController({ activeTrack: '360p' }, { videoAutoSwitch: false });

      for (let i = 0; i < 65; i++) {
        controller.manualSwitch('720p');
      }

      expect(controller.getHistory()).toHaveLength(60);
    });
  });

  describe('start/stop', () => {
    it('start begins a 250ms tick interval', () => {
      vi.useFakeTimers();
      const { controller, metrics } = makeController(
        { bufferSeconds: 5 },
        { videoAutoSwitch: false },
      );

      controller.start();
      vi.advanceTimersByTime(500);
      controller.stop();

      // Should have ticked roughly twice (at 250ms and 500ms)
      expect(metrics.length).toBeGreaterThanOrEqual(2);
      vi.useRealTimers();
    });

    it('stop clears the interval', () => {
      vi.useFakeTimers();
      const { controller, metrics } = makeController(
        { bufferSeconds: 5 },
        { videoAutoSwitch: false },
      );

      controller.start();
      vi.advanceTimersByTime(250);
      const countAfterFirst = metrics.length;
      controller.stop();

      vi.advanceTimersByTime(500);
      expect(metrics.length).toBe(countAfterFirst); // no more ticks after stop

      vi.useRealTimers();
    });
  });

  describe('updateSettings', () => {
    it('updates the settings reference used by subsequent ticks', () => {
      const { controller, player, metrics } = makeController(
        { bufferSeconds: 5, activeTrack: '360p', bandwidthBps: 10_000_000 },
        { videoAutoSwitch: true },
      );

      // First tick should switch (auto mode)
      controller._tick();
      expect(player.switchTrack).toHaveBeenCalledTimes(1);
      player.switchTrack.mockClear();
      controller.releaseSwitchingGuard();

      // Switch to manual mode
      controller.updateSettings(makeSettings({ videoAutoSwitch: false }));

      // Now tick should not switch
      controller._tick();
      expect(player.switchTrack).not.toHaveBeenCalled();

      // Mode should be reflected in emitted metrics
      const lastMetrics = metrics[metrics.length - 1]!;
      expect(lastMetrics.mode).toBe('manual');
    });
  });
});
