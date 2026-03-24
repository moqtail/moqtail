import { describe, it, expect, vi, beforeEach } from 'vitest'
import { AbrController } from './abr'
import type { AbrMetrics } from './abr'

// Minimal Track shape needed by AbrController
const tracks = [
  { name: '360p', bitrate: 500_000,   role: 'video' as const, codec: 'hvc1', width: 640,  height: 360  },
  { name: '480p', bitrate: 1_000_000, role: 'video' as const, codec: 'hvc1', width: 854,  height: 480  },
  { name: '720p', bitrate: 2_000_000, role: 'video' as const, codec: 'hvc1', width: 1280, height: 720  },
  { name: '1080p', bitrate: 4_000_000, role: 'video' as const, codec: 'hvc1', width: 1920, height: 1080 },
]

function makeMockPlayer(overrides: Partial<{
  bandwidthBps: number
  bufferSeconds: number
  activeTrack: string | null
}> = {}) {
  return {
    getMetrics: vi.fn(() => ({
      bandwidthBps: overrides.bandwidthBps ?? 3_000_000,
      fastEmaBps: overrides.bandwidthBps ?? 3_000_000,
      slowEmaBps: overrides.bandwidthBps ?? 3_000_000,
      bufferSeconds: overrides.bufferSeconds ?? 4.0,
      activeTrack: overrides.activeTrack ?? '720p',
    })),
    switchTrack: vi.fn(),
    setEmaAlphas: vi.fn(),
  }
}

describe('BOLA score computation', () => {
  it('score is positive for lowest tier at any non-zero buffer', () => {
    const player = makeMockPlayer({ bufferSeconds: 0.6 })
    const onUpdate = vi.fn()
    const ctrl = new AbrController(player as any, tracks, onUpdate)
    const metrics = player.getMetrics()
    const scores = ctrl.computeBolaScores(metrics.bufferSeconds)
    expect(scores['360p']).toBeGreaterThan(0)
  })

  it('score is negative for highest tier when buffer is critically low', () => {
    const player = makeMockPlayer({ bufferSeconds: 0.1 })
    const onUpdate = vi.fn()
    const ctrl = new AbrController(player as any, tracks, onUpdate)
    const scores = ctrl.computeBolaScores(0.1)
    expect(scores['1080p']).toBeLessThan(0)
  })

  it('higher buffer level produces lower BOLA scores', () => {
    const player = makeMockPlayer()
    const onUpdate = vi.fn()
    const ctrl = new AbrController(player as any, tracks, onUpdate)
    const atLowBuffer = ctrl.computeBolaScores(1.0)
    const atHighBuffer = ctrl.computeBolaScores(4.0)
    for (const name of tracks.map(t => t.name)) {
      // BOLA formula subtracts bufferLevel, so higher buffer → lower score
      expect(atLowBuffer[name]).toBeGreaterThan(atHighBuffer[name])
    }
  })
})

describe('AbrController decision logic', () => {
  it('stays on current track during DEWMA cold start (bandwidthBps === 0)', () => {
    const player = makeMockPlayer({ bandwidthBps: 0, bufferSeconds: 4.0, activeTrack: '720p' })
    const onUpdate = vi.fn()
    const ctrl = new AbrController(player as any, tracks, onUpdate)
    ctrl.setMode('auto')
    ctrl.start()
    ctrl._tick()
    expect(player.switchTrack).not.toHaveBeenCalled()
    ctrl.stop()
  })

  it('triggers emergency downgrade when buffer < emergencyBufferFloor', () => {
    const player = makeMockPlayer({ bandwidthBps: 3_000_000, bufferSeconds: 0.3, activeTrack: '720p' })
    const onUpdate = vi.fn()
    const ctrl = new AbrController(player as any, tracks, onUpdate)
    ctrl.setMode('auto')
    ctrl.start()
    ctrl._tick()
    expect(player.switchTrack).toHaveBeenCalledWith('360p')
    ctrl.stop()
  })

  it('does not switch when already on the best BOLA tier', () => {
    const player = makeMockPlayer({ bandwidthBps: 10_000_000, bufferSeconds: 4.0, activeTrack: '1080p' })
    const onUpdate = vi.fn()
    const ctrl = new AbrController(player as any, tracks, onUpdate)
    ctrl.setMode('auto')
    ctrl.start()
    ctrl._tick()
    expect(player.switchTrack).not.toHaveBeenCalled()
    ctrl.stop()
  })

  it('does not switch when #switching guard is active', () => {
    const player = makeMockPlayer({ bandwidthBps: 0.1, bufferSeconds: 0.2, activeTrack: '1080p' })
    const onUpdate = vi.fn()
    const ctrl = new AbrController(player as any, tracks, onUpdate)
    ctrl.setMode('auto')
    ctrl.start()
    ctrl._tick()   // first tick: sets #switching = true, calls switchTrack
    ctrl._tick()   // second tick: guard is active, no second call
    expect(player.switchTrack).toHaveBeenCalledTimes(1)
    ctrl.stop()
  })

  it('manual mode: manualSwitch calls player.switchTrack regardless of BOLA', () => {
    const player = makeMockPlayer({ bandwidthBps: 100, bufferSeconds: 0.1, activeTrack: '1080p' })
    const onUpdate = vi.fn()
    const ctrl = new AbrController(player as any, tracks, onUpdate)
    ctrl.setMode('manual')
    ctrl.manualSwitch('480p')
    expect(player.switchTrack).toHaveBeenCalledWith('480p')
  })

  it('auto mode: does not fire on tick when mode is manual', () => {
    const player = makeMockPlayer({ bandwidthBps: 100, bufferSeconds: 0.1, activeTrack: '1080p' })
    const onUpdate = vi.fn()
    const ctrl = new AbrController(player as any, tracks, onUpdate)
    ctrl.setMode('manual')
    ctrl._tick()
    expect(player.switchTrack).not.toHaveBeenCalled()
  })

  it('setThresholds recomputes bolaV when bufferMax changes', () => {
    const player = makeMockPlayer()
    const onUpdate = vi.fn()
    const ctrl = new AbrController(player as any, tracks, onUpdate)
    const vBefore = ctrl.getThresholds().bolaV
    ctrl.setThresholds({ bufferMax: 8.0 })
    expect(ctrl.getThresholds().bolaV).toBeGreaterThan(vBefore)
  })

  it('records switch event in history', () => {
    const player = makeMockPlayer({ bandwidthBps: 3_000_000, bufferSeconds: 0.2, activeTrack: '720p' })
    const onUpdate = vi.fn()
    const ctrl = new AbrController(player as any, tracks, onUpdate)
    ctrl.setMode('auto')
    ctrl.start()
    ctrl._tick()
    const history = ctrl.getHistory()
    expect(history.length).toBe(1)
    expect(history[0].reason).toBe('auto-emergency')
    ctrl.stop()
  })
})
