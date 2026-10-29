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

import { MFUReassembler, processMMTPWithReassembly, type ReassembledMFU } from '@blockcast/mmt-container/mfu'
import { mfuReassemblyTimeoutMs, reliableMoqMfuReassembly } from '@blockcast/mmt-container/mfu-reassembly-policy'
import type { MoqtObject } from '../model/data/object'

export interface MmtpReceiverOptions {
  /** Upper bound on incomplete MFUs held at once (libmmt default when omitted). */
  maxBuffered?: number
  /**
   * Called for every object whose payload is not an acceptable MMTP packet.
   * The object is dropped and counted; the receiver keeps running.
   */
  onReject?: (error: unknown, object: MoqtObject) => void
}

export interface MmtpReceiverStats {
  /** Objects carrying a payload that were fed to the reassembler. */
  packets: number
  /** Status-only objects (end of group / end of track), which carry no MMTP. */
  statusObjects: number
  /** MFUs emitted. */
  mfus: number
  /** Payloads rejected as malformed or protocol-violating. */
  rejected: number
  /**
   * Incomplete MFUs discarded to stay within `maxBuffered`. Each one is lost
   * media that neither `rejected` nor `onReject` reports.
   */
  evicted: number
}

/**
 * Reassembles the MFUs of one MMTP track from its MoQ objects.
 *
 * MoQ subscriptions are reliable and ordered, so reassembly runs clock-free
 * (libmmt's `reliable-moq` policy): an incomplete MFU is never completed or
 * discarded by a timer, only by its own fragments or the `maxBuffered` bound.
 */
export class MmtpTrackReceiver {
  readonly #reassembler: MFUReassembler
  readonly #onReject: ((error: unknown, object: MoqtObject) => void) | undefined
  readonly #stats: Omit<MmtpReceiverStats, 'evicted'> = { packets: 0, statusObjects: 0, mfus: 0, rejected: 0 }
  // libmmt zeroes its counters on reset(); evictions before a reset live here.
  #evictedBeforeReset = 0

  constructor(options: MmtpReceiverOptions = {}) {
    this.#reassembler = new MFUReassembler({
      timeoutMs: mfuReassemblyTimeoutMs(reliableMoqMfuReassembly()),
      ...(options.maxBuffered !== undefined ? { maxBuffered: options.maxBuffered } : {}),
    })
    this.#onReject = options.onReject
  }

  get stats(): Readonly<MmtpReceiverStats> {
    return { ...this.#stats, evicted: this.#evictedBeforeReset + this.#reassembler.stats.evictedMemoryLimit }
  }

  /** Feed one object; returns the MFU it completed, if any. */
  push(object: MoqtObject): ReassembledMFU | null {
    // Only status objects (end of group / end of track) lack a payload.
    if (object.payload === null) {
      this.#stats.statusObjects++
      return null
    }
    this.#stats.packets++
    let mfu: ReassembledMFU | null
    try {
      mfu = processMMTPWithReassembly(object.payload, this.#reassembler)
    } catch (error) {
      this.#stats.rejected++
      this.#onReject?.(error, object)
      return null
    }
    if (mfu) this.#stats.mfus++
    return mfu
  }

  /**
   * A transform for the object stream of `MOQtailClient.subscribe()`:
   * `subscription.stream.pipeThrough(receiver.transform())`.
   */
  transform(): TransformStream<MoqtObject, ReassembledMFU> {
    return new TransformStream({
      transform: (object, controller) => {
        const mfu = this.push(object)
        if (mfu) controller.enqueue(mfu)
      },
    })
  }

  /** Drop all reassembly state, e.g. before re-subscribing. */
  reset(): void {
    this.#evictedBeforeReset += this.#reassembler.stats.evictedMemoryLimit
    this.#reassembler.reset()
  }
}
