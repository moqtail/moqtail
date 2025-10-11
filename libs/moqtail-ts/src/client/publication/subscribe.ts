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

import { FilterType, Subscribe, PublishDone, PublishDoneStatusCode, SubscribeUpdate } from '@/model/control'
import { MOQtailClient } from '../client'
import { Track } from '../track/track'
import {
  InternalError,
  Location,
  ReasonPhrase,
  SubgroupHeaderType,
  VersionSpecificParameter,
  VersionSpecificParameters,
} from '@/model'
import { SendStream } from '../data_stream'
import { SubgroupHeader } from '@/model/data/subgroup_header'
import { MoqtObject } from '@/model/data/object'
import { SimpleLock } from '../../util/simple_lock'
import { getTransportPriority } from '../util/priority'

/**
 * @public
 * Manages the publication of MOQT objects to a subscriber for a specific track.
 * Handles live object streaming, subscription lifecycle, and stream management.
 */
export class SubscribePublication {
  /**
   * The latest location that was published to the subscriber.
   */
  public latestLocation: Location | undefined

  /**
   * The alias for the track being published.
   */
  #trackAlias: bigint

  /**
   * The starting location for the subscription.
   */
  #startLocation: Location

  /**
   * The end group for AbsoluteRange subscriptions, if specified.
   */
  #endGroup: bigint | undefined

  /**
   * The priority of the subscriber.
   */
  #subscriberPriority: number

  /**
   * The priority of the publisher.
   */
  #publisherPriority: number

  /**
   * Whether objects should be forwarded to the subscriber.
   */
  #forward: boolean

  /**
   * The subscription parameters for this publication.
   */
  #subscribeParameters: VersionSpecificParameter[]

  /**
   * The number of streams opened for this subscription.
   */
  #streamsOpened: bigint = 0n

  /**
   * Function to cancel publishing, if set.
   */
  #cancelPublishing?: () => void

  /**
   * Whether publishing has started.
   */
  #isStarted = false

  /**
   * Whether publishing is completed.
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
   * Creates a new SubscribePublication instance.
   * @param client - The MOQT client managing the subscription.
   * @param track - The track being published.
   * @param subscribeMsg - The subscribe message containing subscription details.
   * @param largestLocation - The largest location seen so far, used for determining start location.
   */
  constructor(
    private readonly client: MOQtailClient,
    readonly track: Track,
    private readonly subscribeMsg: Subscribe,
    largestLocation?: Location,
  ) {
    this.#trackAlias = track.trackAlias!
    this.#publisherPriority = track.publisherPriority
    this.#subscriberPriority = subscribeMsg.subscriberPriority
    switch (subscribeMsg.filterType) {
      case FilterType.LatestObject:
        if (largestLocation) {
          this.#startLocation = new Location(largestLocation.group, largestLocation.object + 1n)
        } else {
          this.#startLocation = new Location(0n, 0n)
        }
        break
      case FilterType.NextGroupStart:
        if (largestLocation) {
          this.#startLocation = new Location(largestLocation.group + 1n, 0n)
        } else {
          this.#startLocation = new Location(0n, 0n)
        }
        break
      case FilterType.AbsoluteStart:
        this.#startLocation = subscribeMsg.startLocation!
        break
      case FilterType.AbsoluteRange:
        this.#startLocation = subscribeMsg.startLocation!
        this.#endGroup = subscribeMsg.endGroup
        break
    }
    this.#forward = subscribeMsg.forward
    this.#subscribeParameters = VersionSpecificParameters.fromKeyValuePairs(subscribeMsg.parameters)
    this.publish()
  }

  /**
   * Calculates the stream priority based on publisher and subscriber priorities.
   */
  get #streamPriority(): number {
    return 0.4 * getTransportPriority(this.#publisherPriority) + 0.6 * getTransportPriority(this.#subscriberPriority)
  }

  /**
   * Cancels the publication and cleans up resources.
   * Removes the publication from the client's publication map.
   */
  cancel(): void {
    if (this.#cancelPublishing) {
      this.#cancelPublishing()
      this.client.publications.delete(this.subscribeMsg.requestId)
    }
    this.#isCompleted = true
  }

  /**
   * Marks the publication as done and sends a PublishDone message to the client.
   * @param statusCode - The status code indicating why the publication ended.
   * @throws :{@link InternalError} If sending the message fails.
   */
  async done(statusCode: PublishDoneStatusCode): Promise<void> {
    this.#isCompleted = true
    const publishDone = new PublishDone(
      this.subscribeMsg.requestId,
      statusCode,
      BigInt(this.#streamsOpened),
      new ReasonPhrase('Subscription ended'),
    )
    // TODO: Handle track completion, there might be ongoing streams. Wait for all to finish before cleaning the state
    await this.client.controlStream.send(publishDone)
  }

  /**
   * Updates the subscription parameters and locations based on a SubscribeUpdate message.
   * @param msg - The update message containing new subscription details.
   */
  update(msg: SubscribeUpdate): void {
    // TODO: Control checks on update rules e.g only narrowing, end>start either here or in update handler
    this.#startLocation = msg.startLocation
    this.#endGroup = msg.endGroup
    this.#subscriberPriority = msg.subscriberPriority
    this.#forward = msg.forward
    this.#subscribeParameters = VersionSpecificParameters.fromKeyValuePairs(msg.parameters)
  }

  /**
   * Publishes MOQT objects to the subscriber as they become available.
   * Handles stream creation, object writing, and stream closure based on subscription parameters.
   * @throws :{@link InternalError} If the track does not support live content.
   */
  async publish(): Promise<void> {
    if (!this.track.trackSource.live)
      throw new InternalError('SubscribePublication.publish', 'Track does not support live content')

    // TODO: HybridContent is also allowed
    this.track.trackSource.live.onDone(() => {
      this.done(PublishDoneStatusCode.TrackEnded)
    })

    this.#cancelPublishing = this.track.trackSource.live.onNewObject(async (obj: MoqtObject) => {
      if (this.#isCompleted) return
      if (!this.#forward) return
      if (!this.#isStarted && this.#startLocation.compare(obj.location) <= 0) {
        this.#isStarted = true
      }
      if (this.#isStarted) {
        try {
          if (!this.#streams.has(obj.location.group)) {
            await this.#lock.acquire()
            if (!this.#streams.has(obj.location.group)) {
              // New group or first object
              const writeStream = await this.client.webTransport.createUnidirectionalStream({
                sendOrder: this.#streamPriority,
              })
              let subgroupId: bigint | undefined
              if (SubgroupHeaderType.hasExplicitSubgroupId(obj.subgroupHeaderType)) subgroupId = obj.subgroupId!
              const header = new SubgroupHeader(
                obj.subgroupHeaderType,
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

          // If this is the last object in the group (for AbsoluteRange/endGroup), close the stream
          if (this.#endGroup && obj.location.group === this.#endGroup) {
            try {
              await this.#lock.acquire()
              if (this.#streams.has(obj.location.group)) {
                await sendStream.close()
                this.#streams.delete(obj.location.group)
              }
              await this.#lock.release()
            } catch (err) {
              console.warn('error in closing stream: id, endGroup, err', this.#id, this.#endGroup, err)
            }
            await this.done(PublishDoneStatusCode.SubscriptionEnded)
            this.cancel()
          } else if (this.latestLocation && this.latestLocation.group !== obj.location.group) {
            // TODO: Maybe don't close the previous stream, discuss
            // If group changed, close previous group's stream
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
      }
    })
  }
}
