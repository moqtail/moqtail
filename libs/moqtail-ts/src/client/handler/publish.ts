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

import { FilterType, GroupOrder, Publish, PublishOk } from '../../model/control'
import { ControlMessageHandler } from './handler'

export const handlerPublish: ControlMessageHandler<Publish> = async (client, msg) => {
  // Bubble the event up to the application layer
  if (client.onPeerPublish) {
    client.onPeerPublish(msg)
  }

  // Implicit Consent: Automatically accept the publish request.
  // Note: Adjust the PublishOk constructor parameters to match your specific
  // Draft version implementation in ../../model/control.
  const publishOk = new PublishOk(
    msg.requestId,
    1,
    255,
    GroupOrder.Ascending,
    FilterType.LatestObject,
    undefined,
    undefined,
    [],
  )
  await client.controlStream.send(publishOk)
}
