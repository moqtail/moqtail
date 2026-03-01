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

import { MOQtailClient, LiveTrackSource } from 'moqtail-ts/client'
import {
  FullTrackName,
  Tuple,
  MoqtObject,
  Location,
  ObjectForwardingPreference,
  FilterType,
  GroupOrder,
  SubscribeError,
} from 'moqtail-ts/model'

const MOQTAIL_DEMO_NS = 'moqtail/demo'
const SIGNALLING_TRACK_NAME = 'signalling'
const PUBLISH_TO_SUBSCRIBE_DELAY_MS = 500
const SUBSCRIBE_TO_JOIN_DELAY_MS = 500

export type SignalCallbacks = {
  onPeerJoin: (peerId: 1 | 2) => void
  onOwnJoinWelcomed: () => void
}

export async function setupSignalling(
  moqClient: MOQtailClient,
  userId: 1 | 2,
  callbacks: SignalCallbacks,
): Promise<{ sendSignal: (text: string) => void; cleanup: () => void }> {
  const demoNs = Tuple.fromUtf8Path(MOQTAIL_DEMO_NS)
  const signalFTN = FullTrackName.tryNew(demoNs, SIGNALLING_TRACK_NAME)

  // 1. Register the shared signalling track locally
  let objectId = 0
  let signalController: ReadableStreamDefaultController<MoqtObject> | null = null
  const signalStream = new ReadableStream<MoqtObject>({
    start(controller) {
      signalController = controller
    },
    cancel() {
      signalController = null
    },
  })
  const textSource = new LiveTrackSource(signalStream)
  moqClient.addOrUpdateTrack({
    fullTrackName: signalFTN,
    forwardingPreference: ObjectForwardingPreference.Subgroup,
    trackSource: { live: textSource },
    publisherPriority: 1,
  })

  function sendSignal(text: string) {
    if (!signalController) return
    const payload = new TextEncoder().encode(text)
    signalController.enqueue(
      MoqtObject.newWithPayload(
        signalFTN,
        new Location(0n, BigInt(objectId++)),
        1,
        ObjectForwardingPreference.Subgroup,
        1,
        null,
        payload,
      ),
    )
  }

  // 2. PUBLISH — tell the relay about this track so it registers the alias
  //    and can route our outgoing data stream to subscribers.
  const sigTrack = moqClient.trackSources.get(signalFTN.toString())
  console.log('signalling: sigTrack', sigTrack, 'trackAlias', sigTrack?.trackAlias)
  if (sigTrack?.trackAlias != null) {
    console.log('signalling: calling publish with trackAlias', sigTrack.trackAlias)
    const publishResult = await moqClient.publish(signalFTN, true, sigTrack.trackAlias)
    console.log('signalling: publish result', publishResult)
  } else {
    console.warn('signalling: track alias not yet assigned, skipping PUBLISH')
  }

  // 3. Wait for the relay to process the PUBLISH and for the peer to connect
  console.log('signalling: waiting for relay to process PUBLISH...')
  await new Promise<void>((r) => setTimeout(r, PUBLISH_TO_SUBSCRIBE_DELAY_MS))
  console.log('signalling: done waiting, now subscribing')

  // 4. SUBSCRIBE — start receiving messages from all publishers on this track
  console.log('signalling: calling subscribe for', signalFTN.toString())
  const subResponse = await moqClient.subscribe({
    fullTrackName: signalFTN,
    groupOrder: GroupOrder.Original,
    filterType: FilterType.LatestObject,
    forward: true,
    priority: 1,
  })

  console.log('signalling: subscribe response', subResponse)
  if (subResponse instanceof SubscribeError) {
    console.warn('signalling: subscribe failed', subResponse)
  } else {
    const { stream } = subResponse
    const reader = stream.getReader()
    ;(async () => {
      try {
        for (;;) {
          const { done, value } = await reader.read()
          if (done || !value) break
          if (!value.payload) continue
          const text = new TextDecoder().decode(value.payload)
          const match = text.match(/^user_([12]):(.+)$/)
          if (!match) continue
          const msgUserId = parseInt(match[1]) as 1 | 2
          const signal = match[2]
          if (signal === 'join') {
            console.log('Join message was received for user %d.', userId)
            if (msgUserId !== userId) {
              callbacks.onPeerJoin(msgUserId)
            }
          } else if (signal === 'welcome') {
            console.log('Welcome message was received for user %d.', userId)
            if (msgUserId === userId) {
              callbacks.onOwnJoinWelcomed()
            }
          }
        }
      } catch (err) {
        console.warn('signalling: reader error:', err)
      }
    })()
  }

  // 5. Wait for the subscription to establish, then send our join signal
  console.log('signalling: waiting for subscription to establish...')
  await new Promise<void>((r) => setTimeout(r, SUBSCRIBE_TO_JOIN_DELAY_MS))
  console.log('signalling: sending join signal for user', userId)
  sendSignal(`user_${userId}:join`)
  console.log('Join message was sent.')

  return {
    sendSignal,
    cleanup: () => {
      if (signalController) {
        try {
          signalController.close()
        } catch {
          // already closed
        }
      }
    },
  }
}
