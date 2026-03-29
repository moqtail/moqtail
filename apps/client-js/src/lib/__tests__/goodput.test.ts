import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { GoodputTracker } from '../goodput';

describe('GoodputTracker', () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it('returns 0 before any samples', () => {
    const t = new GoodputTracker();
    expect(t.getBandwidthBps()).toBe(0);
    expect(t.getFastEmaBps()).toBe(0);
    expect(t.getSlowEmaBps()).toBe(0);
  });

  it('returns 0 during window warmup (< 100ms elapsed)', () => {
    const t = new GoodputTracker(0.5, 0.1);
    t.recordObject(10000, 0); // first sample seeds the window
    vi.advanceTimersByTime(50);
    t.recordObject(10000, 0); // 50ms in — still warming up
    expect(t.getBandwidthBps()).toBe(0);
  });

  it('produces a throughput estimate after window warmup', () => {
    const t = new GoodputTracker(0.5, 0.1);
    t.recordObject(50000, 0); // first sample
    vi.advanceTimersByTime(200);
    t.recordObject(50000, 0); // 200ms later
    // 100000 bytes in 200ms = 100000 * 8 * 1000 / 200 = 4,000,000 bps
    expect(t.getBandwidthBps()).toBeGreaterThan(3_000_000);
    expect(t.getBandwidthBps()).toBeLessThan(5_000_000);
  });

  it('returns min(fast, slow) as conservative estimate', () => {
    const t = new GoodputTracker(0.5, 0.1);
    // Fill first window
    t.recordObject(50000, 0);
    vi.advanceTimersByTime(1000);
    t.recordObject(50000, 0);
    // Second window with more data (higher throughput)
    vi.advanceTimersByTime(3000);
    t.recordObject(200000, 0);
    vi.advanceTimersByTime(1000);
    t.recordObject(200000, 0);
    // min(fast, slow) — slow reacts less
    expect(t.getBandwidthBps()).toBe(Math.min(t.getFastEmaBps(), t.getSlowEmaBps()));
  });

  it('resets both EMAs to 0 on reset()', () => {
    const t = new GoodputTracker();
    t.recordObject(10000, 0);
    vi.advanceTimersByTime(1000);
    t.recordObject(10000, 0);
    t.reset();
    expect(t.getBandwidthBps()).toBe(0);
    expect(t.getFastEmaBps()).toBe(0);
    expect(t.getSlowEmaBps()).toBe(0);
  });

  it('getSampleCount tracks number of recorded objects', () => {
    const t = new GoodputTracker();
    expect(t.getSampleCount()).toBe(0);
    t.recordObject(1000, 0);
    expect(t.getSampleCount()).toBe(1);
    t.recordObject(2000, 0);
    expect(t.getSampleCount()).toBe(2);
  });

  it('getLastObjectBytes returns last recorded size', () => {
    const t = new GoodputTracker();
    t.recordObject(5000, 0);
    expect(t.getLastObjectBytes()).toBe(5000);
    t.recordObject(12000, 0);
    expect(t.getLastObjectBytes()).toBe(12000);
  });
});
