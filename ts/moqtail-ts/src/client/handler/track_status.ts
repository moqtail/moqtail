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

import { RequestOk, TrackStatus } from '../../model/control'
import { RequestStreamMessageHandler } from './handler'
import { logger } from '../../util/logger'

export const handlerTrackStatus: RequestStreamMessageHandler<TrackStatus> = async (_client, msg, stream) => {
  logger.debug('handler/track_status', `requestId=${msg.requestId} ftn="${msg.fullTrackName}"`)
  // TODO (#273): report the real track status. REQUEST_OK now carries the Track
  // Properties that would hold it, but nothing populates them yet, so this stays a bare
  // acknowledgement.
  await stream.send(new RequestOk())
}
