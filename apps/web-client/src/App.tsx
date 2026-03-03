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

import { useRef, useState } from 'react'
import { MOQtailClient } from 'moqtail-ts/client'
import { FullTrackName, Tuple, ObjectForwardingPreference } from 'moqtail-ts/model'
import { createMoqtailClient } from './moq/client'
import { setupSignalling } from './moq/signalling'
import {
  announceNamespaces,
  setupTracks,
  startVideoEncoder,
  startAudioEncoder,
  prepareReceiverForCanvas,
  pipeStreamToCanvas,
} from './composables/useVideoPipeline'
import './startup'

type Screen = 'select' | 'connecting' | 'main'

export default function App() {
  const [screen, setScreen] = useState<Screen>('select')
  const [userId, setUserId] = useState<1 | 2 | null>(null)
  const [isCamOn, setIsCamOn] = useState(false)
  const [isMicOn, setIsMicOn] = useState(false)
  const [isPublishing, setIsPublishing] = useState(false)
  const [isReceiving, setIsReceiving] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const localVideoRef = useRef<HTMLVideoElement>(null)
  const remoteCanvasRef = useRef<HTMLCanvasElement>(null)
  const mediaStreamRef = useRef<MediaStream | null>(null)
  const stopEncoderRef = useRef<(() => Promise<void>) | null>(null)
  // Stable refs for use inside async callbacks
  const moqClientRef = useRef<MOQtailClient | null>(null)
  const userIdRef = useRef<1 | 2 | null>(null)
  const sendSignalRef = useRef<((text: string) => void) | null>(null)
  const isMicOnRef = useRef(false)
  const subscribedNsRef = useRef(new Set<string>())
  const workerRef = useRef<Worker | null>(null)

  async function subscribeNamespaceOnce(client: MOQtailClient, ns: Tuple) {
    console.log('subscribeNamespaceOnce: ns: %s', ns.toUtf8Path())
    const key = ns.toString()
    if (subscribedNsRef.current.has(key)) return
    subscribedNsRef.current.add(key)
    await client.subscribeNamespace(ns)
  }

  async function handleUserSelect(id: 1 | 2) {
    setUserId(id)
    userIdRef.current = id
    setError(null)
    subscribedNsRef.current.clear()
    workerRef.current = null
    setScreen('connecting')

    try {
      const client = await createMoqtailClient()
      moqClientRef.current = client

      // Wire up PUBLISH handler before signalling starts
      client.onPeerPublish = (msg, stream) => {
        const worker = workerRef.current
        let trackName = new TextDecoder().decode(msg.fullTrackName.name)

        if (!worker) {
          console.warn('[onTrackPublished] no decode worker ready yet for track:', trackName)
          return
        }

        if (trackName === 'video') {
          pipeStreamToCanvas(stream, 'moq', worker)
          setIsReceiving(true)
        } else if (trackName === 'audio') {
          pipeStreamToCanvas(stream, 'moq-audio', worker)
        }
      }

      // Switch to main screen before the 1s signalling delay so the canvas
      // is mounted and ready before we start receiving peer signals.
      setScreen('main')

      const { sendSignal } = await setupSignalling(client, id, {
        onPeerJoin: (peerId) => {
          console.log('onPeerJoin peerId: %d', peerId)
          initializeCanvas()
          const peerNs = Tuple.fromUtf8Path(`moqtail/demo/user_${peerId}`)
          subscribeNamespaceOnce(client, peerNs)
          sendSignalRef.current?.(`user_${peerId}:welcome`)
        },
        onOwnJoinWelcomed: () => {
          const otherId = (id === 1 ? 2 : 1) as 1 | 2
          console.log('onOwnJoinWelcomed: otherId: %d', otherId)
          initializeCanvas()
          subscribeNamespaceOnce(client, Tuple.fromUtf8Path(`moqtail/demo/user_${otherId}`))
        },
      })

      sendSignalRef.current = sendSignal
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err)
      setError(msg)
      setScreen('select')
    }
  }

  function initializeCanvas() {
    // Pre-create decode worker + audio pipeline for the remote canvas
    if (workerRef.current) return
    prepareReceiverForCanvas(remoteCanvasRef).then((w) => {
      console.log('remoteCanvasRef ready', w)
      workerRef.current = w
    })
  }

  async function startPublishing() {
    const client = moqClientRef.current
    const id = userIdRef.current
    if (!client || !id) return

    const stream = await navigator.mediaDevices.getUserMedia({
      video: { aspectRatio: 16 / 9 },
      audio: true,
    })
    // Respect current mic state
    stream.getAudioTracks().forEach((t) => {
      t.enabled = isMicOnRef.current
    })
    mediaStreamRef.current = stream

    if (localVideoRef.current) {
      localVideoRef.current.srcObject = stream
      localVideoRef.current.muted = true
    }

    const userNs = Tuple.fromUtf8Path(`moqtail/demo/user_${id}`)
    const videoFTN = FullTrackName.tryNew(userNs, 'video')
    const audioFTN = FullTrackName.tryNew(userNs, 'audio')

    await announceNamespaces(client, userNs)
    const tracks = setupTracks(client, audioFTN, videoFTN)

    // Retrieve track aliases assigned by addOrUpdateTrack (inside setupTracks)
    const videoTrack = client.trackSources.get(videoFTN.toString())!
    const audioTrack = client.trackSources.get(audioFTN.toString())!

    // PUBLISH messages inform namespace subscribers (who subscribed via subscribeNamespace)
    // of the new tracks; onTrackPublished fires on the subscriber with the data stream.
    await client.publish(videoFTN, true, videoTrack.trackAlias!)
    await client.publish(audioFTN, true, audioTrack.trackAlias!)

    const videoResult = await startVideoEncoder({
      stream,
      videoFullTrackName: videoFTN,
      videoStreamController: tracks.getVideoStreamController(),
      publisherPriority: 1,
      objectForwardingPreference: ObjectForwardingPreference.Subgroup,
    })

    await startAudioEncoder({
      stream,
      audioFullTrackName: audioFTN,
      audioStreamController: tracks.getAudioStreamController(),
      publisherPriority: 1,
      audioGroupId: 0,
      objectForwardingPreference: ObjectForwardingPreference.Subgroup,
    })

    stopEncoderRef.current = videoResult?.stop ?? null
    setIsPublishing(true)
  }

  async function stopPublishing() {
    await stopEncoderRef.current?.()
    stopEncoderRef.current = null
    mediaStreamRef.current?.getTracks().forEach((t) => t.stop())
    mediaStreamRef.current = null
    if (localVideoRef.current) localVideoRef.current.srcObject = null
    setIsPublishing(false)
    setIsCamOn(false)
    isMicOnRef.current = false
    setIsMicOn(false)
  }

  async function toggleCam() {
    if (isCamOn) {
      await stopPublishing()
    } else {
      setIsCamOn(true)
      await startPublishing()
    }
  }

  async function toggleMic() {
    const next = !isMicOn
    isMicOnRef.current = next
    setIsMicOn(next)
    if (mediaStreamRef.current) {
      mediaStreamRef.current.getAudioTracks().forEach((t) => {
        t.enabled = next
      })
    }
  }

  // ── User selection screen ──────────────────────────────────────────────────
  if (screen === 'select') {
    return (
      <div style={styles.center}>
        <div style={styles.selectCard}>
          <h1 style={styles.title}>MOQtail Demo</h1>
          <p style={styles.subtitle}>Which user are you?</p>
          <div style={styles.selectButtons}>
            <button onClick={() => handleUserSelect(1)} style={styles.selectBtn}>
              User 1
            </button>
            <button onClick={() => handleUserSelect(2)} style={styles.selectBtn}>
              User 2
            </button>
          </div>
          {error && (
            <div style={styles.errorBox}>
              <strong>Connection failed</strong>
              <p style={{ marginTop: 8, fontSize: 13 }}>{error}</p>
            </div>
          )}
        </div>
      </div>
    )
  }

  // ── Connecting screen ──────────────────────────────────────────────────────
  if (screen === 'connecting') {
    return (
      <div style={styles.center}>
        <div style={styles.statusText}>Connecting to relay…</div>
      </div>
    )
  }

  // ── Main screen ────────────────────────────────────────────────────────────
  return (
    <div style={styles.root}>
      <div style={styles.header}>
        <h1 style={styles.title}>MOQtail Demo</h1>
        <div style={styles.badges}>
          <span style={styles.badge('gray')}>User {userId}</span>
          {isPublishing && <span style={styles.badge('green')}>Publishing</span>}
          {isReceiving && <span style={styles.badge('blue')}>Receiving</span>}
        </div>
      </div>

      <div style={styles.videos}>
        <div style={styles.videoBox}>
          <span style={styles.videoLabel}>You (Local)</span>
          <video ref={localVideoRef} autoPlay playsInline muted style={styles.video} />
        </div>
        <div style={styles.videoBox}>
          <span style={styles.videoLabel}>Remote</span>
          <canvas ref={remoteCanvasRef} style={styles.video} />
        </div>
      </div>

      <div style={styles.controls}>
        <button onClick={toggleCam} style={styles.btn(isCamOn)}>
          {isCamOn ? 'Cam On' : 'Cam Off'}
        </button>
        <button onClick={toggleMic} style={styles.btn(isMicOn)} disabled={!isCamOn}>
          {isMicOn ? 'Mic On' : 'Mic Off'}
        </button>
      </div>
    </div>
  )
}

