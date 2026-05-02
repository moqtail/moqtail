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

import { MOQtailClient, LiveTrackSource } from 'moqtail/client';
import {
  FullTrackName,
  Tuple,
  MoqtObject,
  Location,
  ObjectForwardingPreference,
  FilterType,
  GroupOrder,
  RequestError,
} from 'moqtail/model';

const MOQTAIL_DEMO_NS = 'moqtail/demo';
const SIGNALLING_TRACK_NAME = 'signalling';
const PUBLISH_TO_SUBSCRIBE_DELAY_MS = 500;
const SUBSCRIBE_TO_JOIN_DELAY_MS = 500;
// We need a random starting point for group ids not to collide with other publisher's group ids
const BASE_GROUP_ID = BigInt(Math.ceil(Math.random() * 1000000));

// Signal formats:
//   join:          {sender}:{senderToken}:join
//   welcome:       {sender}:{receiver}:{receiverToken}:welcome
//   duplicate_user:{sender}:{receiver}:{receiverToken}:duplicate_user
// username: alphanumeric + dashes (spaces replaced), max 25 chars
// token: random hex session token per session

export type SignalCallbacks = {
  onPeerJoin: (peerUsername: string) => void;
  onOwnJoinWelcomed: (peerUsername: string) => void;
  onDuplicateUser: () => void;
};

export async function setupSignalling(
  moqClient: MOQtailClient,
  username: string,
  token: string,
  callbacks: SignalCallbacks,
): Promise<{ sendSignal: (text: string) => void; cleanup: () => void }> {
  const demoNs = Tuple.fromUtf8Path(MOQTAIL_DEMO_NS);
  const signalFTN = FullTrackName.tryNew(demoNs, SIGNALLING_TRACK_NAME);

  // 1. Register the shared signalling track locally
  let lastGroupId = BASE_GROUP_ID;
  let objectId = 0n;
  let signalController: ReadableStreamDefaultController<MoqtObject> | null = null;
  const signalStream = new ReadableStream<MoqtObject>({
    start(controller) {
      signalController = controller;
    },
    cancel() {
      signalController = null;
    },
  });
  const textSource = new LiveTrackSource(signalStream);
  moqClient.addOrUpdateTrack({
    fullTrackName: signalFTN,
    forwardingPreference: ObjectForwardingPreference.Subgroup,
    trackSource: { live: textSource },
    publisherPriority: 1,
    trackAlias: 1000n, // need to use the same track alias
  });

  function sendSignal(text: string) {
    if (!signalController) return;

    let groupId = lastGroupId + 1n;

    objectId = 0n;
    lastGroupId = groupId;

    const payload = new TextEncoder().encode(text);

    console.log('sendSignal', text, groupId);

    signalController.enqueue(
      MoqtObject.newWithPayload(
        signalFTN,
        new Location(groupId, objectId),
        1,
        ObjectForwardingPreference.Subgroup,
        0,
        null,
        payload,
      ),
    );
  }

  // 2. PUBLISH — tell the relay about this track so it registers the alias
  //    and can route our outgoing data stream to subscribers.
  const sigTrack = moqClient.trackSources.get(signalFTN.toString());
  console.log('signalling: sigTrack', sigTrack, 'trackAlias', sigTrack?.trackAlias);
  if (sigTrack?.trackAlias != null) {
    console.log('signalling: calling publish with trackAlias', sigTrack.trackAlias);
    const publishResult = await moqClient.publish(signalFTN, true, sigTrack.trackAlias);
    console.log('signalling: publish result', publishResult);
  } else {
    console.warn('signalling: track alias not yet assigned, skipping PUBLISH');
  }

  // 3. Wait for the relay to process the PUBLISH and for the peer to connect
  console.log('signalling: waiting for relay to process PUBLISH...');
  await new Promise<void>(r => setTimeout(r, PUBLISH_TO_SUBSCRIBE_DELAY_MS));
  console.log('signalling: done waiting, now subscribing');

  // 4. SUBSCRIBE — start receiving messages from all publishers on this track
  console.log('signalling: calling subscribe for', signalFTN.toString());
  const subResponse = await moqClient.subscribe({
    fullTrackName: signalFTN,
    groupOrder: GroupOrder.Original,
    filterType: FilterType.LatestObject,
    forward: true,
    priority: 1,
  });

  console.log('signalling: subscribe response', subResponse);
  if (subResponse instanceof RequestError) {
    console.warn('signalling: subscribe failed', subResponse);
  } else {
    const { stream } = subResponse;
    const reader = stream.getReader();
    (async () => {
      try {
        for (;;) {
          const { done, value } = await reader.read();
          if (done || !value) break;
          if (!value.payload) continue;
          const text = new TextDecoder().decode(value.payload);
          // Formats:
          //   join:          {sender}:{senderToken}:join
          //   welcome:       {sender}:{receiver}:{receiverToken}:welcome
          //   duplicate_user:{sender}:{receiver}:{receiverToken}:duplicate_user
          const joinMatch = text.match(/^([^:]+):([^:]+):join$/);
          const welcomeMatch = text.match(/^([^:]+):([^:]+):([^:]+):welcome$/);
          const dupMatch = text.match(/^([^:]+):([^:]+):([^:]+):duplicate_user$/);

          if (joinMatch) {
            const [, sender, senderToken] = joinMatch;
            console.log('signal received: join from %s (token: %s)', sender, senderToken);
            if (sender === username && senderToken === token) {
              // Own echo, ignore
              continue;
            }
            if (sender === username) {
              // Another session joined with the same username — they are the duplicate
              console.log('Duplicate user detected: %s', sender);
              sendSignal(`${username}:${sender}:${senderToken}:duplicate_user`);
            } else {
              callbacks.onPeerJoin(sender);
              sendSignal(`${username}:${sender}:${senderToken}:welcome`);
            }
          } else if (welcomeMatch) {
            const [, sender, receiver, receiverToken] = welcomeMatch;
            console.log('signal received: welcome from %s for %s', sender, receiver);
            if (receiver === username && receiverToken === token) {
              callbacks.onOwnJoinWelcomed(sender);
            }
          } else if (dupMatch) {
            const [, sender, receiver, receiverToken] = dupMatch;
            console.log('signal received: duplicate_user from %s for %s', sender, receiver);
            if (receiver === username && receiverToken === token) {
              callbacks.onDuplicateUser();
            }
          }
        }
      } catch (err) {
        console.warn('signalling: reader error:', err);
      }
    })();
  }

  // 5. Wait for the subscription to establish, then send our join signal
  console.log('signalling: waiting for subscription to establish...');
  await new Promise<void>(r => setTimeout(r, SUBSCRIBE_TO_JOIN_DELAY_MS));
  console.log('signalling: sending join signal for', username);
  sendSignal(`${username}:${token}:join`);
  console.log('Join message was sent.');

  return {
    sendSignal,
    cleanup: () => {
      if (signalController) {
        try {
          signalController.close();
        } catch {
          // already closed
        }
      }
    },
  };
}
