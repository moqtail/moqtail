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
  DRAFT_14,
  FetchError,
  FetchType,
  FilterType,
  FullTrackName,
  GroupOrder,
  Location,
  MoqtObject,
  SubscribeError,
  Tuple,
} from 'moqtail';
import { MOQtailClient } from 'moqtail/client';
import { CMSFCatalog } from 'moqtail/model';
import { logger } from '@/lib/logger';
import { GoodputTracker } from '@/lib/goodput';

interface PendingSwitch {
  trackName: string;
  initData: ArrayBuffer;
  mimeType: string;
}

interface MOQStreamStruct {
  trackName: string;
  source: ReadableStream<MoqtObject>;
  requestId: bigint;
  tracker: GoodputTracker;
  lastGroupId: bigint;
  pendingSwitch: PendingSwitch | null;
  buffer?: {
    sourceBuffer: SourceBuffer;
    ac: AbortController;
  };
}

interface SubscribeOptions {
  trackName: string;
  priority?: number;
}

export interface PlayerOptions {
  /** The URL of the relay to connect to. */
  relayUrl: string;
  /** The namespace to use for this session. */
  namespace: Tuple;
  /** Whether to receive the catalog via SUBSCRIBE message. */
  receiveCatalogViaSubscribe?: boolean;
  /** Catalog location (default: group 0, object 1) */
  catalogLocation?: [Location, Location];
  /** Called when a switchTrack() completes (success or failure). Releases the ABR switching guard. */
  onTrackSwitched?: (trackName: string) => void;
}

const DefaultOptions = {
  relayUrl: 'https://relay.moqtail.dev',
  namespace: Tuple.fromUtf8Path('/moqtail'),
  receiveCatalogViaSubscribe: false,
  catalogLocation: [new Location(0n, 0n), new Location(0n, 1n)],
  onTrackSwitched: undefined as ((trackName: string) => void) | undefined,
} satisfies Required<Omit<PlayerOptions, 'onTrackSwitched'>> &
  Pick<PlayerOptions, 'onTrackSwitched'>;

export class Player {
  catalog: CMSFCatalog | null = null;
  client: MOQtailClient | null = null;

  #element: HTMLVideoElement | null = null;
  #mse?: MediaSource;
  #streams: MOQStreamStruct[] = [];
  #options: Required<Omit<PlayerOptions, 'onTrackSwitched'>> &
    Pick<PlayerOptions, 'onTrackSwitched'>;

  constructor(options: Partial<PlayerOptions> = {}) {
    this.#options = { ...DefaultOptions, ...options };
  }

  async initialize() {
    // If we already received the catalog, skip initialization
    if (this.catalog) return this.catalog;

    try {
      // Initialize the client and fetch the catalog
      this.client = await MOQtailClient.new({
        url: this.#options.relayUrl,
        supportedVersions: [DRAFT_14],
      });
    } catch (error) {
      logger.error('media', 'Failed to connect to relay', (error as Error).message);
      throw error;
    }

    // Fetch the catalog
    try {
      this.catalog = await this.retrieveCatalog();
    } catch (error) {
      logger.error('media', 'Failed to retrieve catalog', (error as Error).message);
      throw error;
    }

    return this.catalog;
  }

