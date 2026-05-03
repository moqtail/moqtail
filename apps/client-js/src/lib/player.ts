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
  FetchType,
  FilterType,
  FullTrackName,
  GroupOrder,
  Location,
  LogLevel,
  MoqtObject,
  Tuple,
} from 'moqtail';
import { MOQtailClient } from 'moqtail/client';
import { CMSFCatalog, RequestError } from 'moqtail/model';
import { logger } from '@/lib/logger';

interface MOQStreamStruct {
  trackName: string;
  source: ReadableStream<MoqtObject>;
  requestId: bigint;
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
}

const DefaultOptions: Required<PlayerOptions> = {
  relayUrl: 'https://relay.moqtail.dev',
  namespace: Tuple.fromUtf8Path('/moqtail'),
  receiveCatalogViaSubscribe: false,
  catalogLocation: [new Location(0n, 0n), new Location(0n, 1n)],
};

export class Player {
  catalog: CMSFCatalog | null = null;
  client: MOQtailClient | null = null;

  #element: HTMLVideoElement | null = null;
  #mse?: MediaSource;
  #streams: MOQStreamStruct[] = [];
  #options: Required<PlayerOptions>;

  constructor(options: Partial<PlayerOptions> = {}) {
    this.#options = { ...DefaultOptions, ...options };
  }

