import { describe, it, expect } from 'vitest';
import { GoodputTracker } from './goodput';

describe('GoodputTracker', () => {
  it('returns 0 before any samples', () => {
    const t = new GoodputTracker();
    expect(t.getBandwidthBps()).toBe(0);
    expect(t.getFastEmaBps()).toBe(0);
    expect(t.getSlowEmaBps()).toBe(0);
  });

  it('initialises both EMAs to first sample value on cold start', () => {
    const t = new GoodputTracker(0.5, 0.1);
    // 1000 bytes in 8ms = 1,000,000 bps
    t.recordObject(1000, 8);
    expect(t.getFastEmaBps()).toBeCloseTo(1_000_000, 0);
    expect(t.getSlowEmaBps()).toBeCloseTo(1_000_000, 0);
    expect(t.getBandwidthBps()).toBeCloseTo(1_000_000, 0);
  });

  it('returns min(fast, slow) as conservative estimate', () => {
    const t = new GoodputTracker(0.5, 0.1);
    t.recordObject(1000, 8); // seed at 1 Mbps
    t.recordObject(2000, 8); // 2 Mbps: fast (alpha=0.5) reacts more than slow (alpha=0.1)
    expect(t.getFastEmaBps()).toBeGreaterThan(t.getSlowEmaBps());
    expect(t.getBandwidthBps()).toBe(t.getSlowEmaBps());
  });

  it('fast EMA drops quickly on bandwidth reduction', () => {
    const t = new GoodputTracker(0.5, 0.1);
    // Seed with 4 Mbps
    for (let i = 0; i < 10; i++) t.recordObject(4000, 8);
    const highFast = t.getFastEmaBps();
    // Now 0.5 Mbps
    t.recordObject(500, 8);
    expect(t.getFastEmaBps()).toBeLessThan(highFast * 0.8); // fast dropped > 20%
    expect(t.getBandwidthBps()).toBe(t.getFastEmaBps()); // min picks fast now
  });

  it('resets both EMAs to 0 on reset()', () => {
    const t = new GoodputTracker();
    t.recordObject(1000, 8);
    t.reset();
    expect(t.getBandwidthBps()).toBe(0);
    expect(t.getFastEmaBps()).toBe(0);
    expect(t.getSlowEmaBps()).toBe(0);
  });

  it('ignores zero-duration samples', () => {
    const t = new GoodputTracker();
    t.recordObject(1000, 0);
    expect(t.getBandwidthBps()).toBe(0);
  });

  it('ignores negative-duration samples', () => {
    const t = new GoodputTracker();
    t.recordObject(1000, -5);
    expect(t.getBandwidthBps()).toBe(0);
  });

  it('setAlphas() changes EMA reactivity', () => {
    const t = new GoodputTracker(0.5, 0.1);
    t.setAlphas(0.9, 0.5);
    t.recordObject(1000, 8); // seed at 1 Mbps
    t.recordObject(2000, 8); // 2 Mbps
    // fast = 0.9*2M + 0.1*1M = 1.9M; slow = 0.5*2M + 0.5*1M = 1.5M
    expect(t.getFastEmaBps()).toBeCloseTo(1_900_000, -3);
    expect(t.getSlowEmaBps()).toBeCloseTo(1_500_000, -3);
  });
});
