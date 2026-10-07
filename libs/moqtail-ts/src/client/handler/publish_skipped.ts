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
import { PublishSkipped, SubscribeTracks } from '../../model/control'
import { RequestStreamMessageHandler } from './handler'
import { logger } from '../../util/logger'

export const handlerPublishSkipped: RequestStreamMessageHandler<PublishSkipped> = async (client, msg, stream) => {
  const first = stream.first
  if (!(first instanceof SubscribeTracks)) {
    throw new ProtocolViolationError(
      'handlerPublishSkipped',
      'PUBLISH_SKIPPED on a stream this side did not open with SUBSCRIBE_TRACKS',
    )
  }

  logger.log(
    'handler/publish_skipped',
    'prefix',
    first.trackNamespacePrefix.toUtf8Path(),
    'suffix',
    msg.trackNamespaceSuffix.toUtf8Path(),
  )

  client.onPeerPublishSkipped?.(first.trackNamespacePrefix, msg)
}
