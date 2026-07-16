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

import {
  MOQtailClient,
  LiveTrackSource,
  FullTrackName,
  Location,
  MoqtObject,
  ExtensionHeaders,
  ObjectForwardingPreference,
  Tuple,
  type CMSF,
} from 'moqtail';
import { CMAFMuxer } from './cmaf-muxer';
import { logger } from './logger';
import type { SourceState, PublishStatus } from '../types';

// H.264 High Profile Level 4.0 — resolution and bitrate set at runtime from track settings
const CAMERA_ENCODER_CONFIG_BASE: Omit<VideoEncoderConfig, 'width' | 'height' | 'bitrate'> = {
  codec: 'avc1.640028',
  framerate: 30,
  latencyMode: 'realtime',
  avc: { format: 'avc' },
};

// Screen share: higher bitrate to preserve text/UI detail; resolution detected at runtime
const SCREEN_ENCODER_CONFIG: VideoEncoderConfig = {
  codec: 'avc1.640028',
  width: 1920,
  height: 1080,
  bitrate: 4_000_000,
  framerate: 30,
  latencyMode: 'realtime',
  avc: { format: 'avc' },
};

// Test source at lower fps/bitrate — synthetic content compresses well
const TEST_ENCODER_CONFIG: VideoEncoderConfig = {
  codec: 'avc1.640028',
  width: 1280,
  height: 720,
  bitrate: 1_000_000,
  framerate: 10,
  latencyMode: 'realtime',
  avc: { format: 'avc' },
};

const CATALOG_INTERVAL_MS = 1000;

function toBase64(bytes: Uint8Array): string {
  let bin = '';
  for (const b of bytes) bin += String.fromCharCode(b);
  return btoa(bin);
}

export interface PublisherConfig {
  relayUrl: string;
  namespace: string[];
  sources: SourceState[];
  onStatus: (status: PublishStatus, error?: string) => void;
}

interface TrackChannel {
  ftn: FullTrackName;
  controller: ReadableStreamDefaultController<MoqtObject>;
  groupId: bigint;
  objectId: bigint;
}

interface VideoSourceState {
  channel: TrackChannel;
  encoder: VideoEncoder;
  muxer: CMAFMuxer;
  config: VideoEncoderConfig;
  initData: string | null;
}

export class Publisher {
  private client: MOQtailClient | null = null;
  private catalogChannel: TrackChannel | null = null;
  // Each video source kind gets its own track, encoder, and muxer
  private videoTracks = new Map<string, VideoSourceState>();
  private catalogIntervalId: ReturnType<typeof setInterval> | null = null;
  private testSourceIntervalId: ReturnType<typeof setInterval> | null = null;
  public testStream: MediaStream | null = null;
  private videoProcessors: Array<{ processor: any; stop: () => void }> = [];
  private stopped = false;

  private cmsf: CMSF = { version: 1, tracks: [] };
  private startTimestamp = 0;

  constructor(private config: PublisherConfig) {}

