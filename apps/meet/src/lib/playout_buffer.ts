/**
 * Copyright 2025 The MOQtail Authors
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

import { MoqtObject, ExtensionHeaders, ExtensionHeader } from 'moqtail/model';
import Heap from 'heap-js';

const DEFAULT_TARGET_LATENCY_MS = 100;
const DEFAULT_MAX_LATENCY_MS = 1000;

export interface BufferedObject {
  object: MoqtObject;
  createdAt: number;
}

export interface Clock {
  now(): number;
}

export class PlayoutBuffer {
  #reader: ReadableStreamDefaultReader<MoqtObject>;
  #buffer: Heap<BufferedObject> = new Heap((a: BufferedObject, b: BufferedObject) => {
    return a.object.location.compare(b.object.location);
  });
  #isRunning: boolean = true;
  #targetLatencyMs: number;
  #maxLatencyMs: number;
  #clock: Clock | undefined;

  onObject: ((obj: MoqtObject | null) => void) | null = null;

  constructor(
    objectStream: ReadableStream<MoqtObject>,
    readonly options?: {
      targetLatencyMs: number;
      maxLatencyMs: number;
      clock: Clock;
    },
  ) {
    this.#targetLatencyMs = this.options?.targetLatencyMs ?? DEFAULT_TARGET_LATENCY_MS;
    this.#maxLatencyMs = this.options?.maxLatencyMs ?? DEFAULT_MAX_LATENCY_MS;
    this.#clock = this.options?.clock;
    this.#reader = objectStream.getReader();
    this.#fillBuffer();
    this.#serveBuffer();
  }

  cleanup(): void {
    this.#isRunning = false;
    this.onObject?.(null);
  }

  #getNormalizedTime(): number {
    return this.#clock ? this.#clock.now() : Date.now();
  }

  async #serveBuffer(): Promise<void> {
    while (this.#isRunning) {
      const now = this.#getNormalizedTime();
      const oldest = this.#buffer.peek();

      if (oldest && this.onObject) {
        const timeUntilReady = this.#targetLatencyMs - (now - oldest.createdAt);
        if (timeUntilReady <= 0) {
          const bufferedObj = this.#buffer.pop()!;
          this.onObject(bufferedObj.object);
        } else {
          await new Promise(resolve => setTimeout(resolve, Math.min(timeUntilReady, 50)));
        }
      } else {
        await new Promise(resolve => setTimeout(resolve, 10));
      }
    }
  }

  async #fillBuffer(): Promise<void> {
    while (this.#isRunning) {
      try {
        const { value, done } = await this.#reader.read();
        if (done) {
          this.cleanup();
          return;
        }
        this.#evictOnMaxLatency();
        this.#buffer.push({
          object: value,
          createdAt: this.#extractCreatedAt(value),
        });
      } catch {
        this.cleanup();
      }
    }
  }

  #extractCreatedAt(object: MoqtObject): number {
    if (object.extensionHeaders) {
      const headers = ExtensionHeaders.fromKeyValuePairs(object.extensionHeaders);
      for (const header of headers) {
        if (ExtensionHeader.isCaptureTimestamp(header)) {
          return Number(header.timestamp);
        }
      }
    }
    return this.#getNormalizedTime();
  }

  #evictOnMaxLatency(): void {
    const now = this.#getNormalizedTime();
    const oldest = this.#buffer.peek();
    if (oldest && now - oldest.createdAt > this.#maxLatencyMs) {
      if (oldest.object.location.object === 0n) {
        this.#dropGop(oldest.object.location.group);
      } else {
        this.#buffer.pop();
      }
      this.#evictOnMaxLatency();
    }
  }

  #dropGop(groupId: bigint): void {
    while (this.#buffer.length > 0) {
      const oldest = this.#buffer.peek();
      if (oldest && oldest.object.location.group === groupId) {
        this.#buffer.pop();
      } else {
        break;
      }
    }
  }
}
