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

import type { ControlMessage, DataEvent, DataHeaderEvent, MOQtailClient } from 'moqtail';
import type { MOQtailPlayer } from 'moqtail/player';

const decoder = new TextDecoder();

/**
 * Calls `listener` with each incoming data event of the player's client, across reconnects,
 * and the name of the track its stream carries. A stream can open before the response naming
 * its track, so `track` stays undefined until a later event of that stream. Streams opened
 * before the call are skipped. Returns the function that stops listening.
 */
export function followData(
  player: MOQtailPlayer,
  listener: (event: DataEvent, track: string | undefined) => void,
): () => void {
  const streams = new Map<number, { header: DataHeaderEvent; track?: string }>();
  let client: MOQtailClient | undefined;

  const onData = (event: DataEvent) => {
    if (event.direction !== 'in') return;
    if (event.kind === 'header') streams.set(event.streamId, { header: event });
    const stream = streams.get(event.streamId);
    if (!stream) return;
    if (event.kind === 'end') streams.delete(event.streamId);
    if (stream.track === undefined) {
      const name = client?.trackNameOf(stream.header);
      if (name) stream.track = decoder.decode(name.name);
    }
    listener(event, stream.track);
  };

  return eachClient(player, next => {
    client = next;
    next.on('dataevent', onData);
    return () => next.off('dataevent', onData);
  });
}

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

/** Runs `attach` on the player's current and every later client, detaching the one before. */
function eachClient(player: MOQtailPlayer, attach: (client: MOQtailClient) => () => void) {
  let detach: (() => void) | undefined;
  const use = (client: MOQtailClient) => {
    detach?.();
    detach = attach(client);
  };
  if (player.client) use(player.client);
  player.on('client', use);
  return () => {
    player.off('client', use);
    detach?.();
  };
}
