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

import type { MOQtailClient } from '@/client/client'
import type { Track } from '@/client/track/track'
import { Location } from '@/model/common/location'
import { PublishDoneStatusCode } from '@/model/constants'
import { InternalError } from '@/model/error/error'
import type { RequestUpdate, Subscribe } from '@/model/messages'
import { applyParameterUpdate, endLocation, startLocation } from '@/model/parameters'
import type { MessageParameters } from '@/model/parameters'
import { CombinedFilter } from './filter'
import { SendStream } from '@/client/data_stream'
import { RequestStream } from '@/client/request_stream'
import { MoqtObject } from '@/model/data/object'
import { SubgroupIds, subgroupHeaderFor } from './subgroup'
import { assertWithinFilterRangeLimit } from '@/client/util/validators'
import { SimpleLock } from '@/util/simple_lock'
import { getTransportPriority } from '@/client/util/priority'
import { logger } from '@/util/logger'

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
   * The last Location delivered, inclusive, when the Location Filter has an end.
   */
  #endLocation: Location | undefined

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
  #parameters: MessageParameters

  /**
   * What the subscription's Range Filters let through.
   */
  #filter: CombinedFilter

  /**
   * The Subgroup ID of each Object, which the filter and the stream header need.
   */
  #subgroupIds = new SubgroupIds()

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
   * Highest group this publication has already opened a stream for or skipped objects of.
   * A stream for a group at or below it either reopens a subgroup or starts past the
   * subgroup's first object, so it cannot claim FIRST_OBJECT.
   */
  #highestGroupSeen: bigint | undefined

  /**
   * Creates a new SubscribePublication instance.
   * @param client - The MOQT client managing the subscription.
   * @param track - The track being published.
   * @param subscribeMsg - The subscribe message containing subscription details.
   * @param requestStream - The bidi request stream the SUBSCRIBE arrived on. Every
   * message this publication sends back to the subscriber goes on it.
   * @param largestLocation - The largest location seen so far, used for determining start location.
   */
  constructor(
    private readonly client: MOQtailClient,
    readonly track: Track,
    private readonly subscribeMsg: Subscribe,
    private readonly requestStream: RequestStream,
    largestLocation?: Location,
  ) {
    this.#trackAlias = track.trackAlias!
    this.#publisherPriority = track.publisherPriority

    const { subscriberPriority, forward, locationFilter } = subscribeMsg.parameters
    this.#subscriberPriority = subscriberPriority ?? 128
    this.#forward = forward ?? true
    this.#parameters = subscribeMsg.parameters
    // The handler has already refused a SUBSCRIBE with filters that cannot be applied.
    this.#filter = new CombinedFilter(this.#parameters)
    // No filter starts at the beginning and never ends.
    this.#startLocation = (locationFilter && startLocation(locationFilter, largestLocation)) ?? new Location(0n, 0n)
    this.#endLocation = locationFilter && endLocation(locationFilter)
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

    const streamCount = BigInt(this.#streamsOpened)
    // TODO: Handle track completion, there might be ongoing streams. Wait for all to finish before cleaning the state
    await this.requestStream.send({ kind: 'publishDone', statusCode, streamCount, errorReason: 'Subscription ended' })
  }

  /**
   * Updates the subscription parameters and locations based on a RequestUpdate message.
   * @param msg - The update message containing new request details.
   * @throws :`InvalidFilterError` If the Range Filters the subscription would be
   * left with cannot be applied; nothing is updated.
   */
  update(msg: RequestUpdate): void {
    // The filters are checked as they would stand afterwards, before any of the update
    // is taken: a refused update leaves the subscription as it was.
    const parameters = applyParameterUpdate(this.#parameters, msg.parameters)

    assertWithinFilterRangeLimit(parameters, this.client.maxFilterRanges, 'REQUEST_UPDATE')
    this.#filter = new CombinedFilter(parameters)

    // An update changes the filter only when it carries one.
    const { locationFilter, subscriberPriority, forward } = msg.parameters

    if (locationFilter) {
      this.#startLocation = startLocation(locationFilter, this.latestLocation) ?? new Location(0n, 0n)
      this.#endLocation = endLocation(locationFilter)
    }

    if (subscriberPriority !== undefined) this.#subscriberPriority = subscriberPriority
    if (forward !== undefined) this.#forward = forward
    this.#parameters = parameters
  }

  /**
   * Closes the stream carrying `group`, if one is open.
   */
  async #closeGroupStream(group: bigint): Promise<void> {
    await this.#lock.acquire()
    try {
      const stream = this.#streams.get(group)
      if (stream) {
        this.#streams.delete(group)
        await stream.close()
      }
    } catch (err) {
      logger.warn('publication/subscribe', 'error in closing stream: id, group, err', this.#id, group, err)
    } finally {
      await this.#lock.release()
    }
  }

  #markGroupSeen(group: bigint): void {
    if (this.#highestGroupSeen === undefined || group > this.#highestGroupSeen) {
      this.#highestGroupSeen = group
    }
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

    // Objects produced before the subscription started are never forwarded, so the group they
    // belong to cannot claim to start at its first object.
    this.#highestGroupSeen = this.track.trackSource.live.largestLocation?.group

    this.#cancelPublishing = this.track.trackSource.live.onNewObject(async (obj: MoqtObject) => {
      if (this.#isCompleted) return
      this.#subgroupIds.note(obj)
      if (!this.#forward) {
        this.#markGroupSeen(obj.location.group)
        return
      }
      if (!this.#isStarted && this.#startLocation.compare(obj.location) <= 0) {
        this.#isStarted = true
      }
      if (!this.#isStarted) this.#markGroupSeen(obj.location.group)
      // Past the end of the filter nothing more is delivered, but the subscription stays:
      // a REQUEST_UPDATE can still move the filter.
      if (this.#isStarted && this.#endLocation && obj.location.compare(this.#endLocation) > 0) {
        this.#markGroupSeen(obj.location.group)
        if (this.latestLocation) await this.#closeGroupStream(this.latestLocation.group)
        return
      }
      if (!this.#isStarted) return
      const subgroupId = this.#subgroupIds.of(obj)
      if (!this.#filter.matches(obj, subgroupId)) {
        // Marked seen, so a stream opened for this group later does not claim its first
        // Object. A group that has moved on still ends the stream of the one before.
        this.#markGroupSeen(obj.location.group)
        if (this.latestLocation && this.latestLocation.group !== obj.location.group) {
          await this.#closeGroupStream(this.latestLocation.group)
        }
        return
      }
      try {
        if (!this.#streams.has(obj.location.group)) {
          await this.#lock.acquire()
          if (!this.#streams.has(obj.location.group)) {
            // New group or first object
            const writeStream = await this.client.webTransport.createUnidirectionalStream({
              sendOrder: this.#streamPriority,
            })
            const firstObject = this.#highestGroupSeen === undefined || obj.location.group > this.#highestGroupSeen
            const header = subgroupHeaderFor(
              obj,
              subgroupId ?? obj.location.object,
              this.#trackAlias,
              this.#publisherPriority,
              firstObject,
            )
            const sendStream = await SendStream.new(
              writeStream,
              header,
              this.client,
              undefined,
              () => this.track.fullTrackName,
            )
            this.#streams.set(obj.location.group, sendStream)
            this.#streamsOpened++
            this.#markGroupSeen(obj.location.group)
          }
          await this.#lock.release()
        }

        const sendStream = this.#streams.get(obj.location.group)!
        await this.#lock.acquire()
        await sendStream.write(obj.tryIntoSubgroupObject())
        await this.#lock.release()

        if (this.#endLocation && obj.location.equals(this.#endLocation)) {
          // The filter's last Object: nothing follows on this stream.
          await this.#closeGroupStream(obj.location.group)
        } else if (this.latestLocation && this.latestLocation.group !== obj.location.group) {
          // TODO: Maybe don't close the previous stream, discuss
          // If group changed, close previous group's stream
          await this.#closeGroupStream(this.latestLocation.group)
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
