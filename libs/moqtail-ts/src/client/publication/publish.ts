/**
 * Copyright 2025 The MOQtail Authors
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 * http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

import { Publish } from '@/model/control'
import { MOQtailClient } from '../client'
import { Track } from '../track/track'
import { InternalError, Location, SubgroupHeaderType } from '@/model'
import { SendStream } from '../data_stream'
import { SubgroupHeader } from '@/model/data/subgroup_header'
import { MoqtObject } from '@/model/data/object'
import { SimpleLock } from '../../util/simple_lock'
import { getTransportPriority } from '../util/priority'

/**
 * @public
 * Manages the proactive publication of MOQT objects from a publisher to the relay.
 * Handles live object streaming and stream management.
 */
export class PublishPublication {
  /**
   * The latest location that was published to the relay.
   */
  public latestLocation: Location | undefined

  /**
   * The alias for the track being published.
   */
  #trackAlias: bigint

  /**
   * The priority of the publisher.
   */
  #publisherPriority: number

  /**
   * The number of streams opened for this publication.
   */
  #streamsOpened: bigint = 0n

  /**
   * Function to cancel publishing, if set.
   */
  #cancelPublishing?: () => void

  /**
   * Whether publishing is completed/cancelled.
   */
  #isCompleted = false

  /**
   * Lock for synchronizing stream operations.
   */
  #lock: SimpleLock = new SimpleLock()

  /**
   * Map of group IDs to their corresponding send streams.
   */
  #streams: Map<bigint, SendStream> = new Map()

  /**
   * Unique identifier for this publication instance.
   */
  #id = Math.floor(Math.random() * 1000000)

  /**
   * Creates a new PublishPublication instance.
   * @param client - The MOQT client managing the connection.
   * @param track - The track being proactively published.
   * @param publishMsg - The publish message that initiated this session.
   */
  constructor(
    private readonly client: MOQtailClient,
    readonly track: Track,
    private readonly publishMsg: Publish,
  ) {
    this.#trackAlias = track.trackAlias!
    this.#publisherPriority = track.publisherPriority

    // Start pushing data immediately
    this.publishToRelay()
  }

  /**
   * Calculates the stream priority based on publisher priority.
   * (Since this is a proactive push, there is no subscriber priority to average with).
   */
  get #streamPriority(): number {
    return getTransportPriority(this.#publisherPriority)
  }

  /**
   * Cancels the publication and cleans up resources.
   * Removes the publication from the client's publication map.
   */
  cancel(): void {
    if (this.#cancelPublishing) {
      this.#cancelPublishing()
      this.client.publications.delete(this.publishMsg.requestId)
    }
    this.#isCompleted = true

    // Attempt to cleanly close any open streams
    this.#lock.acquire().then(() => {
      for (const [groupId, stream] of this.#streams.entries()) {
        stream.close().catch((e) => console.warn(`Failed to close stream for group ${groupId}:`, e))
      }
      this.#streams.clear()
      this.#lock.release()
    })
  }

  /**
   * Publishes MOQT objects to the relay as they become available.
   * Handles stream creation, object writing, and stream closure.
   * @throws :{@link InternalError} If the track does not support live content.
   */
  async publishToRelay(): Promise<void> {
    if (!this.track.trackSource.live)
      throw new InternalError('PublishPublication.publishToRelay', 'Track does not support live content')

    this.track.trackSource.live.onDone(() => {
      this.cancel()
    })

    this.#cancelPublishing = this.track.trackSource.live.onNewObject(async (obj: MoqtObject) => {
      if (this.#isCompleted) return

      try {
        if (!this.#streams.has(obj.location.group)) {
          await this.#lock.acquire()
          // Double-check after acquiring lock
          if (!this.#streams.has(obj.location.group)) {
            // New group requires a new Unidirectional stream
            const writeStream = await this.client.webTransport.createUnidirectionalStream({
              sendOrder: this.#streamPriority,
            })

            let subgroupId: bigint | undefined
            if (SubgroupHeaderType.hasExplicitSubgroupId(obj.getSubgroupHeaderType(true))) subgroupId = obj.subgroupId!

            const header = new SubgroupHeader(
              obj.getSubgroupHeaderType(true),
              this.#trackAlias,
              obj.location.group,
              subgroupId,
              this.#publisherPriority,
            )

            const sendStream = await SendStream.new(writeStream, header)
            this.#streams.set(obj.location.group, sendStream)
            this.#streamsOpened++
          }
          await this.#lock.release()
        }

        const sendStream = this.#streams.get(obj.location.group)!
        await this.#lock.acquire()
        await sendStream.write(obj.tryIntoSubgroupObject())
        await this.#lock.release()

        // Close previous group's stream if the group ID has incremented
        if (this.latestLocation && this.latestLocation.group !== obj.location.group) {
          const prevGroup = this.latestLocation.group
          try {
            await this.#lock.acquire()
            const prevStream = this.#streams.get(prevGroup)
            if (prevStream) {
              try {
                await prevStream.close()
              } catch (err) {
                console.warn('error in closing stream', prevGroup, err)
              }
              this.#streams.delete(prevGroup)
            }
            await this.#lock.release()
          } catch (err) {
            console.warn(
              'error in closing stream: id, latestLocation.group, err',
              this.#id,
              this.latestLocation.group,
              err,
            )
          }
        }

        await this.#lock.acquire()
        this.latestLocation = obj.location
        await this.#lock.release()
      } catch (err) {
        this.cancel()
        throw err
      }
    })
  }
}
