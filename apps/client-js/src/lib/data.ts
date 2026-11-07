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

import type { DataEvent, DataHeaderEvent, MOQtailClient } from 'moqtail';
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

  const use = (next: MOQtailClient) => {
    client?.off('dataevent', onData);
    client = next;
    client.on('dataevent', onData);
  };
  if (player.client) use(player.client);
  player.on('client', use);
  return () => {
    player.off('client', use);
    client?.off('dataevent', onData);
  };
}
