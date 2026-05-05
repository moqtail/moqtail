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

import { ProtocolViolationError } from '@/model/error'
import { FetchCancel } from '../../model/control'
import { FetchPublication } from '../publication/fetch'
import { ControlMessageHandler } from './handler'
import { logger } from '../../util/logger'

export const handlerFetchCancel: ControlMessageHandler<FetchCancel> = async (client, msg) => {
  logger.log('handler/fetch_cancel', 'requestId', msg.requestId)
  const publication = client.publications.get(msg.requestId)
  if (publication instanceof FetchPublication) {
    publication.cancel()
  } else {
    throw new ProtocolViolationError('handlerFetchCancel', 'No fetch request found for the given request id')
  }
}
