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

import {
  Fetch,
  FetchHeader,
  FetchHeaderType,
  FetchType,
  InternalError,
  Location,
  MOQtailError,
  MoqtObject,
} from '@/model'
import { FetchObjectContext } from '@/model/data/fetch_object'
import { MOQtailClient } from '../client'
import { Track } from '../track/track'
import { SubscribePublication } from './subscribe'
import { PublishPublication } from './publish'
import { logger } from '../../util/logger'

// TODO: Use group order
// TODO: Use fetch parameters
export class FetchPublication {
  readonly #requestId: bigint
  readonly #track: Track
  readonly #startLocation: Location
  readonly #endLocation: Location
  readonly #msg: Fetch
  readonly #client: MOQtailClient
  #stream: WritableStream | undefined
  #writer: WritableStreamDefaultWriter | undefined
  #objects: MoqtObject[] | undefined
  #isCanceled = false

  constructor(client: MOQtailClient, track: Track, fetchRequest: Fetch) {
    this.#client = client
    this.#requestId = fetchRequest.requestId
    this.#track = track
    this.#msg = fetchRequest
    let joiningRequest: SubscribePublication | PublishPublication | FetchPublication | undefined
    switch (this.#msg.typeAndProps.type) {
      case FetchType.Standalone:
        // TODO: Tie up fetch type and relevant props as {type: 1, props: standAlone} | {type: 2, props: joining} | {type: 3, props: joining}
        this.#startLocation = this.#msg.typeAndProps.props.startLocation
        this.#endLocation = this.#msg.typeAndProps.props.endLocation
        break

      case FetchType.Relative:
        joiningRequest = client.publications.get(this.#msg.typeAndProps.props.joiningRequestId)
        if (!(joiningRequest instanceof SubscribePublication))
          throw new InternalError('FetchPublication.constructor', 'No subscription for the joining request id')
        if (!joiningRequest.latestLocation)
          throw new InternalError('FetchPublication.constructor', 'joiningRequest.largestLocation does not exist')
        this.#startLocation = new Location(
          joiningRequest.latestLocation.group - this.#msg.typeAndProps.props.joiningStart,
          0n,
        )
        this.#endLocation = joiningRequest.latestLocation
        break

      case FetchType.Absolute:
        joiningRequest = client.publications.get(this.#msg.typeAndProps.props.joiningRequestId)
        if (!(joiningRequest instanceof SubscribePublication))
          throw new InternalError('FetchPublication.constructor', 'No subscription for the joining request id')
        if (!joiningRequest.latestLocation)
          throw new InternalError('FetchPublication.constructor', 'joiningRequest.largestLocation does not exist')
        this.#startLocation = new Location(this.#msg.typeAndProps.props.joiningStart, 0n)
        this.#endLocation = joiningRequest.latestLocation
        break
    }
    logger.debug('publication/fetch', `created requestId=${this.#requestId} type=${this.#msg.typeAndProps.type}`)
    this.publish()
  }

  cancel() {
    logger.debug('publication/fetch', `canceled requestId=${this.#requestId}`)
    this.#isCanceled = true
  }

  async publish(): Promise<void> {
    if (this.#isCanceled) return
    if (!this.#track.trackSource.past) throw new MOQtailError('FetchPublication.publish, Track does not support fetch')
    logger.debug(
      'publication/fetch',
      `publishing requestId=${this.#requestId} start=${this.#startLocation} end=${this.#endLocation}`,
    )
    try {
      this.#objects = await this.#track.trackSource.past.getRange(this.#startLocation, this.#endLocation)
      // TODO: Calculate and use stream priority from subscriber priority from the msg + publisher priority from the track
      this.#stream = await this.#client.webTransport.createUnidirectionalStream()
      this.#writer = this.#stream.getWriter()
      const header = new FetchHeader(FetchHeaderType.Type0x05, this.#requestId)
      await this.#writer.write(header)
      let fetchPrevCtx: FetchObjectContext | undefined = undefined
      for (const obj of this.#objects) {
        if (this.#isCanceled) {
          logger.debug('publication/fetch', `aborted during publish requestId=${this.#requestId}`)
          await this.#writer.abort('Fetch cancelled during publish')
          this.#client.publications.delete(this.#requestId)
          return
        }
        const fetchObj = obj.tryIntoFetchObject()
        await this.#writer.write(fetchObj.serialize(fetchPrevCtx).toUint8Array())
        const newCtx = fetchObj.toContext()
        if (newCtx) fetchPrevCtx = newCtx
      }
      await this.#writer.close()
      logger.debug('publication/fetch', `published requestId=${this.#requestId} objects=${this.#objects.length}`)
      this.#client.publications.delete(this.#requestId)
    } catch (error: unknown) {
      await this.#writer?.abort('Fetch failed during publish')
      const message = error instanceof Error ? error.message : String(error)
      logger.error('publication/fetch', `publish failed requestId=${this.#requestId}`, message)
      throw new InternalError('FetchPublication.publish', `Failed to publish: ${message}`)
    }
  }
}