  async start(): Promise<void> {
    const { relayUrl, namespace, sources, onStatus } = this.config;
    this.stopped = false;
    this.startTimestamp = performance.now() * 1000;

    try {
      logger.info('publisher', `connecting to relay: ${relayUrl}`);
      this.client = await MOQtailClient.new({ url: relayUrl });
      logger.info('publisher', 'relay connected');

      const ns = Tuple.fromUtf8Path(namespace.join('/'));
      await this.client.publishNamespace(ns);
      logger.info('publisher', `namespace announced: ${namespace.join('/')}`);

      this.catalogChannel = await this.setupTrack(namespace, 'catalog');

      const videoSources = sources.filter(
        s => s.enabled && ['camera', 'screen', 'test'].includes(s.kind),
      );
      logger.info('publisher', `sources: video=[${videoSources.map(s => s.kind).join(',')}]`);

      // Create a dedicated MOQ track + encoder + muxer for each video source
      for (const src of videoSources) {
        const kind = src.kind as 'camera' | 'screen' | 'test';
        let cfg: VideoEncoderConfig;

        if (kind === 'screen') {
          // Detect actual capture resolution for screen share quality
          const track = src.stream?.getVideoTracks()[0];
          const settings = track?.getSettings();
          const w = Math.min(settings?.width ?? 1280, 1280);
          const h = Math.min(settings?.height ?? 720, 720);
          cfg = { ...SCREEN_ENCODER_CONFIG, width: w, height: h };
        } else if (kind === 'test') {
          cfg = TEST_ENCODER_CONFIG;
        } else {
          // Use native camera resolution; bitrate scales with pixel count
          const track = src.stream?.getVideoTracks()[0];
          const settings = track?.getSettings();
          const w = settings?.width ?? 1280;
          const h = settings?.height ?? 720;
          const bitrate = Math.min(Math.round((w * h * 30 * 0.04) / 1000) * 1000, 4_000_000); // ~0.04 bpp, cap 4 Mbps for ISP upload limits
          cfg = { ...CAMERA_ENCODER_CONFIG_BASE, width: w, height: h, bitrate };
        }

        logger.info(
          'publisher',
          `video source: kind=${kind} resolution=${cfg.width}x${cfg.height} bitrate=${cfg.bitrate}`,
        );
        this.cmsf.tracks.push({
          name: kind,
          role: 'video',
          packaging: 'cmaf',
          codec: cfg.codec,
          width: cfg.width,
          height: cfg.height,
          bitrate: cfg.bitrate,
          timescale: 90000,
        });

        const channel = await this.setupTrack(namespace, kind);
        const muxer = this.createVideoMuxer(kind);
        const encoder = this.createVideoEncoder(kind, cfg, muxer);
        encoder.configure(cfg);

        this.videoTracks.set(kind, { channel, encoder, muxer, config: cfg, initData: null });
      }

      // Start per-source pipelines
      for (const src of videoSources) {
        const kind = src.kind as 'camera' | 'screen' | 'test';
        if (kind === 'test') {
          this.startTestSource(namespace.join('/'));
        } else if (kind === 'camera' && src.stream) {
          this.startCameraSource(src.stream, src.embedTimestamp ?? false);
        } else if (kind === 'screen' && src.stream) {
          this.startScreenSource(src.stream, src.embedTimestamp ?? false);
        }
      }

      this.publishCatalogOnce();
      this.catalogIntervalId = setInterval(() => this.publishCatalogOnce(), CATALOG_INTERVAL_MS);
      logger.info('publisher', 'catalog publish loop started');

      onStatus('publishing');
      logger.info('publisher', 'publishing');
    } catch (err) {
      logger.error('publisher', `start failed: ${(err as Error).message}`);
      if (!this.stopped) {
        onStatus('error', (err as Error).message);
        await this.stop();
      }
    }
  }

  async stop(): Promise<void> {
    if (this.stopped) return;
    this.stopped = true;

    if (this.catalogIntervalId !== null) {
      clearInterval(this.catalogIntervalId);
      this.catalogIntervalId = null;
    }
    if (this.testSourceIntervalId !== null) {
      clearInterval(this.testSourceIntervalId);
      this.testSourceIntervalId = null;
    }
    if (this.testStream) {
      this.testStream.getTracks().forEach(t => t.stop());
      this.testStream = null;
    }

    for (const { stop } of this.videoProcessors) stop();
    this.videoProcessors = [];

    for (const { encoder, channel } of this.videoTracks.values()) {
      try {
        encoder.close();
      } catch {
        /* ignore */
      }
      try {
        channel.controller.close();
      } catch {
        /* ignore */
      }
    }
    this.videoTracks.clear();

    try {
      this.catalogChannel?.controller.close();
    } catch {
      /* ignore */
    }

    if (this.client) {
      try {
        await this.client.publishNamespaceDone(Tuple.fromUtf8Path(this.config.namespace.join('/')));
      } catch {
        /* ignore */
      }
      try {
        await this.client.disconnect();
      } catch {
        /* ignore */
      }
      this.client = null;
    }
  }

  // ─── Private helpers ────────────────────────────────────────────────────────

  private async setupTrack(namespace: string[], trackName: string): Promise<TrackChannel> {
    const ftn = FullTrackName.tryNew(namespace.join('/'), trackName);
    let controller!: ReadableStreamDefaultController<MoqtObject>;
    const stream = new ReadableStream<MoqtObject>({
      start: c => {
        controller = c;
      },
    });

    this.client!.addOrUpdateTrack({
      fullTrackName: ftn,
      trackSource: { live: new LiveTrackSource(stream) },
      publisherPriority: trackName === 'catalog' ? 0 : 1,
    });
    const track = this.client!.trackSources.get(ftn.toString())!;
    await this.client!.publish(ftn, true, track.trackAlias!);
    logger.info('publisher', `track published: ${trackName} (alias=${track.trackAlias})`);

    return { ftn, controller, groupId: 0n, objectId: 0n };
  }

  private createVideoMuxer(kind: string): CMAFMuxer {
    const muxer = new CMAFMuxer();
    muxer.onInit = (moov: Uint8Array) => {
      const state = this.videoTracks.get(kind);
      if (state) {
        state.initData = toBase64(moov);
        logger.info('publisher', `${kind} muxer init segment ready: ${moov.byteLength}B`);
      }
    };
    muxer.onSegment = evt => {
      const state = this.videoTracks.get(kind);
      if (state) this.enqueueObject(state.channel, evt.data, evt.isKey);
    };
    return muxer;
  }

