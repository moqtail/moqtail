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

import { requestOk } from '@/model/messages'
import type { TrackStatus } from '@/model/messages'
import type { RequestStreamMessageHandler } from './handler'

export const handlerTrackStatus: RequestStreamMessageHandler<TrackStatus> = async (_client, _msg, stream) => {
  // TODO (#273): report the real track status. REQUEST_OK carries the Track Properties
  // that would hold it, but nothing populates them yet, so this stays a bare acknowledgement.
  await stream.send(requestOk())
}
