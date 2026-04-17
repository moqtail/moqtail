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

import { PublishNamespaceCancel } from '../../model/control'
import { ControlMessageHandler } from './handler'
import { createLogger } from '../../util/logger'

const logger = createLogger('handler/publish_namespace_cancel')

export const handlerPublishNamespaceCancel: ControlMessageHandler<PublishNamespaceCancel> = async (_client, msg) => {
  logger.debug('not implemented', msg)
  // TODO: Implement PublishNamespaceCancel handler logic
}
