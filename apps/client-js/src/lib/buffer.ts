/**
 * Copyright 2026 The MOQtail Authors
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

import { logger } from '@/lib/logger';

// MSE Buffer Configuration
export const MSE_IMMEDIATE_SEEK_THRESHOLD = 0.1; // seconds
export const DEFAULT_LIVE_EDGE_DELAY = 1.25; // seconds
export const DEFAULT_LIVE_EDGE_TOLERANCE = 0.1; // seconds
export const DEFAULT_BUFFER_CHECK_INTERVAL = 250; // milliseconds
export const DEFAULT_STALL_THRESHOLD = 0.5; // seconds
export const DEFAULT_CATCHUP_PLAYBACK_RATE = 1.05; // 5% faster

interface MSEBufferConfig {
  /** Delay from live edge in seconds (default: 1.25) */
  liveEdgeDelay: number;
  /** Tolerance for live edge in seconds (default: 0.1) */
  liveEdgeTolerance: number;
  /** Interval for checking buffered regions in milliseconds (default: 250) */
  bufferCheckInterval: number;
  /** Threshold for detecting stalls in seconds (default: 0.5) */
  stallThreshold: number;
  /** Playback rate for catching up to live edge (default: 1.05 = 5% faster) */
  catchupPlaybackRate: number;
}

class MSEBuffer {
  private config: MSEBufferConfig;
  private bufferCheckInterval: number | null = null;
  private isDisposed: boolean = false;
  private isCatchingUp: boolean = false;
  private isCatchingDown: boolean = false;
  private originalPlaybackRate: number = 1.0;

  constructor(
    public video: HTMLVideoElement,
    config: Partial<MSEBufferConfig> = {},
  ) {
    this.config = {
      liveEdgeDelay: DEFAULT_LIVE_EDGE_DELAY,
      liveEdgeTolerance: DEFAULT_LIVE_EDGE_TOLERANCE,
      bufferCheckInterval: DEFAULT_BUFFER_CHECK_INTERVAL,
      stallThreshold: DEFAULT_STALL_THRESHOLD,
      catchupPlaybackRate: DEFAULT_CATCHUP_PLAYBACK_RATE,
      ...config,
    };

    this.init();
  }

  private init() {
    // Attach event listeners
    this.video.addEventListener('pause', this.handlePause);
    this.video.addEventListener('play', this.handlePlay);
    this.video.addEventListener('waiting', this.handleWaiting);
    this.video.addEventListener('stalled', this.handleStalled);

    // Listen tab visibility changes
    document.addEventListener('visibilitychange', this.handleTabChange);

    // Start periodic buffer checking
    this.startBufferMonitoring();
  }

  private handleTabChange = () => {
    if (document.hidden) {
      // Tab is hidden, pause monitoring
      if (this.bufferCheckInterval) {
        clearInterval(this.bufferCheckInterval);
        this.bufferCheckInterval = null;
      }
    } else {
      // Calculate the live edge
      const buffered = this.video.buffered;
      if (buffered.length > 0) {
        logger.info('buffer', '[mseBuffer] Tab became visible, seeking to live edge');
        const liveEdge = buffered.end(buffered.length - 1);
        this.seek(
          Math.max(liveEdge - this.config.liveEdgeDelay, buffered.start(buffered.length - 1)),
        );
        this.video.playbackRate = this.originalPlaybackRate;
        this.isCatchingUp = false;
        this.video.play();
      }

      // Tab is visible, resume monitoring
      this.startBufferMonitoring();
    }
  };

  private handlePause = () => {
    logger.info('buffer', '[mseBuffer] Video is paused');
    this.resetPlaybackRate();
  };

  private handlePlay = () => {
    logger.info('buffer', '[mseBuffer] Video is playing');
    this.originalPlaybackRate = this.video.playbackRate;
  };

  private handleWaiting = () => {
    logger.info('buffer', '[mseBuffer] Video is waiting for data');
    this.checkBufferedRegions();
  };

  private handleStalled = () => {
    logger.info('buffer', '[mseBuffer] Video stalled event fired');
    this.checkBufferedRegions();
  };

  private startBufferMonitoring() {
    if (this.bufferCheckInterval) {
      clearInterval(this.bufferCheckInterval);
    }

    this.bufferCheckInterval = window.setInterval(() => {
      if (this.isDisposed) return;
      this.periodicBufferCheck();
    }, this.config.bufferCheckInterval);
  }

  private periodicBufferCheck() {
    // Only check for live streams
    if (!this.video.duration || isFinite(this.video.duration)) {
      return; // Not a live stream
    }

    this.checkBufferedRegions(false);
  }

