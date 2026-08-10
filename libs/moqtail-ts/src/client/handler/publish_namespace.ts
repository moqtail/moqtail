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

import { PublishNamespace, RequestOk } from '../../model/control'
import { RequestStreamMessageHandler } from './handler'
import { logger } from '../../util/logger'

export const handlerPublishNamespace: RequestStreamMessageHandler<PublishNamespace> = async (client, msg, stream) => {
  logger.log('handler/publish_namespace', 'namespace', msg.trackNamespace.toUtf8Path())
  if (client.onNamespacePublished) {
    client.onNamespacePublished(msg)
  }
  await stream.send(new RequestOk(msg.requestId))
}