  private createVideoEncoder(
    kind: string,
    cfg: VideoEncoderConfig,
    muxer: CMAFMuxer,
  ): VideoEncoder {
    return new VideoEncoder({
      output: (chunk, meta) => {
        if (chunk.type === 'key' && !muxer['initialized']) {
          muxer.initVideo(
            cfg.width!,
            cfg.height!,
            (meta?.decoderConfig?.description as ArrayBuffer) ?? new ArrayBuffer(0),
          );
        }
        muxer.addVideoChunk(chunk, meta ?? undefined);
      },
      error: e => {
        if (!this.stopped) logger.error('publisher', `${kind} VideoEncoder error: ${e.message}`);
      },
    });
  }

  private makeTrackReader(
    track: MediaStreamTrack,
    onFrame: (f: VideoFrame) => void,
  ): { processor: any; stop: () => void } {
    // @ts-ignore - MediaStreamTrackProcessor is available in Chrome
    const processor = new MediaStreamTrackProcessor({ track });
    const reader = processor.readable.getReader();
    let running = true;
    const read = async () => {
      while (running) {
        const { value, done } = await reader.read();
        if (done || !value) break;
        onFrame(value);
      }
    };
    read();
    return {
      processor,
      stop: () => {
        running = false;
        reader.cancel();
      },
    };
  }

  private enqueueObject(channel: TrackChannel, payload: Uint8Array, isKey: boolean): void {
    if (this.stopped) return;
    if (isKey) {
      channel.groupId++;
      channel.objectId = 0n;
      logger.debug(
        'publisher',
        `keyframe → group=${channel.groupId} payload=${payload.byteLength}B`,
      );
    }
    const headers = new ExtensionHeaders()
      .addTimestamp(Date.now())
      .addVideoFrameMarking(isKey ? 1 : 0)
      .build();

    const obj = MoqtObject.newWithPayload(
      channel.ftn,
      new Location(channel.groupId, channel.objectId),
      1,
      ObjectForwardingPreference.Subgroup,
      0n,
      headers,
      payload,
    );
    channel.objectId++;
    try {
      channel.controller.enqueue(obj);
    } catch {
      logger.warn('publisher', 'video stream closed, dropping object');
    }
  }

  private publishCatalogOnce(): void {
    if (!this.catalogChannel || this.stopped) return;

    for (const [kind, state] of this.videoTracks.entries()) {
      if (state.initData) {
        const entry = this.cmsf.tracks.find(t => t.name === kind);
        if (entry) entry.initData = state.initData;
      }
    }
    const json = JSON.stringify(this.cmsf);
    const payload = new TextEncoder().encode(json);
    const ch = this.catalogChannel;
    const obj = MoqtObject.newWithPayload(
      ch.ftn,
      new Location(ch.groupId, 0n),
      0,
      ObjectForwardingPreference.Subgroup,
      0n,
      null,
      payload,
    );
    ch.groupId++;
    try {
      ch.controller.enqueue(obj);
    } catch {
      /* stream closed */
    }
  }

  // ─── Video sources ───────────────────────────────────────────────────────────

  private startCameraSource(stream: MediaStream, embedTimestamp: boolean): void {
    const state = this.videoTracks.get('camera');
    if (!state) return;
    const { width, height } = state.config;
    const track = stream.getVideoTracks()[0];
    if (!track) return;
    let frameCount = 0;

    if (embedTimestamp) {
      const canvas = new OffscreenCanvas(width!, height!);
      const ctx = canvas.getContext('2d')!;
      const p = this.makeTrackReader(track, frame => {
        if (this.stopped) {
          frame.close();
          return;
        }
        const isKey = frameCount % 60 === 0;
        ctx.drawImage(frame, 0, 0, width!, height!);
        this.drawTimestamp(ctx, width!, height!);
        const ts = frame.timestamp;
        frame.close();
        const out = new VideoFrame(canvas, { timestamp: ts, alpha: 'discard' });
        state.encoder.encode(out, { keyFrame: isKey });
        out.close();
        frameCount++;
      });
      this.videoProcessors.push(p);
    } else {
      // Encode native frames directly — no canvas copy, lowest latency
      const p = this.makeTrackReader(track, frame => {
        if (this.stopped) {
          frame.close();
          return;
        }
        const isKey = frameCount % 60 === 0;
        state.encoder.encode(frame, { keyFrame: isKey });
        frame.close();
        frameCount++;
      });
      this.videoProcessors.push(p);
    }
  }

