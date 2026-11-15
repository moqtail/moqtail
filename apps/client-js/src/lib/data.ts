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

import type { ControlMessage } from 'moqtail';
import { eachClient, type MOQtailPlayer } from 'moqtail/player';

export { followData } from 'moqtail/player';

/**
 * Calls `listener` with each control message the player's client sends or receives, across
 * reconnects. Each client's SETUP exchange is replayed when it's attached, since it happened
 * earlier. Returns the function that stops listening.
 */
export function followMessages(
  player: MOQtailPlayer,
  listener: (message: ControlMessage, direction: 'in' | 'out') => void,
): () => void {
  const onIn = (message: ControlMessage) => listener(message, 'in');
  const onOut = (message: ControlMessage) => listener(message, 'out');
  return eachClient(player, client => {
    // The handshake finishes before the player hands out its client, so replay it
    if (client.clientSetup) onOut(client.clientSetup);
    if (client.serverSetup) onIn(client.serverSetup);
    client.on('messagereceived', onIn);
    client.on('messagesent', onOut);
    return () => {
      client.off('messagereceived', onIn);
      client.off('messagesent', onOut);
    };
  });
}
