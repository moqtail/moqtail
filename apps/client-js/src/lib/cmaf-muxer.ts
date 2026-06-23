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

import { createFile } from 'mp4box';
import type { ISOFile } from 'mp4box';

export interface SegmentEvent {
  data: Uint8Array;
  isKey: boolean;
  timestamp: number;
}

/**
 * CMAF muxer for a video track.
 * Uses mp4box.js to produce 1-sample-per-fragment fragmented MP4.
 *
 * Flow:
 *   1. Encode with WebCodecs VideoEncoder
 *   2. On first keyframe, call initVideo() to set up mp4box with the decoder config
 *   3. addVideoChunk() produces moof+mdat segments via onSegment
 *   4. onInit fires once with the moov (init segment) bytes
 */
export class CMAFMuxer {
  private file: ISOFile | null = null;
  private trackId: number | null = null;
  private initialized = false;
  private timescale = 90000;

  private pendingWidth = 0;
  private pendingHeight = 0;
  private _lastIsKey = false;
  private _lastTimestamp = 0;

  onInit: (moov: Uint8Array) => void = () => {};
  onSegment: (evt: SegmentEvent) => void = () => {};

  constructor() {}

  /**
   * Initialize the mp4box file for video encoding.
   * Must be called on the first keyframe when decoderConfig.description is available.
   */
  initVideo(width: number, height: number, avcDecoderConfigRecord: ArrayBuffer): void {
    if (this.initialized) return;
    this.initialized = true;
    this.pendingWidth = width;
    this.pendingHeight = height;

    const file = createFile();
    this.file = file;

    file.onSegment = (_id, _user, buffer, _nextSample, _isLast) => {
      this.onSegment({
        data: new Uint8Array(buffer),
        isKey: this._lastIsKey,
        timestamp: this._lastTimestamp,
      });
    };

    this.trackId = file.addTrack({
      type: 'avc1',
      width,
      height,
      timescale: this.timescale,
      avcDecoderConfigRecord,
    });

    file.setSegmentOptions(this.trackId, undefined, {
      nbSamplesPerFragment: 1,
      rapAlignement: false,
    });

    const initSeg = file.initializeSegmentation();
    file.start();
    this.onInit(new Uint8Array(initSeg.buffer));
  }

  addVideoChunk(chunk: EncodedVideoChunk, meta?: EncodedVideoChunkMetadata): void {
    if (!this.initialized) {
      // Defer init until we have the decoder config from the first keyframe
      if (chunk.type !== 'key' || !meta?.decoderConfig?.description) return;
      this.initVideo(
        this.pendingWidth || 1280,
        this.pendingHeight || 720,
        meta.decoderConfig.description as ArrayBuffer,
      );
    }

    if (!this.file || this.trackId === null) return;

    const data = new Uint8Array(chunk.byteLength);
    chunk.copyTo(data);

    const dts = Math.round((chunk.timestamp * this.timescale) / 1_000_000);
    const duration = chunk.duration
      ? Math.round((chunk.duration * this.timescale) / 1_000_000)
      : Math.round((1000 / 30) * (this.timescale / 1000));

    this._lastIsKey = chunk.type === 'key';
    this._lastTimestamp = chunk.timestamp;

    this.file.addSample(this.trackId, data, {
      duration,
      cts: dts,
      dts,
      is_sync: chunk.type === 'key',
    });
  }
}