  async initialize() {
    // If we already received the catalog, skip initialization
    if (this.catalog) return this.catalog;

    logger.info('player', `Connecting to relay: ${this.#options.relayUrl}`);
    try {
      this.client = await MOQtailClient.new({
        url: this.#options.relayUrl,
        callbacks: {
          onMessageSent: msg => logger.debug('player', `control →relay: ${msg.constructor.name}`),
          onMessageReceived: msg =>
            logger.debug('player', `control ←relay: ${msg.constructor.name}`),
        },
      });
      MOQtailClient.setLogLevel(LogLevel.WARN);
      logger.info('player', 'Connected to relay');
    } catch (error) {
      logger.error('media', 'Failed to connect to relay', (error as Error).message);
      throw error;
    }

    logger.info('player', 'Retrieving catalog...');
    try {
      this.catalog = await this.retrieveCatalog();
      logger.info('player', `Catalog retrieved — ${this.catalog.getTracks().length} track(s)`);
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

    logger.info('player', `addMediaTrack: "${trackName}"`);

    // We require a catalog entry to be present
    if (!this.catalog?.getByTrackName(trackName))
      throw new Error(`Track not found in catalog: ${trackName}`);

    // Verify packaging is 'cmaf' or 'chunk-per-object'
    if (!this.catalog.isCMAF(trackName))
      throw new Error(
        `Unsupported packaging type for track ${trackName}, only 'cmaf' and 'chunk-per-object' are supported`,
      );

    const codecString = this.catalog.getCodecString(trackName);
    const role = this.catalog.getRole(trackName);
    logger.info('player', `addMediaTrack: "${trackName}" role=${role} codec=${codecString}`);

    // Get the stream struct
    const struct = await this.subscribe({ trackName });

    // Create new Source Buffer
    await this.#newSourceBufferMSE(struct, trackName);

    logger.info(
      'player',
      `addMediaTrack: SourceBuffer created for "${trackName}" requestId=${struct.requestId}`,
    );

    // Return the request ID
    return struct.requestId;
  }

  async startMedia() {
    if (!this.client) throw new Error('MOQProcessor not initialized');
    if (!this.#element) throw new Error('Media element not attached');
    if (this.#streams.length === 0) throw new Error('No active media streams to start');

    logger.info(
      'player',
      `startMedia: ${this.#streams.length} stream(s), MSE state="${this.#mse?.readyState}"`,
    );

    // Convenience function to wait for buffer updates
    const waitForBufferUpdate = (sourceBuffer: SourceBuffer) =>
      new Promise<void>(resolve =>
        sourceBuffer.addEventListener('updateend', () => resolve(), { once: true }),
      );

    // Seek to buffer end
    let gotNotification = 0;
    let target = 0;
    const bufferNotification = (end: number) => {
      if (gotNotification >= this.#streams.length) return false;

      // For live, seek to the max end, for VOD seek to the min start
      target = Math.max(target, end);

      gotNotification++;
      logger.info(
        'player',
        `bufferNotification: stream ${gotNotification}/${this.#streams.length} ready, bufferEnd=${end.toFixed(3)}s`,
      );
      if (gotNotification === this.#streams.length) {
        logger.info(
          'player',
          `All buffers ready — seeking to ${target.toFixed(3)}s and calling play()`,
        );
        this.#element!.currentTime = target;
        this.#element!.play()
          .then(() => logger.info('player', 'play() resolved'))
          .catch(e => logger.error('player', 'play() rejected', e));
      }
      return true;
    };

    // Iterate over all added roles
    for (const struct of this.#streams) {
      logger.info(
        'player',
        `startMedia: setting up stream for "${struct.trackName}" requestId=${struct.requestId}`,
      );

      // Get the init segment for the track
      const initSegment = this.catalog?.getInitData(struct.trackName);
      if (!initSegment) {
        await this.unsubscribe(struct.requestId);
        throw new Error(`Failed to get init segment for track: ${struct.trackName}`);
      }
      logger.debug(
        'player',
        `startMedia: init segment size=${initSegment.byteLength}B for "${struct.trackName}"`,
      );

      // Get the Buffer and AbortController for this track
      const { sourceBuffer, ac } = struct.buffer!;

      // Append the init segment
      try {
        sourceBuffer.appendBuffer(initSegment);
        await waitForBufferUpdate(sourceBuffer);
        logger.info('player', `startMedia: init segment appended for "${struct.trackName}"`);
      } catch (error) {
        await this.unsubscribe(struct.requestId);
        throw new Error(
          `Failed to append init segment for track ${struct.trackName}: ${(error as Error).message}`,
        );
      }

      // MSE State
      let lastMSEErrorLogged = 0;
      let kickStarted = false;
      let objectCount = 0;

      // Create the WritableStream to handle incoming objects
      const writable = new WritableStream<MoqtObject>({
        write: async (object, controller) => {
          try {
            // Skip end-of-group objects
            if (object.isEndOfGroup()) {
              logger.debug(
                'player',
                `[${struct.trackName}] end-of-group (group=${object.groupId}, obj=${object.objectId})`,
              );
              return;
            }

            objectCount++;
            if (objectCount === 1) {
              logger.info(
                'player',
                `[${struct.trackName}] first object received — group=${object.groupId} obj=${object.objectId} size=${object.payload?.byteLength ?? 0}B`,
              );
            } else if (objectCount % 30 === 0) {
              const buf = sourceBuffer.buffered;
              const bufEnd = buf.length > 0 ? buf.end(buf.length - 1).toFixed(3) : 'none';
              logger.debug(
                'player',
                `[${struct.trackName}] object #${objectCount} group=${object.groupId} obj=${object.objectId} bufferEnd=${bufEnd}s`,
              );
            }

            // Make TypeScript happy
            if (!(object.payload?.buffer instanceof ArrayBuffer)) {
              logger.warn(
                'player',
                `[${struct.trackName}] non-ArrayBuffer payload, ignoring (type=${typeof object.payload?.buffer})`,
              );
              return;
            }

            // Cancel if aborted
            if (ac.signal.aborted) {
              controller.error(new DOMException('Stream aborted', 'InternalError'));
              return;
            }

            // Append the data
            let maxRetries = 5;
            while (maxRetries--) {
              try {
                sourceBuffer.appendBuffer(object.payload.buffer);
                await waitForBufferUpdate(sourceBuffer);
                break;
              } catch (error) {
                // Wait for the source buffer to be ready
                if (sourceBuffer.updating) await waitForBufferUpdate(sourceBuffer);
                else if (lastMSEErrorLogged + 5000 < performance.now()) {
                  lastMSEErrorLogged = performance.now();
                  logger.error(
                    'player',
                    `[${struct.trackName}] SourceBuffer appendBuffer failed, retrying (${maxRetries} left): ${(error as Error).message}`,
                  );
                }
              }
            }

            // Check the buffered amount
            if (sourceBuffer.buffered.length > 0 && !kickStarted) {
              const minStart = sourceBuffer.buffered.start(0);
              const maxEnd = sourceBuffer.buffered.end(sourceBuffer.buffered.length - 1);
              const bufferDuration = maxEnd - minStart;
              logger.debug(
                'player',
                `[${struct.trackName}] buffered ${bufferDuration.toFixed(3)}s (${minStart.toFixed(3)}–${maxEnd.toFixed(3)}s), kickStarted=${kickStarted}`,
              );
              if (bufferDuration > 1.0) {
                kickStarted = true;
                bufferNotification(maxEnd);
              }
            }
          } catch (error) {
            logger.error('media', 'Error processing media object:', error);
            controller.error(error);
          }
        },
      });

      // Pipe to the writable stream
      logger.info('player', `startMedia: piping "${struct.trackName}" stream into WritableStream`);
      const promise = struct.source.pipeTo(writable, { signal: ac.signal });

      // Cleanup stream
      promise
        .catch(error => {
          if (!['AbortError', 'InternalError'].includes(error.name)) {
            logger.error(
              'player',
              `[${struct.trackName}] pipeTo error: ${error.name} — ${error.message}`,
            );
            throw error;
          }
          logger.info('player', `[${struct.trackName}] pipeTo ended: ${error.name}`);
        })
        .finally(async () => {
          logger.info(
            'player',
            `[${struct.trackName}] stream finished — MSE readyState="${this.#mse!.readyState}"`,
          );
          if (this.#mse!.readyState === 'open') this.#mse!.endOfStream();
        });
    }
  }

  async #newSourceBufferMSE(struct: MOQStreamStruct, trackName: string) {
    if (!this.#mse) throw new Error('MediaSource not initialized');

    logger.info(
      'player',
      `#newSourceBufferMSE: "${trackName}" MSE readyState="${this.#mse.readyState}"`,
    );

    // Wait for media source to be open
    if (this.#mse.readyState === 'closed') {
      logger.info('player', '#newSourceBufferMSE: waiting for MSE sourceopen...');
      await new Promise(resolve => {
        const onSourceOpen = () => {
          this.#mse!.removeEventListener('sourceopen', onSourceOpen);
          logger.info('player', '#newSourceBufferMSE: MSE sourceopen fired');
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
    logger.info(
      'player',
      `#newSourceBufferMSE: mimeType="${mimeType}" supported=${MediaSource.isTypeSupported(mimeType)}`,
    );
    if (!MediaSource.isTypeSupported(mimeType)) {
      await this.unsubscribe(struct.requestId);
      throw new Error(`MIME type not supported: ${mimeType}`);
    }

    // Create a new SourceBuffer
    const sourceBuffer = this.#mse.addSourceBuffer(mimeType);
    logger.info('player', `#newSourceBufferMSE: SourceBuffer added for "${trackName}"`);

    // Register the SourceBuffer
    struct.buffer = {
      ac: new AbortController(),
      sourceBuffer,
    };
  }

  async retrieveCatalog(): Promise<CMSFCatalog> {
    if (!this.client) throw new Error('MOQProcessor not initialized');

    const ns = this.#options.namespace.toUtf8Path();
    const via = this.#options.receiveCatalogViaSubscribe ? 'SUBSCRIBE' : 'FETCH';
    logger.info('player', `retrieveCatalog: ns="${ns}" via=${via}`);

    let struct: MOQStreamStruct;
    if (this.#options.receiveCatalogViaSubscribe) {
      struct = await this.subscribe({ trackName: 'catalog', priority: 0 });
    } else {
      const [startLoc, endLoc] = this.#options.catalogLocation;
      logger.info(
        'player',
        `retrieveCatalog: FETCH group=${startLoc.group}:${startLoc.object} → ${endLoc.group}:${endLoc.object}`,
      );
      const result = await this.client.fetch({
        groupOrder: GroupOrder.Original,
        priority: 0,
        typeAndProps: {
          type: FetchType.Standalone,
          props: {
            fullTrackName: getFullTrackName(this.#options.namespace, 'catalog'),
            startLocation: startLoc,
            endLocation: endLoc,
          },
        },
      });
      if (result instanceof RequestError) {
        logger.error(
          'player',
          `retrieveCatalog: FETCH failed — code=${result.errorCode} reason="${result.reasonPhrase.phrase}"`,
        );
        throw new Error(`Error occured during catalog fetch: ${result.reasonPhrase.phrase}`);
      }
      logger.info('player', `retrieveCatalog: FETCH OK requestId=${result.requestId}`);
      struct = {
        trackName: 'catalog',
        requestId: result.requestId,
        source: result.stream,
      };
    }

    // Pull the latest catalog object
    const reader = struct.source.getReader();
    let buffer: ArrayBufferLike | undefined;
    let objectsRead = 0;
    while (!buffer) {
      const result = await reader.read();
      if (result.done) {
        logger.error(
          'player',
          `retrieveCatalog: stream closed after ${objectsRead} object(s) — no catalog data`,
        );
        await reader.releaseLock();
        throw new Error('Catalog stream closed unexpectedly while waiting for data');
      }
      const value = result.value;
      objectsRead++;
      if (value.isEndOfGroup()) {
        logger.debug('player', `retrieveCatalog: skipping end-of-group object (#${objectsRead})`);
        continue;
      }
      if (!value.payload?.buffer) {
        logger.warn('media', 'Received catalog object without payload, ignoring');
        continue;
      }
      logger.info(
        'player',
        `retrieveCatalog: got catalog payload — ${value.payload.byteLength}B (after ${objectsRead} read(s))`,
      );
      buffer = value.payload.buffer;
    }

    // Parse and store the catalog
    const catalog = CMSFCatalog.from(buffer);
    const tracks = catalog.getTracks();
    logger.info(
      'player',
      `retrieveCatalog: parsed ${tracks.length} track(s): ${tracks.map(t => `${t.name}(${t.role})`).join(', ')}`,
    );

    if (this.#options.receiveCatalogViaSubscribe) await this.unsubscribe(struct.requestId);
    return catalog;
  }

  private async subscribe(params: SubscribeOptions): Promise<MOQStreamStruct> {
    if (!this.client) throw new Error('MOQProcessor not initialized');

    const ftn = getFullTrackName(this.#options.namespace, params.trackName);
    logger.info(
      'player',
      `subscribe: "${params.trackName}" ftn="${ftn.toString()}" priority=${params.priority ?? 0}`,
    );

    const result = await this.client.subscribe({
      fullTrackName: ftn,
      groupOrder: GroupOrder.Original,
      filterType: FilterType.LatestObject,
      forward: true,
      priority: params.priority ?? 0,
    });
    if (result instanceof RequestError) {
      logger.error(
        'player',
        `subscribe: "${params.trackName}" failed — code=${result.errorCode} reason="${result.reasonPhrase.phrase}"`,
      );
      throw new Error(`Error occured during subscription: ${result.reasonPhrase.phrase}`);
    }
    logger.info('player', `subscribe: "${params.trackName}" OK requestId=${result.requestId}`);

    const struct: MOQStreamStruct = {
      trackName: params.trackName,
      requestId: result.requestId,
      source: result.stream,
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

    logger.info('player', `unsubscribe: "${struct.trackName}" requestId=${requestId}`);
    await this.client.unsubscribe(struct.requestId);
    this.#streams.splice(index, 1);
  }
}

function getFullTrackName(ns: Tuple, name: string): FullTrackName {
  return FullTrackName.tryNew(ns, new TextEncoder().encode(name));
}
