import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { GoodputTracker } from '../goodput';

describe('GoodputTracker', () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  // ---------------------------------------------------------------------------
  // Fallback path (no WebTransport.getStats — same behaviour as before)
  // ---------------------------------------------------------------------------

  describe('fallback (no transport)', () => {
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

  // ---------------------------------------------------------------------------
  // Transport stats path (WebTransport.getStats)
  // ---------------------------------------------------------------------------

  describe('transport stats path', () => {
    function makeMockTransport(initialBytes = 0) {
      let bytesReceived = initialBytes;
      return {
        transport: {
          getStats: vi.fn().mockImplementation(() => Promise.resolve({ bytesReceived })),
        } as unknown as WebTransport,
        setBytesReceived(n: number) {
          bytesReceived = n;
        },
      };
    }

    it('poll() is a no-op when no transport is set', async () => {
      const t = new GoodputTracker();
      await t.poll(); // should not throw
      expect(t.getBandwidthBps()).toBe(0);
    });

    it('seeds baseline on first poll without updating EMA', async () => {
      const { transport } = makeMockTransport(1000);
      const t = new GoodputTracker();
      t.setTransport(transport);

      await t.poll();
      expect(t.getBandwidthBps()).toBe(0); // no EMA update on first poll
    });

    it('computes throughput from bytesReceived delta', async () => {
      const { transport, setBytesReceived } = makeMockTransport(0);
      const t = new GoodputTracker(0.5, 0.1);
      t.setTransport(transport);

      // First poll — seeds baseline
      await t.poll();

      // Simulate 100ms passing and 50000 bytes received
      vi.advanceTimersByTime(200);
      setBytesReceived(50000);
      await t.poll();

      // 50000 bytes in 200ms = 50000 * 8 * 1000 / 200 = 2,000,000 bps
      expect(t.getBandwidthBps()).toBeGreaterThan(1_500_000);
      expect(t.getBandwidthBps()).toBeLessThan(2_500_000);
    });

    it('skips EMA update when delta is too short (< 50ms)', async () => {
      const { transport, setBytesReceived } = makeMockTransport(0);
      const t = new GoodputTracker();
      t.setTransport(transport);

      await t.poll(); // seed
      vi.advanceTimersByTime(30); // only 30ms
      setBytesReceived(100000);
      await t.poll();

      expect(t.getBandwidthBps()).toBe(0); // not enough elapsed time
    });

    it('recordObject only tracks metadata when transport stats are active', () => {
      const { transport } = makeMockTransport(0);
      const t = new GoodputTracker();
      t.setTransport(transport);

      t.recordObject(5000, 0);
      t.recordObject(8000, 0);

      // Metadata is tracked
      expect(t.getLastObjectBytes()).toBe(8000);
      expect(t.getSampleCount()).toBe(2);
      // But no EMA from recordObject — only poll() feeds EMA
      expect(t.getBandwidthBps()).toBe(0);
    });

    it('reset clears transport stats state', async () => {
      const { transport, setBytesReceived } = makeMockTransport(0);
      const t = new GoodputTracker(0.5, 0.1);
      t.setTransport(transport);

      await t.poll();
      vi.advanceTimersByTime(200);
      setBytesReceived(50000);
      await t.poll();
      expect(t.getBandwidthBps()).toBeGreaterThan(0);

      t.reset();
      expect(t.getBandwidthBps()).toBe(0);
    });

    it('falls back to manual counting when getStats is missing', () => {
      const transport = {} as unknown as WebTransport; // no getStats
      const t = new GoodputTracker(0.5, 0.1);
      t.setTransport(transport);

      // Should behave like fallback path
      t.recordObject(50000, 0);
      vi.advanceTimersByTime(200);
      t.recordObject(50000, 0);

      expect(t.getBandwidthBps()).toBeGreaterThan(3_000_000);
    });
  });
});
