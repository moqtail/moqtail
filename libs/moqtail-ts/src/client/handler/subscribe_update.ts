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

import { ProtocolViolationError } from '@/model'
import { SubscribeUpdate } from '../../model/control'
import { ControlMessageHandler } from './handler'
import { SubscribePublication } from '../publication/subscribe'
import { createLogger } from '../../util/logger'

const logger = createLogger('handler/subscribe_update')

export const handlerSubscribeUpdate: ControlMessageHandler<SubscribeUpdate> = async (client, msg) => {
  logger.log('requestId', msg.requestId)
  const publication = client.publications.get(msg.requestId)
  if (publication instanceof SubscribePublication) {
    publication.update(msg)
  } else {
    throw new ProtocolViolationError('handlerSubscribeUpdate', 'No subscribe request found for the given request id')
  }
}