  private startScreenSource(stream: MediaStream, embedTimestamp: boolean): void {
    const state = this.videoTracks.get('screen');
    if (!state) return;
    const { width, height } = state.config;
    const canvas = new OffscreenCanvas(width!, height!);
    const ctx = canvas.getContext('2d')!;
    let frameCount = 0;

    const track = stream.getVideoTracks()[0];
    if (!track) return;

    const p = this.makeTrackReader(track, frame => {
      if (this.stopped) {
        frame.close();
        return;
      }
      const isKey = frameCount % 60 === 0; // keyframe every 2s — screen content changes less
      ctx.drawImage(frame, 0, 0, width!, height!);
      if (embedTimestamp) this.drawTimestamp(ctx, width!, height!);
      const ts = frame.timestamp;
      frame.close();
      const out = new VideoFrame(canvas, { timestamp: ts, alpha: 'discard' });
      state.encoder.encode(out, { keyFrame: isKey });
      out.close();
      frameCount++;
    });
    this.videoProcessors.push(p);
  }

  private startTestSource(namespace: string): void {
    const state = this.videoTracks.get('test');
    if (!state) return;
    const { width, height, framerate } = state.config;
    const fps = framerate as number;

    const canvas = document.createElement('canvas');
    canvas.width = width!;
    canvas.height = height!;
    const ctx = canvas.getContext('2d')!;

    this.testStream = canvas.captureStream(fps);
    logger.info('publisher', 'test source started');

    const favicon = new Image();
    favicon.src = '/favicon.svg';

    let frameCount = 0;

    this.testSourceIntervalId = setInterval(() => {
      if (this.stopped) return;

      const now = performance.now();
      const ts = Math.round((now - this.startTimestamp / 1000) * 1000);

      ctx.fillStyle = '#0f0f1a';
      ctx.fillRect(0, 0, width!, height!);

      const hue = (now / 50) % 360;
      ctx.fillStyle = `hsl(${hue}, 80%, 40%)`;
      ctx.fillRect(0, height! - 8, width!, 8);

      const cellSize = 40;
      ctx.strokeStyle = '#1e2a4a';
      ctx.lineWidth = 1;
      for (let x = 0; x < width!; x += cellSize) {
        ctx.beginPath();
        ctx.moveTo(x, 0);
        ctx.lineTo(x, height!);
        ctx.stroke();
      }
      for (let y = 0; y < height!; y += cellSize) {
        ctx.beginPath();
        ctx.moveTo(0, y);
        ctx.lineTo(width!, y);
        ctx.stroke();
      }

      ctx.fillStyle = '#e2e8f0';
      ctx.font = 'bold 28px monospace';
      ctx.textBaseline = 'middle';
      ctx.fillText(namespace, 40, height! / 2 - 24);

      ctx.fillStyle = '#60a5fa';
      ctx.font = '20px monospace';
      ctx.fillText(new Date().toISOString(), 40, height! / 2 + 16);

      if (favicon.complete && favicon.naturalWidth > 0) {
        const gridCols = Math.floor(width! / cellSize);
        const gridRows = Math.floor(height! / cellSize);
        const imgH = 12 * cellSize;
        const imgW = Math.round(imgH * (1805 / 3197));
        const imgX = (gridCols - 2) * cellSize - imgW;
        const imgY = Math.floor((gridRows - 12) / 2) * cellSize;
        ctx.drawImage(favicon, imgX, imgY, imgW, imgH);
      }

      const isKey = frameCount % fps === 0;
      const frame = new VideoFrame(canvas, {
        timestamp: ts,
        duration: Math.round(1_000_000 / fps),
        alpha: 'discard',
      });
      state.encoder.encode(frame, { keyFrame: isKey });
      frame.close();
      frameCount++;
    }, 1000 / fps);
  }

  private drawTimestamp(ctx: OffscreenCanvasRenderingContext2D, w: number, h: number): void {
    const ts = new Date().toISOString();
    ctx.save();
    ctx.font = 'bold 16px monospace';
    ctx.textBaseline = 'bottom';
    const margin = 8;
    const pad = 6;
    const metrics = ctx.measureText(ts);
    const bw = metrics.width + pad * 2;
    const bh = 24;
    const bx = w - bw - margin;
    const by = h - bh - margin;
    ctx.fillStyle = 'rgba(0,0,0,0.55)';
    ctx.fillRect(bx, by, bw, bh);
    ctx.fillStyle = '#ffffff';
    ctx.fillText(ts, bx + pad, h - margin);
    ctx.restore();
  }
}
