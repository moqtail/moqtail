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
import { logger } from '../../util/logger'

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
   * Highest group this publication has already opened a stream for or skipped objects of.
   * A stream for a group at or below it reopens a subgroup, so it cannot claim FIRST_OBJECT.
   */
  #highestGroupSeen: bigint | undefined

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
   * How many data streams this publication has opened, which is what a PUBLISH_DONE
   * reports to the subscriber.
   */
  get streamsOpened(): bigint {
    return this.#streamsOpened
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
        stream
          .close()
          .catch((e) => logger.warn('publication/publish', `Failed to close stream for group ${groupId}:`, e))
      }
      this.#streams.clear()
      this.#lock.release()
    })
  }

  #markGroupSeen(group: bigint): void {
    if (this.#highestGroupSeen === undefined || group > this.#highestGroupSeen) {
      this.#highestGroupSeen = group
    }
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

    // Objects produced before publishing started are never sent, so the group they belong to
    // cannot claim to start at its first object.
    this.#highestGroupSeen = this.track.trackSource.live.largestLocation?.group

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

            const firstObject = this.#highestGroupSeen === undefined || obj.location.group > this.#highestGroupSeen
            const headerType = obj.getSubgroupHeaderType(true, false, firstObject)

            let subgroupId: bigint | undefined
            if (SubgroupHeaderType.hasExplicitSubgroupId(headerType)) subgroupId = obj.subgroupId!

            const header = new SubgroupHeader(
              headerType,
              this.#trackAlias,
              obj.location.group,
              subgroupId,
              this.#publisherPriority,
            )

            const sendStream = await SendStream.new(writeStream, header)
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
                logger.warn('publication/publish', 'error in closing stream', prevGroup, err)
              }
              this.#streams.delete(prevGroup)
            }
            await this.#lock.release()
          } catch (err) {
            logger.warn(
              'publication/publish',
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

if (import.meta.vitest) {
  const { describe, test, expect, vi } = import.meta.vitest
  const { FullTrackName, ObjectForwardingPreference } = await import('@/model')
  const { LiveTrackSource } = await import('../track/content_source')
  const { SubgroupHeader: Header } = await import('@/model/data/subgroup_header')
  const { FrozenByteBuffer } = await import('@/model/common/byte_buffer')

  describe('PublishPublication FIRST_OBJECT', () => {
    const ftn = FullTrackName.tryNew('room/alice', 'video')

    /** Header types of the subgroup streams the publication opened, in order. */
    function publishing(): { headerTypes: number[]; push: (group: number, object: number) => void; done: () => void } {
      const headerTypes: number[] = []
      const webTransport = {
        createUnidirectionalStream: async () => {
          let first = true
          return new WritableStream<Uint8Array>({
            write: (chunk) => {
              if (!first) return
              first = false
              headerTypes.push(Number(Header.deserialize(new FrozenByteBuffer(chunk)).type))
            },
          })
        },
      }

      let source!: ReadableStreamDefaultController<MoqtObject>
      const live = new LiveTrackSource(new ReadableStream<MoqtObject>({ start: (controller) => (source = controller) }))
      const track: Track = {
        fullTrackName: ftn,
        trackSource: { live },
        publisherPriority: 0,
        trackAlias: 1n,
      }
      const client = { webTransport, publications: new Map() } as unknown as MOQtailClient
      new PublishPublication(client, track, { requestId: 0n } as unknown as Publish)

      const push = (group: number, object: number) =>
        source.enqueue(
          MoqtObject.newWithPayload(
            ftn,
            new Location(BigInt(group), BigInt(object)),
            0,
            ObjectForwardingPreference.Subgroup,
            0n,
            null,
            new Uint8Array([1]),
          ),
        )
      return { headerTypes, push, done: () => source.close() }
    }

    test('sets the bit on each new subgroup', async () => {
      const { headerTypes, push } = publishing()
      push(0, 0)
      push(1, 0)
      await vi.waitFor(() => expect(headerTypes).toHaveLength(2))
      expect(headerTypes.map(SubgroupHeaderType.isFirstObject)).toEqual([true, true])
    })

    test('leaves the bit clear when a closed subgroup is reopened', async () => {
      const { headerTypes, push } = publishing()
      push(0, 0)
      push(1, 0)
      await vi.waitFor(() => expect(headerTypes).toHaveLength(2))
      // Group 0's stream was closed when group 1 arrived; a later group 0 object reopens it.
      push(0, 1)
      await vi.waitFor(() => expect(headerTypes).toHaveLength(3))
      expect(headerTypes.map(SubgroupHeaderType.isFirstObject)).toEqual([true, true, false])
    })

    test('leaves the bit clear for a group already in progress when publishing starts', async () => {
      const headerTypes: number[] = []
      const webTransport = {
        createUnidirectionalStream: async () => {
          let first = true
          return new WritableStream<Uint8Array>({
            write: (chunk) => {
              if (!first) return
              first = false
              headerTypes.push(Number(Header.deserialize(new FrozenByteBuffer(chunk)).type))
            },
          })
        },
      }
      let source!: ReadableStreamDefaultController<MoqtObject>
      const live = new LiveTrackSource(new ReadableStream<MoqtObject>({ start: (controller) => (source = controller) }))
      const object = (group: number, objectId: number) =>
        MoqtObject.newWithPayload(
          ftn,
          new Location(BigInt(group), BigInt(objectId)),
          0,
          ObjectForwardingPreference.Subgroup,
          0n,
          null,
          new Uint8Array([1]),
        )

      // Group 3 is already flowing before anyone publishes it, so its first object is gone.
      source.enqueue(object(3, 0))
      await vi.waitFor(() => expect(live.largestLocation?.group).toBe(3n))

      const client = { webTransport, publications: new Map() } as unknown as MOQtailClient
      new PublishPublication(
        client,
        { fullTrackName: ftn, trackSource: { live }, publisherPriority: 0, trackAlias: 1n },
        {
          requestId: 0n,
        } as unknown as Publish,
      )

      source.enqueue(object(3, 1))
      source.enqueue(object(4, 0))
      await vi.waitFor(() => expect(headerTypes).toHaveLength(2))
      expect(headerTypes.map(SubgroupHeaderType.isFirstObject)).toEqual([false, true])
    })
  })
}