  private checkBufferedRegions(logDetails: boolean = true) {
    const buffered = this.video.buffered;
    const currentTime = this.video.currentTime;

    if (buffered.length === 0) {
      logger.info('buffer', '[mseBuffer] No buffered data available');
      return;
    }

    if (logDetails) {
      logger.info('buffer', '[mseBuffer] Checking buffered regions:');
      for (let i = 0; i < buffered.length; i++) {
        const start = buffered.start(i);
        const end = buffered.end(i);
        logger.info(
          'buffer',
          `[mseBuffer]   Range ${i}: ${start.toFixed(2)}s - ${end.toFixed(2)}s`,
        );
      }
    }

    // Check if we're at the end of a buffer range and need to jump to the next one
    const shouldSeek = this.shouldSeekToNextRange(currentTime, buffered);

    if (shouldSeek.seek) {
      logger.info(
        'buffer',
        `[mseBuffer] At end of range, seeking to next buffered range: ${shouldSeek.targetTime!.toFixed(2)}s`,
      );

      // Perform the seek, only if targetTime is ahead of currentTime
      if (shouldSeek.targetTime <= currentTime) return;
      this.seek(shouldSeek.targetTime);

      // Resume the video if it was paused
      if (this.video.paused) {
        this.video
          .play()
          .then(() => {
            logger.info('buffer', '[mseBuffer] Video was paused and now playing...');
          })
          .catch(e => {
            logger.warn('buffer', '[mseBuffer] Video was paused and could not play it...', e);
          });
      }
    } else {
      // For live streams, check if we need to catch up to live edge
      if (!isFinite(this.video.duration)) this.maintainLiveEdgeDelay();
    }
  }

  private shouldSeekToNextRange(
    currentTime: number,
    buffered: TimeRanges,
  ): { seek: true; targetTime: number } | { seek: false } {
    // If no buffered ranges, cannot seek
    if (buffered.length === 0) return { seek: false };

    // Find which buffered range we're currently in (if any)
    let currentRangeIndex = -1;
    for (let i = 0; i < buffered.length; i++) {
      const start = buffered.start(i);
      const end = buffered.end(i);

      if (currentTime >= start && currentTime <= end) {
        currentRangeIndex = i;
        break;
      }
    }

    // If we're not in any buffered range, find the nearest one to seek to
    if (currentRangeIndex === -1) {
      logger.warn('buffer', '[mseBuffer] Current time is not in any buffered range');
      return this.findNearestBufferedRange(currentTime, buffered);
    }

    // Check if we're close to the end of the current range
    const currentRangeEnd = buffered.end(currentRangeIndex);
    const distanceToEnd = currentRangeEnd - currentTime;

    // Only consider seeking if we're very close to the end (within threshold)
    if (distanceToEnd > this.config.stallThreshold) return { seek: false };

    // Check if there's a next buffered range
    if (currentRangeIndex + 1 < buffered.length) {
      for (let nextRange = currentRangeIndex + 1; nextRange < buffered.length; nextRange++) {
        const nextRangeStart = buffered.start(currentRangeIndex + 1);
        const gap = nextRangeStart - currentRangeEnd;

        // If gap is too small, seek to next range immediately
        if (gap < MSE_IMMEDIATE_SEEK_THRESHOLD) {
          logger.info(
            'buffer',
            `[mseBuffer] Small gap of ${gap.toFixed(3)}s to next range, seeking immediately`,
          );
          return { seek: true, targetTime: nextRangeStart };
        }

        // Next range must have enough buffer to jump to
        const nextRangeEnd = buffered.end(currentRangeIndex + 1);
        const nextRangeDuration = nextRangeEnd - nextRangeStart;

        if (nextRangeDuration < this.config.stallThreshold) {
          logger.warn(
            'buffer',
            `[mseBuffer] Next range too short (${nextRangeDuration.toFixed(3)}s), not seeking`,
          );
          continue;
        }

        if (nextRangeDuration < this.config.liveEdgeDelay) {
          logger.warn(
            'buffer',
            `[mseBuffer] Next range shorter than live edge delay (${nextRangeDuration.toFixed(
              3,
            )}s < ${this.config.liveEdgeDelay}s), not seeking`,
          );
          continue;
        }

        if (gap > 0) {
          logger.warn(
            'buffer',
            `[mseBuffer] At buffer end with gap of ${gap.toFixed(3)}s, must jump to next range`,
          );
          return { seek: true, targetTime: nextRangeStart };
        }
      }
    }

    return { seek: false };
  }

