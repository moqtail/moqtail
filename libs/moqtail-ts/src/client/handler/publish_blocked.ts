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

import { ProtocolViolationError } from '@/model/error'
import { PublishBlocked, SubscribeTracks } from '../../model/control'
import { RequestStreamMessageHandler } from './handler'
import { logger } from '../../util/logger'

export const handlerPublishBlocked: RequestStreamMessageHandler<PublishBlocked> = async (client, msg, stream) => {
  // §10.20: PUBLISH_BLOCKED answers a SUBSCRIBE_TRACKS, and its suffix is only
  // meaningful next to the prefix that stream was opened with.
  const first = stream.first
  if (!(first instanceof SubscribeTracks)) {
    throw new ProtocolViolationError(
      'handlerPublishBlocked',
      'PUBLISH_BLOCKED on a stream this side did not open with SUBSCRIBE_TRACKS',
    )
  }

  logger.log(
    'handler/publish_blocked',
    'prefix',
    first.trackNamespacePrefix.toUtf8Path(),
    'suffix',
    msg.trackNamespaceSuffix.toUtf8Path(),
  )

  client.onPeerPublishBlocked?.(first.trackNamespacePrefix, msg)
}
