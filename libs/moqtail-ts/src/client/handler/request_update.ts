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

import { ReasonPhrase } from '@/model'
import {
  ControlMessageType,
  PublishDone,
  PublishDoneStatusCode,
  RequestError,
  RequestErrorCode,
  RequestOk,
  RequestUpdate,
} from '../../model/control'
import { StreamResetCode } from '../../model/error/stream_reset'
import { ProtocolViolationError } from '../../model/error/error'
import { RequestStreamMessageHandler } from './handler'
import { RequestStream } from '../request_stream'
import { MOQtailClient } from '../client'
import { FetchPublication } from '../publication/fetch'
import { PublishPublication } from '../publication/publish'
import { SubscribePublication } from '../publication/subscribe'
import { logger } from '../../util/logger'
import { rangeFilterRejection } from '../util/validators'
import { InvalidFilterError } from '../../model/parameter/message/range_filter'

/**
 * A REQUEST_UPDATE modifies the request whose stream it arrived on, and §10.9 requires
 * exactly one REQUEST_OK or REQUEST_ERROR back on that stream. Only the sender of a
 * request may update it, plus a subscriber on a PUBLISH it was offered. An update
 * arriving anywhere else, a TRACK_STATUS among them, closes the session.
 */
export const handlerRequestUpdate: RequestStreamMessageHandler<RequestUpdate> = async (
  client,
  msg,
  stream,
  requestId,
) => {
  logger.log('handler/request_update', 'requestId', requestId)

  if (!isExpectedUpdate(stream)) {
    await client.disconnect(
      new ProtocolViolationError(
        'handler/request_update',
        `Unexpected REQUEST_UPDATE on a ${stream.first ? 'locally' : 'peer'} opened ${describe(stream.openingType)} stream`,
      ),
    )
    return
  }

  const filterRejection = rangeFilterRejection(msg.parameters, client.maxFilterRanges)
  if (filterRejection) {
    const reason = filterRejection.reasonPhrase.phrase
    logger.warn('handler/request_update', `requestId=${requestId} has filters this client cannot take: ${reason}`)
    await failUpdate(client, stream, requestId, RequestErrorCode.InvalidFilter, reason)
    return
  }

  // A subscription, or a PUBLISH this side is making. The update moves its filters
  // and Forward State, which for a PUBLISH parks or resumes the push. A publication
  // checks the filters it would be left with before taking any of it.
  const publication = client.publications.get(requestId)
  if (publication instanceof SubscribePublication || publication instanceof PublishPublication) {
    try {
      publication.update(msg)
    } catch (error) {
      const invalid = error instanceof InvalidFilterError
      logger.error('handler/request_update', `update failed for requestId=${requestId}`, error)
      await failUpdate(
        client,
        stream,
        requestId,
        invalid ? RequestErrorCode.InvalidFilter : RequestErrorCode.InternalError,
        invalid ? error.details : 'REQUEST_UPDATE failed',
      )
      return
    }
    await stream.send(new RequestOk())
    return
  }

  // A FETCH, a PUBLISH-established subscription and a namespace request all lack the
  // state to update here, so the update fails and takes the request with it.
  logger.warn('handler/request_update', `no updatable request for requestId=${requestId}`)
  await failUpdate(client, stream, requestId, RequestErrorCode.NotSupported, 'REQUEST_UPDATE cannot be applied')
}

/**
 * Whether the peer may send a REQUEST_UPDATE on this stream. On a stream the peer
 * opened, it is the requester and may update any of the updatable request types. On a
 * stream this side opened, it may only update a PUBLISH, as that PUBLISH's subscriber.
 */
function isExpectedUpdate(stream: RequestStream): boolean {
  const openingType = stream.openingType
  if (openingType === undefined) return false
  if (stream.first) return openingType === ControlMessageType.Publish
  return ControlMessageType.isUpdatable(openingType)
}

function describe(openingType: ControlMessageType | undefined): string {
  return openingType === undefined ? 'unknown' : ControlMessageType[openingType]
}

/**
 * Refuses the update, then ends the request the way §10.9.1 requires for its type: a
 * subscription is terminated with PUBLISH_DONE(UPDATE_FAILED), a FETCH has its data
 * stream reset, and a namespace request has its bidi stream closed.
 */
async function failUpdate(
  client: MOQtailClient,
  stream: RequestStream,
  requestId: bigint,
  errorCode: RequestErrorCode,
  reason: string,
): Promise<void> {
  await stream.send(new RequestError(errorCode, 0n, new ReasonPhrase(reason)))

  const publication = client.publications.get(requestId)
  switch (stream.openingType) {
    case ControlMessageType.Subscribe:
    case ControlMessageType.Publish:
      if (publication instanceof SubscribePublication) {
        await publication.done(PublishDoneStatusCode.UpdateFailed)
      } else {
        const streamCount = publication instanceof PublishPublication ? publication.streamsOpened : 0n
        await stream.send(
          new PublishDone(PublishDoneStatusCode.UpdateFailed, streamCount, new ReasonPhrase('REQUEST_UPDATE failed')),
        )
      }
      publication?.cancel()
      client.publications.delete(requestId)
      break

    case ControlMessageType.Fetch:
      if (publication instanceof FetchPublication) await publication.resetDataStream(StreamResetCode.Cancelled)
      client.publications.delete(requestId)
      break

    case ControlMessageType.PublishNamespace:
    case ControlMessageType.SubscribeNamespace:
    case ControlMessageType.SubscribeTracks:
      await stream.close()
      break
  }
}
