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

import { SubscribeNamespace, SubscribeNamespaceOk } from '../../model/control'
import { ControlMessageHandler } from './handler'

export const handlerSubscribeNamespace: ControlMessageHandler<SubscribeNamespace> = async (client, msg) => {
  // Bubble the event up to the application layer
  if (client.onPeerSubscribeNamespace) {
    client.onPeerSubscribeNamespace(msg)
  }

  // Implicit Consent: Automatically acknowledge the namespace subscription.
  const okMsg = new SubscribeNamespaceOk(msg.requestId)
  await client.controlStream.send(okMsg)
}
