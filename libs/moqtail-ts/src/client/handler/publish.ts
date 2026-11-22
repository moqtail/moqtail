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
import type { Publish } from '@/model/messages'
import type { RequestStreamMessageHandler } from './handler'

export const handlerPublish: RequestStreamMessageHandler<Publish> = async (client, msg, stream) => {
  // Registered before the push is accepted, since its data streams may follow at once
  const objects = client.acceptPushedTrack(msg)
  client.emit('peerpublish', msg, objects)

  // The publisher waits for this before opening any data stream. PUBLISH is answered by
  // REQUEST_OK: PUBLISH_OK is that message's name for this request type (§10.5).
  await stream.send(requestOk())
}