  async dispose() {
    // Unsubscribe from all active streams
    await Promise.all(this.#streams.map(s => this.unsubscribe(s.requestId)));

    // Close the client connection
    await this.client?.disconnect();

    // Reset state
    this.catalog = null;
    this.client = null;
    this.#element = null;
    this.#mse = undefined;
    this.#streams = [];
  }

  async attachMedia(element: HTMLVideoElement) {
    // Create a MediaSource and set it as the video element's source
    const mediaSource = new MediaSource();
    element.src = URL.createObjectURL(mediaSource);
    this.#element = element;
    this.#mse = mediaSource;
  }

  async addMediaTrack(trackName: string) {
    if (!this.#mse) throw new Error('MediaSource not initialized');
    if (!this.catalog) throw new Error('Catalog not loaded');
    if (!this.client) throw new Error('MOQProcessor not initialized');

    // We require a catalog entry to be present
    if (!this.catalog?.getByTrackName(trackName))
      throw new Error(`Track not found in catalog: ${trackName}`);

    // Verify packaging is playable by this player ('loc', 'cmaf', or 'chunk-per-object').
    if (!this.catalog.isCMAF(trackName))
      throw new Error(
        `Unsupported packaging type for track ${trackName}, only 'loc', 'cmaf', and 'chunk-per-object' are supported`,
      );

    // Get the stream struct
    const struct = await this.subscribe({ trackName });

    // Create new Source Buffer
    await this.#newSourceBufferMSE(struct, trackName);

    // Return the request ID
    return struct.requestId;
  }

  async startMedia() {
    if (!this.client) throw new Error('MOQProcessor not initialized');
    if (!this.#element) throw new Error('Media element not attached');
    if (this.#streams.length === 0) throw new Error('No active media streams to start');

    // Convenience function to wait for buffer updates
    const waitForBufferUpdate = (sourceBuffer: SourceBuffer) =>
      new Promise<void>(resolve =>
        sourceBuffer.addEventListener('updateend', () => resolve(), { once: true }),
      );

    // Seek behind the live edge so the player starts with buffer runway.
    // Without this offset the player lands on the live edge (0 s buffer),
    // immediately stalls, recovers for a moment, then stalls again —
    // creating the "video gets stuck" symptom.
    const LIVE_EDGE_STARTUP_OFFSET = 1.0; // seconds behind the live edge

    let gotNotification = 0;
    let target = 0;
    const bufferNotification = (end: number) => {
      if (gotNotification >= this.#streams.length) return false;

      // Start behind the live edge so there is buffer to consume while
      // new data continues arriving. The MSEBuffer module then fine-tunes
      // the distance via playback-rate adjustments (catchup / catchdown).
      target = Math.max(target, end - LIVE_EDGE_STARTUP_OFFSET);

      gotNotification++;
      if (gotNotification === this.#streams.length) {
        logger.info(
          'media',
          `All buffers ready, seeking to ${target.toFixed(2)}s (live edge ${end.toFixed(2)}s)`,
        );
        this.#element!.currentTime = target;
        this.#element!.play();
      }
      return true;
    };

    // Iterate over all added roles
    for (const struct of this.#streams) {
      // Get the init segment for the track
      const initSegment = this.catalog?.getInitData(struct.trackName);
      if (!initSegment) {
        await this.unsubscribe(struct.requestId);
        throw new Error(`Failed to get init segment for track: ${struct.trackName}`);
      }

      // Get the Buffer and AbortController for this track
      const { sourceBuffer, ac } = struct.buffer!;

      // Append the init segment
      try {
        sourceBuffer.appendBuffer(initSegment);
        await waitForBufferUpdate(sourceBuffer);
      } catch (error) {
        await this.unsubscribe(struct.requestId);
        throw new Error(
          `Failed to append init segment for track ${struct.trackName}: ${(error as Error).message}`,
        );
      }

      // MSE State
      let lastMSEErrorLogged = 0;
      let kickStarted = false;

      // Create the WritableStream to handle incoming objects
      const writable = new WritableStream<MoqtObject>({
        write: async (object, controller) => {
          try {
            // Skip end-of-group objects
            if (object.isEndOfGroup()) {
              logger.info(
                'media',
                `Received end-of-group object for track ${struct.trackName}, ignoring`,
              );
              return;
            }

            // Make TypeScript happy
            if (!(object.payload?.buffer instanceof ArrayBuffer)) {
              console.warn('Received non-ArrayBuffer payload, ignoring', object);
              return;
            }

            // Cancel if aborted
            if (ac.signal.aborted) {
              controller.error(new DOMException('Stream aborted', 'InternalError'));
              return;
            }

            // Init segment re-injection after a seamless track switch.
            // Detect the transition by comparing the object's fullTrackName
            // (resolved from the wire's track_alias) against the target track.
            // The previous approach (group !== lastGroupId) fired too early —
            // the relay may continue sending old-track groups after the SWITCH
            // is acknowledged, so a group boundary change does NOT imply a track
            // transition. Checking the actual track name is authoritative.
            const objectTrackName = struct.pendingSwitch
              ? new TextDecoder().decode(object.fullTrackName.name)
              : null;
            if (struct.pendingSwitch && objectTrackName === struct.pendingSwitch.trackName) {
              const { initData, mimeType, trackName: newTrackName } = struct.pendingSwitch;
              struct.trackName = newTrackName;
              struct.pendingSwitch = null;

              // changeType() must not be called while the SourceBuffer is updating
              if (sourceBuffer.updating) await waitForBufferUpdate(sourceBuffer);
              sourceBuffer.changeType(mimeType);
              sourceBuffer.appendBuffer(initData);
              await waitForBufferUpdate(sourceBuffer);

              // NOW release the ABR switching guard — the relay has completed the
              // transition and delivered data on the new track. Safe to switch again.
              this.#options.onTrackSwitched?.(newTrackName);
            }

            // Append the data
            let maxRetries = 5;
            while (maxRetries--) {
              try {
                // Append the data
                sourceBuffer.appendBuffer(object.payload.buffer);

                // Wait for the source buffer to be consumed
                await waitForBufferUpdate(sourceBuffer);
                break;
              } catch (error) {
                // Wait for the source buffer to be ready
                if (sourceBuffer.updating) await waitForBufferUpdate(sourceBuffer);
                else if (lastMSEErrorLogged + 5000 < performance.now()) {
                  lastMSEErrorLogged = performance.now();
                  logger.error(
                    'media',
                    `Error appending to SourceBuffer, retrying... (${maxRetries} attempts left)`,
                  );
                }
              }
            }

            // Check the buffered amount
            if (sourceBuffer.buffered.length > 0 && !kickStarted) {
              const minStart = sourceBuffer.buffered.start(0);
              const maxEnd = sourceBuffer.buffered.end(sourceBuffer.buffered.length - 1);
              const bufferDuration = maxEnd - minStart;
              if (bufferDuration > 1.0) bufferNotification(maxEnd);
            }

            // Record goodput sample — tracker uses internal windowed byte counter
            struct.tracker.recordObject(object.payload.byteLength, 0);
            // Track last seen group for switch boundary detection
            struct.lastGroupId = object.location.group;
          } catch (error) {
            logger.error('media', 'Error processing media object:', error);
            controller.error(error);
          }
        },
      });

      // Pipe to the writable stream
      const promise = struct.source.pipeTo(writable, { signal: ac.signal });

      // Cleanup stream — for live streams, do NOT call endOfStream() when the
      // pipe ends. The readable stream can close transiently (e.g., during a
      // SWITCH, relay reconnection, or subscription update). Calling endOfStream()
      // permanently seals the MediaSource, preventing any further data from being
      // appended. Only call endOfStream() when the player is being disposed.
      promise.catch(error => {
        if (!['AbortError', 'InternalError'].includes(error.name)) {
          logger.error('media', 'Stream pipe error:', error);
        }
      });
    }
  }

  getMetrics(): {
    bandwidthBps: number;
    fastEmaBps: number;
    slowEmaBps: number;
    bufferSeconds: number;
    activeTrack: string | null;
    droppedFrames: number;
    totalFrames: number;
    playbackRate: number;
    deliveryTimeMs: number;
    lastObjectBytes: number;
  } {
    const videoStruct = this.#streams.find(s => this.catalog?.getRole(s.trackName) === 'video');
    const buffered = this.#element?.buffered;
    const bufferSeconds =
      buffered && buffered.length > 0 && this.#element
        ? Math.max(0, buffered.end(buffered.length - 1) - this.#element.currentTime)
        : 0;
    const quality = this.#element?.getVideoPlaybackQuality?.();
    return {
      bandwidthBps: videoStruct?.tracker.getBandwidthBps() ?? 0,
      fastEmaBps: videoStruct?.tracker.getFastEmaBps() ?? 0,
      slowEmaBps: videoStruct?.tracker.getSlowEmaBps() ?? 0,
      bufferSeconds,
      activeTrack: videoStruct?.trackName ?? null,
      droppedFrames: quality?.droppedVideoFrames ?? 0,
      totalFrames: quality?.totalVideoFrames ?? 0,
      playbackRate: this.#element?.playbackRate ?? 1,
      deliveryTimeMs: videoStruct?.tracker.getLastDeliveryTimeMs() ?? 0,
      lastObjectBytes: videoStruct?.tracker.getLastObjectBytes() ?? 0,
    };
  }

  setEmaAlphas(alphaFast: number, alphaSlow: number): void {
    const videoStruct = this.#streams.find(s => this.catalog?.getRole(s.trackName) === 'video');
    videoStruct?.tracker.setAlphas(alphaFast, alphaSlow);
  }

  /**
   * Updates the onTrackSwitched callback post-construction.
   * Called by app.tsx after creating the Player and AbrController,
   * to wire the ABR switching guard release without a circular dependency.
   */
  setOnTrackSwitched(cb: (trackName: string) => void): void {
    this.#options.onTrackSwitched = cb;
  }

  /**
   * Seamlessly switches the active video track using the MoQ SWITCH message.
   * The relay will complete delivery of the current group then begin sending
   * the new track. The WritableStream.write handler detects the group boundary
   * and re-injects the new init segment before appending the first new payload.
   *
   * Fire-and-forget from AbrController: do NOT await this externally.
   * The #switching guard in AbrController is released via onTrackSwitched callback.
   */
  async switchTrack(trackName: string): Promise<void> {
    if (!this.client) return;
    if (!this.catalog) return;

    const videoStruct = this.#streams.find(s => this.catalog?.getRole(s.trackName) === 'video');
    if (!videoStruct) return;

    const fullTrackName = getFullTrackName(this.#options.namespace, trackName);
    const initData = this.catalog.getInitData(trackName);
    const role = this.catalog.getRole(trackName);
    const codec = this.catalog.getCodecString(trackName);

    if (!initData || !role || !codec) {
      logger.error('media', `switchTrack: missing catalog data for track ${trackName}`);
      this.#options.onTrackSwitched?.(videoStruct.trackName);
      return;
    }

    const mimeType = `${role}/mp4; codecs="${codec}"`;

    try {
      const result = await this.client.switch({
        fullTrackName,
        subscriptionRequestId: videoStruct.requestId,
      });

      if (result instanceof SubscribeError) {
        logger.error(
          'media',
          `switchTrack: SWITCH rejected for ${trackName}:`,
          result.errorReason.phrase,
        );
        this.#options.onTrackSwitched?.(videoStruct.trackName);
        return;
      }

      // Success: update requestId, arm the write handler.
      // Do NOT reset the tracker — the bandwidth estimate from the previous
      // track is still a valid indicator of network capacity. Resetting it
      // creates a blind spot where ABR rules see 0 bandwidth and can't
      // downgrade if the new track is too aggressive. (dash.js doesn't reset
      // throughput on quality switches either.)
      videoStruct.requestId = result.requestId;
      // Arm the write handler for init segment re-injection at the next group
      // boundary. The onTrackSwitched callback (which releases the ABR switching
      // guard) is NOT called here — it fires in the write handler AFTER the relay
      // has actually delivered data on the new track. This prevents rapid
      // consecutive SWITCH messages that corrupt the relay's switch context.
      videoStruct.pendingSwitch = { trackName, initData: initData.buffer as ArrayBuffer, mimeType };

      // Safety timeout: if the relay never delivers data on the new track
      // (e.g. relay switch-context race condition where the send stream is
      // not found), release the switching guard so the system isn't locked
      // forever. The pendingSwitch is cleared so stale init data isn't
      // injected if the new track eventually arrives much later.
      const SWITCH_TIMEOUT_MS = 5000;
      const pendingRef = videoStruct.pendingSwitch;
      setTimeout(() => {
        if (videoStruct.pendingSwitch === pendingRef) {
          logger.warn(
            'media',
            `switchTrack: timeout waiting for ${trackName} data — releasing switching guard`,
          );
          videoStruct.pendingSwitch = null;
          this.#options.onTrackSwitched?.(videoStruct.trackName);
        }
      }, SWITCH_TIMEOUT_MS);
    } catch (error) {
      logger.error('media', 'switchTrack: unexpected error', error);
      this.#options.onTrackSwitched?.(videoStruct.trackName);
    }
  }

  async #newSourceBufferMSE(struct: MOQStreamStruct, trackName: string) {
    if (!this.#mse) throw new Error('MediaSource not initialized');

    // Wait for media source to be open
    if (this.#mse.readyState === 'closed') {
      await new Promise(resolve => {
        const onSourceOpen = () => {
          this.#mse!.removeEventListener('sourceopen', onSourceOpen);
          resolve(true);
        };
        this.#mse!.addEventListener('sourceopen', onSourceOpen);
      });
    }

    // Get the MIME type
    const codecString = this.catalog?.getCodecString(trackName);
    const role = this.catalog?.getRole(trackName);
    if (!codecString || !role) {
      await this.unsubscribe(struct.requestId);
      throw new Error(`Failed to get codec or role for track: ${trackName}`);
    }

    // Check if the MIME type is supported
    const mimeType = `${role}/mp4; codecs="${codecString}"`;
    if (!MediaSource.isTypeSupported(mimeType)) {
      await this.unsubscribe(struct.requestId);
      throw new Error(`MIME type not supported: ${mimeType}`);
    }

    // Create a new SourceBuffer
    const sourceBuffer = this.#mse.addSourceBuffer(mimeType);

    // Register the SourceBuffer
    struct.buffer = {
      ac: new AbortController(),
      sourceBuffer,
    };
  }

  async retrieveCatalog(): Promise<CMSFCatalog> {
    if (!this.client) throw new Error('MOQProcessor not initialized');

    let struct: MOQStreamStruct;
    if (this.#options.receiveCatalogViaSubscribe) {
      struct = await this.subscribe({ trackName: 'catalog', priority: 0 });
    } else {
      const result = await this.client.fetch({
        groupOrder: GroupOrder.Original,
        priority: 0,
        typeAndProps: {
          type: FetchType.StandAlone,
          props: {
            fullTrackName: getFullTrackName(this.#options.namespace, 'catalog'),
            startLocation: this.#options.catalogLocation[0],
            endLocation: this.#options.catalogLocation[1],
          },
        },
      });
      if (result instanceof FetchError)
        throw new Error(`Error occured during catalog fetch: ${result.reasonPhrase.phrase}`);
      struct = {
        trackName: 'catalog',
        requestId: result.requestId,
        source: result.stream,
        tracker: new GoodputTracker(),
        lastGroupId: -1n,
        pendingSwitch: null,
      };
    }

    // Pull the latest catalog object
    if (!struct.source) {
      throw new Error(
        'Catalog stream unavailable — the publisher may have disconnected. Restart the relay and publisher, then reconnect.',
      );
    }
    const reader = struct.source.getReader();
    let buffer: ArrayBufferLike | undefined;
    while (!buffer) {
      const result = await reader.read();
      if (result.done) {
        reader.releaseLock();
        throw new Error('Catalog stream closed unexpectedly while waiting for data');
      }
      const value = result.value;
      if (value.isEndOfGroup()) continue;
      if (!value.payload?.buffer) {
        logger.warn('media', 'Received catalog object without payload, ignoring');
        continue;
      }
      buffer = value.payload.buffer;
    }

    // Parse and store the catalog
    const catalog = CMSFCatalog.from(buffer);

    // Unsubscribe from the catalog stream since we only needed the latest object
    if (this.#options.receiveCatalogViaSubscribe) await this.unsubscribe(struct.requestId);
    return catalog;
  }

  private async subscribe(params: SubscribeOptions): Promise<MOQStreamStruct> {
    if (!this.client) throw new Error('MOQProcessor not initialized');

    // Send the appropriate control message
    let struct: MOQStreamStruct;
    const result = await this.client.subscribe({
      fullTrackName: getFullTrackName(this.#options.namespace, params.trackName),
      groupOrder: GroupOrder.Original,
      filterType: FilterType.LatestObject,
      forward: true,
      priority: params.priority ?? 0,
    });
    if (result instanceof SubscribeError)
      throw new Error(`Error occured during subscription: ${result.errorReason.phrase}`);
    struct = {
      trackName: params.trackName,
      requestId: result.requestId,
      source: result.stream,
      tracker: new GoodputTracker(),
      lastGroupId: -1n,
      pendingSwitch: null,
    };

    // Add the stream to the pool
    this.#streams.push(struct);
    return struct;
  }

  private async unsubscribe(requestId: bigint) {
    if (!this.client) throw new Error('MOQProcessor not initialized');

    // Find the stream struct
    const index = this.#streams.findIndex(s => s.requestId === requestId);
    if (index === -1) throw new Error(`No active subscription found for requestId ${requestId}`);
    const struct = this.#streams[index];
    if (!struct) throw new Error(`No active subscription found for requestId ${requestId}`);

    // Send the UNSUBSCRIBE message
    await this.client.unsubscribe(struct.requestId);

    // Remove the stream from the pool
    this.#streams.splice(index, 1);
  }
}

function getFullTrackName(ns: Tuple, name: string): FullTrackName {
  return FullTrackName.tryNew(ns, new TextEncoder().encode(name));
}
