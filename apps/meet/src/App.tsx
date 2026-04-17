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

import { useRef, useState } from 'react';
import { MOQtailClient } from 'moqtail/client';
import {
  FullTrackName,
  Tuple,
  ObjectForwardingPreference,
  DefaultPublisherPriorityExtension,
} from 'moqtail/model';
import { createMoqtailClient } from './moq/client';
import { setupSignalling } from './moq/signalling';
import {
  announceNamespaces,
  setupTracks,
  startVideoEncoder,
  startAudioEncoder,
  prepareReceiverForCanvas,
  pipeStreamToCanvas,
} from './composables/useVideoPipeline';
import './startup';
import { LogLevel } from 'moqtail/util';

type Screen = 'entrance' | 'connecting' | 'main';

const USERNAME_MAX_LEN = 25;
const USERNAME_REGEX = /^[a-zA-Z0-9 ]+$/;
const NS_PREFIX = '/moqtail/demo/';

function getDefaultRelay(): string {
  const url = window.appSettings?.relayUrl ?? 'https://localhost:4433';
  return url.replace(/^https?:\/\//, '');
}

function toTrackUsername(username: string): string {
  return username.trim().replace(/ /g, '-');
}

function generateToken(): string {
  return Math.random().toString(16).slice(2, 10);
}

function GitHubIcon() {
  return (
    <svg height="18" viewBox="0 0 16 16" width="18" fill="currentColor" aria-hidden="true">
      <path d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.013 8.013 0 0016 8c0-4.42-3.58-8-8-8z" />
    </svg>
  );
}

const GITHUB_REPO = 'moqtail/moqtail';

export default function App() {
  const [screen, setScreen] = useState<Screen>('entrance');
  const [usernameInput, setUsernameInput] = useState('');
  const [relayInput, setRelayInput] = useState(getDefaultRelay);
  const [displayUsername, setDisplayUsername] = useState('');
  const [peers, setPeers] = useState<string[]>([]);
  const [isCamOn, setIsCamOn] = useState(false);
  const [isMicOn, setIsMicOn] = useState(false);
  const [isPublishing, setIsPublishing] = useState(false);
  const [isReceiving, setIsReceiving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const localVideoRef = useRef<HTMLVideoElement>(null);
  const mediaStreamRef = useRef<MediaStream | null>(null);
  const stopEncoderRef = useRef<(() => Promise<void>) | null>(null);
  // Stable refs for use inside async callbacks
  const moqClientRef = useRef<MOQtailClient | null>(null);
  const trackUsernameRef = useRef<string>('');
  const sendSignalRef = useRef<((text: string) => void) | null>(null);
  const isMicOnRef = useRef(false);
  const subscribedNsRef = useRef(new Set<string>());
  const signalCleanupRef = useRef<(() => void) | null>(null);
  // Per-peer canvas elements, decode workers, and pending streams (queued before worker is ready)
  const peerCanvasesRef = useRef<Map<string, HTMLCanvasElement>>(new Map());
  const peerWorkersRef = useRef<Map<string, Worker>>(new Map());
  const pendingStreamsRef = useRef<Map<string, Array<(worker: Worker) => void>>>(new Map());

  // set log level to debug
  MOQtailClient.setLogLevel(LogLevel.DEBUG);

  function validateUsernameInput(value: string): string | null {
    if (!value.trim()) return 'Username is required';
    if (!USERNAME_REGEX.test(value)) return 'Only letters, numbers, and spaces allowed';
    if (value.length > USERNAME_MAX_LEN) return `Max ${USERNAME_MAX_LEN} characters`;
    return null;
  }

  async function subscribeNamespaceOnce(client: MOQtailClient, ns: Tuple) {
    console.log('subscribeNamespaceOnce: ns: %s', ns.toUtf8Path());
    const key = ns.toUtf8Path();
    if (subscribedNsRef.current.has(key)) return;
    subscribedNsRef.current.add(key);
    await client.subscribeNamespace(ns);
  }

  function addPeer(peerUsername: string) {
    setPeers(prev => (prev.includes(peerUsername) ? prev : [...prev, peerUsername]));
  }

  function delay(ms: number) {
    return new Promise(resolve => {
      setTimeout(() => {
        resolve('');
      }, ms);
    });
  }

  async function handleEnter() {
    const validationError = validateUsernameInput(usernameInput);
    if (validationError) {
      setError(validationError);
      return;
    }

    const trackUsername = toTrackUsername(usernameInput);
    const token = generateToken();
    const relayUrl = `https://${relayInput}`;

    trackUsernameRef.current = trackUsername;
    setDisplayUsername(usernameInput.trim());
    setError(null);
    setPeers([]);
    subscribedNsRef.current.clear();
    peerCanvasesRef.current.clear();
    peerWorkersRef.current.clear();
    pendingStreamsRef.current.clear();
    setScreen('connecting');

    try {
      const client = await createMoqtailClient(relayUrl);
      moqClientRef.current = client;

      // Route incoming publish streams to the correct peer's decode worker
      client.onPeerPublish = (msg, stream) => {
        const nsPath = msg.fullTrackName.namespace.toUtf8Path();
        const peerUsername = nsPath.startsWith(NS_PREFIX) ? nsPath.slice(NS_PREFIX.length) : null;
        console.log('onPeerPublish | peerUsername: %s', peerUsername, nsPath);
        if (!peerUsername) return;

        const worker = peerWorkersRef.current.get(peerUsername);
        const trackName = new TextDecoder().decode(msg.fullTrackName.name);

        if (!worker) {
          // Worker not ready yet — queue and drain once the canvas mounts
          const pipe =
            trackName === 'video'
              ? (w: Worker) => {
                  pipeStreamToCanvas(stream, 'moq', w);
                  setIsReceiving(true);
                }
              : trackName === 'audio'
                ? (w: Worker) => pipeStreamToCanvas(stream, 'moq-audio', w)
                : null;
          if (pipe) {
            const q = pendingStreamsRef.current.get(peerUsername) ?? [];
            q.push(pipe);
            pendingStreamsRef.current.set(peerUsername, q);
          }
          return;
        }

        if (trackName === 'video') {
          pipeStreamToCanvas(stream, 'moq', worker);
          setIsReceiving(true);
        } else if (trackName === 'audio') {
          pipeStreamToCanvas(stream, 'moq-audio', worker);
        }
      };

      setScreen('main');

      const { sendSignal, cleanup } = await setupSignalling(client, trackUsername, token, {
        onPeerJoin: peerUsername => {
          console.log('onPeerJoin peerUsername: %s', peerUsername);
          addPeer(peerUsername);
          const peerNs = Tuple.fromUtf8Path(`${NS_PREFIX}${peerUsername}`);
          // add a random delay
          delay(Math.ceil(Math.random() * 100)).then(_ => {
            subscribeNamespaceOnce(client, peerNs);
          });
        },
        onOwnJoinWelcomed: peerUsername => {
          console.log('onOwnJoinWelcomed: welcomed by %s', peerUsername);
          addPeer(peerUsername);
          const peerNs = Tuple.fromUtf8Path(`${NS_PREFIX}${peerUsername}`);
          delay(Math.ceil(Math.random() * 100)).then(_ => {
            subscribeNamespaceOnce(client, peerNs);
          });
        },
        onDuplicateUser: () => {
          console.warn('Duplicate username detected, returning to entrance');
          cleanup();
          setError('Username already taken. Please choose a different one.');
          setScreen('entrance');
        },
      });

      sendSignalRef.current = sendSignal;
      signalCleanupRef.current = cleanup;
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      setError(msg);
      setScreen('entrance');
    }
  }

  async function startPublishing() {
    const client = moqClientRef.current;
    const trackUsername = trackUsernameRef.current;
    if (!client || !trackUsername) return;

    const stream = await navigator.mediaDevices.getUserMedia({
      video: { aspectRatio: 16 / 9 },
      audio: true,
    });
    stream.getAudioTracks().forEach(t => {
      t.enabled = isMicOnRef.current;
    });
    mediaStreamRef.current = stream;

    if (localVideoRef.current) {
      localVideoRef.current.srcObject = stream;
      localVideoRef.current.muted = true;
    }

    const userNs = Tuple.fromUtf8Path(`${NS_PREFIX}${trackUsername}`);
    const videoFTN = FullTrackName.tryNew(userNs, 'video');
    const audioFTN = FullTrackName.tryNew(userNs, 'audio');

    await announceNamespaces(client, userNs);
    const tracks = setupTracks(client, audioFTN, videoFTN);

    const videoTrack = client.trackSources.get(videoFTN.toString())!;
    const audioTrack = client.trackSources.get(audioFTN.toString())!;

    await client.publish(videoFTN, true, videoTrack.trackAlias!, undefined, [
      new DefaultPublisherPriorityExtension(128),
    ]);
    await client.publish(audioFTN, true, audioTrack.trackAlias!, undefined, [
      new DefaultPublisherPriorityExtension(0),
    ]);

    const videoResult = await startVideoEncoder({
      stream,
      videoFullTrackName: videoFTN,
      videoStreamController: tracks.getVideoStreamController(),
      publisherPriority: 1,
      objectForwardingPreference: ObjectForwardingPreference.Subgroup,
    });

    await startAudioEncoder({
      stream,
      audioFullTrackName: audioFTN,
      audioStreamController: tracks.getAudioStreamController(),
      publisherPriority: 1,
      audioGroupId: 0,
      objectForwardingPreference: ObjectForwardingPreference.Subgroup,
    });

    stopEncoderRef.current = videoResult?.stop ?? null;
    setIsPublishing(true);
  }

  async function stopPublishing() {
    await stopEncoderRef.current?.();
    stopEncoderRef.current = null;
    mediaStreamRef.current?.getTracks().forEach(t => t.stop());
    mediaStreamRef.current = null;
    if (localVideoRef.current) localVideoRef.current.srcObject = null;
    setIsPublishing(false);
    setIsCamOn(false);
    isMicOnRef.current = false;
    setIsMicOn(false);
  }

  async function toggleCam() {
    if (isCamOn) {
      await stopPublishing();
    } else {
      setIsCamOn(true);
      await startPublishing();
    }
  }

  async function toggleMic() {
    const next = !isMicOn;
    isMicOnRef.current = next;
    setIsMicOn(next);
    if (mediaStreamRef.current) {
      mediaStreamRef.current.getAudioTracks().forEach(t => {
        t.enabled = next;
      });
    }
  }

  // ── Entrance screen ────────────────────────────────────────────────────────
  if (screen === 'entrance') {
    const isUsernameInvalid =
      usernameInput.length > 0 &&
      (usernameInput.length > USERNAME_MAX_LEN || !USERNAME_REGEX.test(usernameInput));

    return (
      <div className="flex min-h-dvh items-center justify-center bg-neutral-950 font-sans antialiased">
        <div className="flex min-w-80 flex-col items-center gap-5 rounded-xl border border-white/6 bg-neutral-900/60 px-10 py-8 backdrop-blur-sm">
          <div className="flex items-center gap-2.5">
            <img src="/favicon.svg" alt="MOQtail logo" className="h-5 w-5" />
            <span className="text-sm font-semibold tracking-tight text-neutral-100">
              Meet by MOQtail
            </span>
          </div>

          <div className="flex w-full flex-col gap-3">
            <div className="flex flex-col gap-1.5">
              <label className="text-xs font-medium text-neutral-400" htmlFor="username">
                Username
              </label>
              <input
                id="username"
                type="text"
                value={usernameInput}
                maxLength={USERNAME_MAX_LEN}
                placeholder="Your name"
                autoFocus
                onKeyDown={e => {
                  if (e.key === 'Enter') handleEnter();
                }}
                onChange={e => {
                  setUsernameInput(e.target.value);
                  setError(null);
                }}
                className={`w-full rounded-lg border bg-neutral-800/60 px-3 py-2 text-sm text-neutral-100 placeholder-neutral-600 transition-colors outline-none focus:ring-1 ${
                  isUsernameInvalid
                    ? 'border-red-500/40 focus:border-red-500/60 focus:ring-red-500/30'
                    : 'border-white/8 focus:border-white/20 focus:ring-white/10'
                }`}
              />
              <div className="flex items-center justify-between">
                {isUsernameInvalid ? (
                  <span className="text-[11px] text-red-400">
                    {!USERNAME_REGEX.test(usernameInput)
                      ? 'Only letters, numbers, and spaces'
                      : `Max ${USERNAME_MAX_LEN} characters`}
                  </span>
                ) : (
                  <span />
                )}
                <span
                  className={`ml-auto text-[11px] ${usernameInput.length > USERNAME_MAX_LEN ? 'text-red-400' : 'text-neutral-600'}`}
                >
                  {usernameInput.length}/{USERNAME_MAX_LEN}
                </span>
              </div>
            </div>

            <div className="flex flex-col gap-1.5">
              <label className="text-xs font-medium text-neutral-400" htmlFor="relay">
                Relay
              </label>
              <input
                id="relay"
                type="text"
                value={relayInput}
                onKeyDown={e => {
                  if (e.key === 'Enter') handleEnter();
                }}
                onChange={e => {
                  setRelayInput(e.target.value);
                  setError(null);
                }}
                className="w-full rounded-lg border border-white/8 bg-neutral-800/60 px-3 py-2 text-sm text-neutral-100 placeholder-neutral-600 transition-colors outline-none focus:border-white/20 focus:ring-1 focus:ring-white/10"
              />
            </div>
          </div>

          <button
            onClick={handleEnter}
            disabled={isUsernameInvalid || !usernameInput.trim()}
            className="w-full rounded-lg bg-blue-600 px-8 py-2.5 text-sm font-semibold text-white transition-colors hover:bg-blue-500 active:bg-blue-700 disabled:cursor-not-allowed disabled:opacity-40"
          >
            Enter
          </button>

          {error && (
            <p className="w-full rounded-lg border border-red-500/20 bg-red-500/10 px-3 py-2 text-center text-xs leading-relaxed text-red-400">
              {error}
            </p>
          )}
        </div>
      </div>
    );
  }

  // ── Connecting screen ──────────────────────────────────────────────────────
  if (screen === 'connecting') {
    return (
      <div className="flex min-h-dvh items-center justify-center bg-neutral-950 font-sans antialiased">
        <span className="flex items-center gap-1.5 text-xs text-neutral-400 select-none">
          <span className="h-2 w-2 animate-pulse rounded-full bg-yellow-400" />
          Connecting to relay…
        </span>
      </div>
    );
  }

  // ── Main screen ────────────────────────────────────────────────────────────
  // Fixed 3×2 grid: 1 local + 5 peer slots
  const peerSlots = Array.from({ length: 5 }, (_, i) => peers[i] ?? null);

  return (
    <div className="flex h-dvh flex-col bg-neutral-950 font-sans text-neutral-100 antialiased">
      {/* Header */}
      <header className="flex h-12 shrink-0 items-center justify-between border-b border-white/6 bg-neutral-950/80 px-4 backdrop-blur-sm md:px-5">
        <div className="flex items-center gap-2.5">
          <img src="/favicon.svg" alt="MOQtail logo" className="h-5 w-5" />
          <span className="text-sm font-semibold tracking-tight">Meet by MOQtail</span>
          <span className="text-neutral-700 select-none">·</span>
          <span className="text-xs text-neutral-400 select-none">{displayUsername}</span>
        </div>
        <div className="flex items-center gap-2">
          {isPublishing && (
            <span className="rounded-full bg-emerald-500/15 px-2 py-0.5 text-[11px] font-medium text-emerald-400 ring-1 ring-emerald-500/30">
              Publishing
            </span>
          )}
          {isReceiving && (
            <span className="rounded-full bg-blue-500/15 px-2 py-0.5 text-[11px] font-medium text-blue-400 ring-1 ring-blue-500/30">
              Receiving
            </span>
          )}
          <a
            href={`https://github.com/${GITHUB_REPO}`}
            target="_blank"
            rel="noreferrer"
            className="flex items-center gap-2 text-sm text-neutral-400 transition-colors hover:text-neutral-100"
          >
            <GitHubIcon />
            <span className="hidden text-xs font-medium sm:block">{GITHUB_REPO}</span>
          </a>
        </div>
      </header>

      {/* Videos — fixed 3×2 grid: 1 local + 5 peer slots */}
      <div className="grid min-h-0 flex-1 grid-cols-3 gap-4 p-4">
        {/* Local */}
        <div className="flex min-w-0 flex-col gap-2 rounded-xl border border-white/6 bg-neutral-900/60 p-3">
          <span className="text-center text-[11px] font-semibold tracking-widest text-neutral-500 uppercase select-none">
            {displayUsername} (You)
          </span>
          <video
            ref={localVideoRef}
            autoPlay
            playsInline
            muted
            className="w-full rounded-lg bg-black"
            style={{ aspectRatio: '16/9' }}
          />
        </div>

        {/* 5 peer slots */}
        {peerSlots.map((peerUsername, i) => (
          <div
            key={peerUsername ?? `empty-${i}`}
            className="flex min-w-0 flex-col gap-2 rounded-xl border border-white/6 bg-neutral-900/60 p-3"
          >
            {peerUsername ? (
              <>
                <span className="text-center text-[11px] font-semibold tracking-widest text-neutral-500 uppercase select-none">
                  {peerUsername}
                </span>
                <canvas
                  ref={el => {
                    if (!el || peerCanvasesRef.current.has(peerUsername)) return;
                    peerCanvasesRef.current.set(peerUsername, el);
                    prepareReceiverForCanvas({ current: el }).then(w => {
                      if (!w) return;
                      peerWorkersRef.current.set(peerUsername, w);
                      // Drain any streams that arrived before the worker was ready
                      pendingStreamsRef.current.get(peerUsername)?.forEach(fn => fn(w));
                      pendingStreamsRef.current.delete(peerUsername);
                    });
                  }}
                  className="w-full rounded-lg bg-black"
                  style={{ aspectRatio: '16/9' }}
                />
              </>
            ) : (
              <div
                className="flex flex-1 items-center justify-center rounded-lg border border-dashed border-white/10"
                style={{ aspectRatio: '16/9' }}
              >
                <span className="text-[11px] text-neutral-700 select-none">Waiting…</span>
              </div>
            )}
          </div>
        ))}
      </div>

      {/* Controls */}
      <div className="flex shrink-0 items-center justify-center gap-3 border-t border-white/6 px-4 py-3">
        <button
          onClick={toggleCam}
          className={`rounded-lg px-8 py-2 text-sm font-medium transition-colors ${
            isCamOn
              ? 'bg-blue-600 text-white hover:bg-blue-500 active:bg-blue-700'
              : 'bg-neutral-800 text-neutral-300 hover:bg-neutral-700 active:bg-neutral-900'
          }`}
        >
          {isCamOn ? 'Cam On' : 'Cam Off'}
        </button>
        <button
          onClick={toggleMic}
          disabled={!isCamOn}
          className={`rounded-lg px-8 py-2 text-sm font-medium transition-colors disabled:cursor-not-allowed disabled:opacity-40 ${
            isMicOn
              ? 'bg-blue-600 text-white hover:bg-blue-500 active:bg-blue-700'
              : 'bg-neutral-800 text-neutral-300 hover:bg-neutral-700 active:bg-neutral-900'
          }`}
        >
          {isMicOn ? 'Mic On' : 'Mic Off'}
        </button>
      </div>
    </div>
  );
}