const styles = {
  root: {
    minHeight: '100vh',
    display: 'flex',
    flexDirection: 'column' as const,
    gap: 16,
    padding: 20,
    background: '#0d0d1a',
  },
  center: {
    minHeight: '100vh',
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'center',
    background: '#0d0d1a',
  },
  selectCard: {
    background: '#1a1a2e',
    borderRadius: 12,
    padding: '32px 40px',
    display: 'flex',
    flexDirection: 'column' as const,
    alignItems: 'center',
    gap: 20,
    minWidth: 320,
  },
  title: {
    fontSize: 20,
    fontWeight: 700,
    color: '#a0aec0',
    letterSpacing: 2,
    textTransform: 'uppercase' as const,
    textAlign: 'center' as const,
    margin: 0,
  },
  subtitle: {
    color: '#718096',
    fontSize: 15,
    margin: 0,
  },
  selectButtons: {
    display: 'flex',
    gap: 12,
  },
  selectBtn: {
    padding: '10px 32px',
    borderRadius: 8,
    border: 'none',
    cursor: 'pointer',
    fontWeight: 700,
    fontSize: 15,
    background: '#3182ce',
    color: '#e2e8f0',
  },
  statusText: {
    color: '#718096',
    fontSize: 16,
  },
  errorBox: {
    background: '#2d1515',
    border: '1px solid #e53e3e',
    borderRadius: 8,
    padding: '12px 20px',
    color: '#fc8181',
    maxWidth: 400,
    width: '100%',
    textAlign: 'center' as const,
    fontSize: 14,
  },
  header: {
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'space-between',
  },
  badges: {
    display: 'flex',
    gap: 6,
  },
  badge: (color: 'gray' | 'green' | 'blue') => ({
    fontSize: 11,
    padding: '2px 8px',
    borderRadius: 99,
    background: color === 'green' ? '#276749' : color === 'blue' ? '#2b4f8c' : '#2d3748',
    color: '#e2e8f0',
  }),
  videos: {
    display: 'flex',
    gap: 16,
    flex: 1,
  },
  videoBox: {
    flex: 1,
    display: 'flex',
    flexDirection: 'column' as const,
    gap: 6,
    background: '#1a1a2e',
    borderRadius: 12,
    padding: 12,
    minWidth: 0,
  },
  videoLabel: {
    fontSize: 12,
    color: '#718096',
    textAlign: 'center' as const,
  },
  video: {
    width: '100%',
    aspectRatio: '16/9',
    background: '#000',
    borderRadius: 6,
    display: 'block',
  },
  controls: {
    display: 'flex',
    gap: 12,
    justifyContent: 'center',
  },
  btn: (active: boolean) => ({
    padding: '10px 32px',
    borderRadius: 8,
    border: 'none',
    cursor: 'pointer',
    fontWeight: 600,
    fontSize: 14,
    background: active ? '#3182ce' : '#2d3748',
    color: '#e2e8f0',
  }),
}
