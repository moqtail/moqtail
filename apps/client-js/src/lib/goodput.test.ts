import { describe, it, expect, beforeEach } from 'vitest'
import { GoodputTracker } from './goodput'

describe('GoodputTracker', () => {
  let tracker: GoodputTracker

  beforeEach(() => {
    tracker = new GoodputTracker()
  })

  it('returns 0 before any samples', () => {
    expect(tracker.getBandwidthBps()).toBe(0)
    expect(tracker.getFastEmaBps()).toBe(0)
    expect(tracker.getSlowEmaBps()).toBe(0)
  })

  it('initialises both EMAs to the first sample', () => {
    // 125_000 bytes in 500ms = 2_000_000 bps
    tracker.recordObject(125_000, 500)
    expect(tracker.getFastEmaBps()).toBe(2_000_000)
    expect(tracker.getSlowEmaBps()).toBe(2_000_000)
    expect(tracker.getBandwidthBps()).toBe(2_000_000)
  })

  it('fast EMA reacts more than slow EMA on second sample', () => {
    tracker.recordObject(125_000, 500) // 2 Mbps
    tracker.recordObject(12_500, 500)  // 0.2 Mbps (sudden drop)
    // fast alpha=0.5: 0.5 * 200_000 + 0.5 * 2_000_000 = 1_100_000
    // slow alpha=0.1: 0.1 * 200_000 + 0.9 * 2_000_000 = 1_820_000
    expect(tracker.getFastEmaBps()).toBeCloseTo(1_100_000, 0)
    expect(tracker.getSlowEmaBps()).toBeCloseTo(1_820_000, 0)
  })

  it('getBandwidthBps returns min(fast, slow)', () => {
    tracker.recordObject(125_000, 500) // 2 Mbps
    tracker.recordObject(12_500, 500)  // 0.2 Mbps drop
    // fast < slow after a drop
    expect(tracker.getBandwidthBps()).toBe(tracker.getFastEmaBps())
  })

  it('reset clears both EMAs to 0', () => {
    tracker.recordObject(125_000, 500)
    tracker.reset()
    expect(tracker.getBandwidthBps()).toBe(0)
    expect(tracker.getFastEmaBps()).toBe(0)
    expect(tracker.getSlowEmaBps()).toBe(0)
  })

  it('setAlphas changes smoothing behaviour from next sample', () => {
    tracker.recordObject(125_000, 500) // 2 Mbps seed
    tracker.setAlphas(1.0, 1.0)        // both fully reactive
    tracker.recordObject(12_500, 500)  // 0.2 Mbps
    expect(tracker.getFastEmaBps()).toBeCloseTo(200_000, 0)
    expect(tracker.getSlowEmaBps()).toBeCloseTo(200_000, 0)
  })

  it('handles very short durations without division by zero', () => {
    expect(() => tracker.recordObject(1000, 0)).not.toThrow()
    expect(tracker.getBandwidthBps()).toBeGreaterThan(0)
  })
})