  private findNearestBufferedRange(
    currentTime: number,
    buffered: TimeRanges,
  ): { seek: true; targetTime: number } | { seek: false } {
    if (buffered.length === 0) {
      return { seek: false };
    }

    // For live streams, prefer the most recent buffered range
    if (!isFinite(this.video.duration)) {
      const lastRangeIndex = buffered.length - 1;
      const targetTime = Math.max(
        buffered.start(lastRangeIndex),
        buffered.end(lastRangeIndex) - this.config.liveEdgeDelay,
      );
      return { seek: true, targetTime };
    }

    // For VOD, find the closest buffered range
    let bestTarget = buffered.start(0);
    let minDistance = Math.abs(currentTime - bestTarget);

    for (let i = 0; i < buffered.length; i++) {
      const start = buffered.start(i);
      const end = buffered.end(i);

      // Check distance to start of range
      const distanceToStart = Math.abs(currentTime - start);
      if (distanceToStart < minDistance) {
        minDistance = distanceToStart;
        bestTarget = start;
      }

      // Check distance to end of range
      const distanceToEnd = Math.abs(currentTime - end);
      if (distanceToEnd < minDistance) {
        minDistance = distanceToEnd;
        bestTarget = end;
      }
    }

    return { seek: true, targetTime: bestTarget };
  }

  private maintainLiveEdgeDelay() {
    const buffered = this.video.buffered;
    if (buffered.length === 0) return;

    // Get the end of the last buffered range (live edge)
    const bufferEdge = buffered.end(buffered.length - 1);
    const currentLatency = bufferEdge - this.video.currentTime;
    const targetDistance = this.config.liveEdgeDelay;

    // Use playback rate adjustment to catch up instead of seeking
    if (currentLatency > targetDistance + this.config.liveEdgeTolerance) {
      // We're behind but close, use catchup speed
      if (!this.isCatchingUp) {
        logger.info(
          'buffer',
          `[mseBuffer] Too far from live edge (${currentLatency.toFixed(2)}s), catching up at ${this.config.catchupPlaybackRate}x speed`,
        );
        this.isCatchingUp = true;
        this.isCatchingDown = false;
        this.video.playbackRate = this.config.catchupPlaybackRate;
      }
    } else if (currentLatency < targetDistance - this.config.liveEdgeTolerance) {
      // We're too close to the live edge, slow down slightly
      if (!this.isCatchingDown) {
        const slowdownRate = 1 - (this.config.catchupPlaybackRate - 1);
        logger.info(
          'buffer',
          `[mseBuffer] Close to live edge (${currentLatency.toFixed(2)}s), slowing down to ${slowdownRate.toFixed(2)}x speed`,
        );
        this.isCatchingUp = false;
        this.isCatchingDown = true;
        this.video.playbackRate = slowdownRate;
      }
    } else if (
      (this.isCatchingUp || this.isCatchingDown) &&
      Math.abs(currentLatency - targetDistance) < this.config.liveEdgeTolerance
    ) {
      // We've reached the target distance, return to normal speed
      logger.info(
        'buffer',
        `[mseBuffer] Reached target distance from live edge (${currentLatency.toFixed(2)}s), returning to normal speed`,
      );
      this.resetPlaybackRate();
    }
  }

  private resetPlaybackRate() {
    if (
      this.isCatchingUp ||
      this.isCatchingDown ||
      this.video.playbackRate !== this.originalPlaybackRate
    ) {
      this.video.playbackRate = this.originalPlaybackRate;
      this.isCatchingUp = false;
      this.isCatchingDown = false;
      logger.info('buffer', `[mseBuffer] Playback rate reset to ${this.originalPlaybackRate}x`);
    }
  }

  private seek(time: number) {
    this.video.currentTime = time;
  }

  dispose() {
    if (this.isDisposed) return;
    this.isDisposed = true;

    // Reset playback rate before disposing
    this.resetPlaybackRate();

    // Remove event listeners
    this.video.removeEventListener('pause', this.handlePause);
    this.video.removeEventListener('play', this.handlePlay);
    this.video.removeEventListener('waiting', this.handleWaiting);
    this.video.removeEventListener('stalled', this.handleStalled);

    // Remove tab visibility listener
    document.removeEventListener('visibilitychange', this.handleTabChange);

    // Clear interval
    if (this.bufferCheckInterval) {
      clearInterval(this.bufferCheckInterval);
      this.bufferCheckInterval = null;
    }
  }
}

export default MSEBuffer;
export type { MSEBufferConfig };
