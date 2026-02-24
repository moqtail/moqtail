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

import { Publish } from '../../model/control/publish'
import { Subscribe } from '../../model/control/subscribe'
import { GroupOrder } from '../../model/control/constant'
import { FullTrackName } from '../../model/data/full_track_name'
import { SubscribeRequest } from '../request/subscribe'
import { ControlMessageHandler } from './handler'

/**
 * Handles an incoming PUBLISH message on the subscriber side.
 * The relay forwards PUBLISH to all matching namespace subscribers so they
 * can begin receiving the auto-subscribed track's data streams.
 */
export const handlerPublish: ControlMessageHandler<Publish> = async (client, msg) => {
  const fullTrackName = FullTrackName.tryNew(msg.trackNamespace, msg.trackName)

  // Register alias → full track name so incoming subgroup streams can be attributed
  client.aliasFullTrackNameMap.set(msg.trackAlias, fullTrackName)

  // Create a synthetic SubscribeRequest so #handleRecvStreams can enqueue objects
  const syntheticSub = Subscribe.newLatestObject(msg.requestId, fullTrackName, 0, GroupOrder.Original, true, [])
  const request = new SubscribeRequest(syntheticSub)
  client.subscriptions.set(msg.trackAlias, request)

  // Notify application so it can consume the object stream
  client.onTrackPublished?.(msg, request.stream)
}
