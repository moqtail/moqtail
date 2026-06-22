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

import { useState, useRef, useCallback, useEffect } from 'preact/hooks';
import { uniqueNamesGenerator, colors, animals } from 'unique-names-generator';
import { Player } from '@/lib/player';
import { Publisher } from '@/lib/publisher';
import { Tuple, type CMSFTrack } from 'moqtail';
import MSEBuffer from '@/lib/buffer';
import type { Track, Status, SourceState, SourceKind, PublishStatus } from '@/types';
import { Header } from '@/components/Header';
import { Sidebar } from '@/components/Sidebar';
import { VideoPlayer } from '@/components/VideoPlayer';
import { LocalPreview } from '@/components/LocalPreview';
import { logger } from '@/lib/logger';
import { applyLogLevel, parseLogLevel } from '@/lib/utils';
import { parseMsfUrl, buildMsfUrl } from '@/lib/msf-url';

logger.setDefaultLevel('debug');

type Tab = 'watch' | 'publish';

function generateNamespace(): string {
  const slug = uniqueNamesGenerator({
    dictionaries: [colors, animals],
    separator: '-',
    length: 2,
    style: 'lowerCase',
  });
  return `moqtail/${slug}`;
}

function getWatchUrl(relayUrl: string, namespace: string): string {
  const msfUrl = buildMsfUrl(relayUrl, namespace);
  if (!msfUrl) return '';
  const base = window.location.origin + window.location.pathname;
  return `${base}?url=${encodeURIComponent(msfUrl)}`;
}

function sortTracks(tracks: CMSFTrack[]) {
  return tracks.sort((a, b) => {
    // sort by bitrate (desc), then resolution (desc), then name (asc)
    const bitrateA = a.bitrate || 0;
    const bitrateB = b.bitrate || 0;
    if (bitrateA !== bitrateB) return bitrateB - bitrateA;

    const resA = (a.width || 0) * (a.height || 0);
    const resB = (b.width || 0) * (b.height || 0);
    if (resA !== resB) return resB - resA;

    return a.name.localeCompare(b.name);
  });
}

const DEFAULT_SOURCES: SourceState[] = [
  { kind: 'camera', enabled: false, available: false, embedTimestamp: false },
  { kind: 'screen', enabled: false, available: false, embedTimestamp: false },
  { kind: 'test', enabled: false, available: true, embedTimestamp: true },
];

export function App() {
  // Playback state
  const [relayUrl, setRelayUrl] = useState('https://relay.moqtail.dev');
  const [namespace, setNamespace] = useState('moqtail/testsrc');
  const [status, setStatus] = useState<Status>('idle');
  const [tracks, setTracks] = useState<Track[]>([]);
  const [selectedVideo, setSelectedVideo] = useState<string | null>(null);
  const [selectedAudio, setSelectedAudio] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const playerRef = useRef<Player | null>(null);
  const bufferRef = useRef<MSEBuffer | null>(null);
  const videoRef = useRef<HTMLVideoElement | null>(null);

  // Tab state
  const [tab, setTab] = useState<Tab>('watch');

  // Publish state
  const [publishRelayUrl, setPublishRelayUrl] = useState('https://relay.moqtail.dev');
  const [publishNamespace, setPublishNamespace] = useState(generateNamespace);
  const [publishSources, setPublishSources] = useState<SourceState[]>(DEFAULT_SOURCES);
  const [publishStatus, setPublishStatus] = useState<PublishStatus>('idle');
  const [publishError, setPublishError] = useState<string | null>(null);
  const publisherRef = useRef<Publisher | null>(null);

  // Pre-populate watch fields from a ?url=moqt://... query param.
  useEffect(() => {
    const raw = new URLSearchParams(window.location.search).get('url');
    if (!raw) return;
    const parts = parseMsfUrl(raw);
    if (!parts) return;
    setRelayUrl(parts.relayUrl);
    setNamespace(parts.namespace);
  }, []);

  // Check device availability when switching to publish tab
  useEffect(() => {
    if (tab !== 'publish') return;
    const check = async () => {
      const md = navigator.mediaDevices;
      const hasGetDisplay = typeof md?.getDisplayMedia === 'function';

      let hasCamera = false;
      let camReason: string | undefined;

      try {
        const devices = await md.enumerateDevices();
        hasCamera = devices.some(d => d.kind === 'videoinput');
        if (!hasCamera) camReason = 'No camera detected';
      } catch {
        camReason = 'Cannot enumerate devices';
      }

      setPublishSources(prev =>
        prev.map(s => {
          if (s.kind === 'camera')
            return { ...s, available: hasCamera, unavailableReason: camReason };
          if (s.kind === 'screen')
            return {
              ...s,
              available: hasGetDisplay,
              unavailableReason: hasGetDisplay ? undefined : 'Screen capture not supported',
            };
          return s; // test source always available
        }),
      );
    };
    check();
  }, [tab]);

  const disposePlayer = useCallback(async () => {
    if (playerRef.current) {
      try {
        await playerRef.current.dispose();
      } catch {}
      playerRef.current = null;
    }
    if (bufferRef.current) {
      try {
        bufferRef.current.dispose();
      } catch {}
      bufferRef.current = null;
    }
  }, []);

  const initializePlaybackSession = useCallback(async () => {
    if (!videoRef.current) return null;

    const qs = new URLSearchParams(window.location.search).get('logLevel');
    const raw = qs ?? (window as any).__moqtailLogLevel ?? 'warn';
    applyLogLevel(parseLogLevel(raw) ?? parseLogLevel('warn')!);
    logger.info('player', `log level: "${raw}"`);

    logger.info('app', `initializePlaybackSession: relay="${relayUrl}" ns="${namespace}"`);
    const player = new Player({
      relayUrl,
      namespace: Tuple.fromUtf8Path(namespace),
      receiveCatalogViaSubscribe: true,
    });
    playerRef.current = player;

    const catalog = await player.initialize();
    const allTracks = sortTracks(catalog.getTracks());
    logger.info(
      'app',
      `initializePlaybackSession: ${allTracks.length} track(s): ${allTracks.map(t => `${t.name}(${t.role})`).join(', ')}`,
    );
    setTracks(allTracks);

    await player.attachMedia(videoRef.current);
    bufferRef.current = new MSEBuffer(videoRef.current);
    logger.info('app', 'initializePlaybackSession: media attached, MSEBuffer created');

    return { player, allTracks };
  }, [relayUrl, namespace]);

  const handleConnect = useCallback(async () => {
    if (!videoRef.current) return;
    setStatus('connecting');
    setError(null);
    setTracks([]);
    setSelectedVideo(null);
    setSelectedAudio(null);

    await disposePlayer();

    try {
      const session = await initializePlaybackSession();
      if (!session) return;

      const { player, allTracks } = session;

      const firstVideo = allTracks.find(t => t.role === 'video');
      logger.info('app', `handleConnect: firstVideo="${firstVideo?.name ?? 'none'}"`);
      if (firstVideo) {
        setSelectedVideo(firstVideo.name);
        setStatus('restarting');
        await player.addMediaTrack(firstVideo.name);
        logger.info('app', 'handleConnect: addMediaTrack done, calling startMedia');
        await player.startMedia();
        logger.info('app', 'handleConnect: startMedia done — status=playing');
        setStatus('playing');
      } else {
        logger.warn('app', 'handleConnect: no video track found in catalog');
        setStatus('ready');
      }
    } catch (err) {
      logger.error('app', `handleConnect: error — ${(err as Error).message}`);
      setError((err as Error).message);
      setStatus('error');
      await disposePlayer();
    }
  }, [disposePlayer, initializePlaybackSession]);

  const startPlayback = useCallback(
    async (videoTrack: string | null, audioTrack: string | null) => {
      if (!videoRef.current) return;
      if (!videoTrack && !audioTrack) {
        await disposePlayer();
        setStatus('ready');
        return;
      }

      logger.info(
        'app',
        `startPlayback: video="${videoTrack ?? 'none'}" audio="${audioTrack ?? 'none'}"`,
      );
      setStatus('restarting');
      await disposePlayer();

      try {
        const session = await initializePlaybackSession();
        if (!session) return;

        const { player } = session;

        if (videoTrack) await player.addMediaTrack(videoTrack);
        if (audioTrack) await player.addMediaTrack(audioTrack);

        logger.info('app', 'startPlayback: calling startMedia');
        await player.startMedia();
        logger.info('app', 'startPlayback: startMedia done — status=playing');
        setStatus('playing');
      } catch (err) {
        logger.error('app', `startPlayback: error — ${(err as Error).message}`);
        setError((err as Error).message);
        setStatus('error');
        await disposePlayer();
      }
    },
    [disposePlayer, initializePlaybackSession],
  );

  const handleTrackChange = useCallback(
    (track: Track, checked: boolean) => {
      if (track.role !== 'video' && track.role !== 'audio') return;

      let newVideo = selectedVideo;
      let newAudio = selectedAudio;

      if (track.role === 'video') {
        newVideo = track.name === selectedVideo && !checked ? null : track.name;
      } else {
        newAudio = track.name === selectedAudio && !checked ? null : track.name;
      }

      setSelectedVideo(newVideo);
      setSelectedAudio(newAudio);
      startPlayback(newVideo, newAudio);
    },
    [selectedVideo, selectedAudio, startPlayback],
  );

  const handleSourceToggle = useCallback(async (kind: SourceKind, enabled: boolean) => {
    if (enabled) {
      try {
        let stream: MediaStream;
        if (kind === 'camera') {
          stream = await navigator.mediaDevices.getUserMedia({
            video: {
              width: { ideal: 1280 },
              height: { ideal: 720 },
              aspectRatio: { ideal: 16 / 9 },
            },
          });
        } else if (kind === 'screen') {
          stream = await (navigator.mediaDevices as any).getDisplayMedia({
            video: true,
            audio: false,
          });
        } else {
          // test source — no stream needed
          setPublishSources(prev => prev.map(s => (s.kind === kind ? { ...s, enabled: true } : s)));
          return;
        }

        setPublishSources(prev =>
          prev.map(s => (s.kind === kind ? { ...s, enabled: true, stream } : s)),
        );
      } catch (err) {
        // Permission denied or cancelled — leave unchecked
        logger.warn('app', `source toggle denied for ${kind}: ${(err as Error).message}`);
      }
    } else {
      setPublishSources(prev =>
        prev.map(s => {
          if (s.kind !== kind) return s;
          s.stream?.getTracks().forEach(t => t.stop());
          return { ...s, enabled: false, stream: undefined };
        }),
      );
    }
  }, []);

  const handleTimestampToggle = useCallback((kind: SourceKind, embed: boolean) => {
    setPublishSources(prev =>
      prev.map(s => (s.kind === kind ? { ...s, embedTimestamp: embed } : s)),
    );
  }, []);

  const handlePublish = useCallback(async () => {
    if (publisherRef.current) return;

    setPublishStatus('connecting');
    setPublishError(null);

    const nsParts = publishNamespace.split('/').filter(Boolean);
    const publisher = new Publisher({
      relayUrl: publishRelayUrl,
      namespace: nsParts,
      sources: publishSources,
      onStatus: (s, err) => {
        setPublishStatus(s);
        setPublishError(err ?? null);
      },
    });
    publisherRef.current = publisher;
    await publisher.start();
  }, [publishRelayUrl, publishNamespace, publishSources]);

  const handleStop = useCallback(async () => {
    await publisherRef.current?.stop();
    publisherRef.current = null;
    setPublishStatus('idle');
    setPublishError(null);
  }, []);

  const handleRefreshNamespace = useCallback(() => {
    if (publishStatus === 'publishing' || publishStatus === 'connecting') return;
    setPublishNamespace(generateNamespace());
  }, [publishStatus]);

  const handleTabChange = useCallback(
    async (next: Tab) => {
      if (next === 'publish' && tab === 'watch') {
        await disposePlayer();
        setStatus('idle');
        setError(null);
        setTracks([]);
        setSelectedVideo(null);
        setSelectedAudio(null);
      }
      setTab(next);
    },
    [tab, disposePlayer],
  );

  // Derived state
  const hasTracks = tracks.length > 0;
  const activeCameraStream =
    publishSources.find(s => s.kind === 'camera' && s.enabled)?.stream ?? null;
  const activeScreenStream =
    publishSources.find(s => s.kind === 'screen' && s.enabled)?.stream ?? null;
  const isTestSource = publishSources.some(s => s.kind === 'test' && s.enabled);
  const testStream =
    isTestSource && publishStatus === 'publishing'
      ? (publisherRef.current?.testStream ?? null)
      : null;
  const previewCameraStream = activeCameraStream ?? testStream;
  const showPreview =
    tab === 'publish' &&
    (publishStatus === 'publishing' || !!activeCameraStream || !!activeScreenStream) &&
    (!!previewCameraStream || !!activeScreenStream);

  return (
    <div className="flex h-dvh w-dvw flex-col bg-neutral-950 font-sans text-neutral-100 antialiased">
      <Header status={status} />

      <div className="flex h-full min-h-0 w-full flex-1 grow flex-col md:flex-row">
        <Sidebar
          // Watch tab props
          relayUrl={relayUrl}
          onRelayUrlChange={setRelayUrl}
          namespace={namespace}
          onNamespaceChange={setNamespace}
          status={status}
          tracks={tracks}
          selectedVideo={selectedVideo}
          selectedAudio={selectedAudio}
          onConnect={handleConnect}
          onTrackChange={handleTrackChange}
          error={error}
          // Tab
          tab={tab}
          onTabChange={handleTabChange}
          // Publish tab props
          publishProps={{
            relayUrl: publishRelayUrl,
            onRelayUrlChange: setPublishRelayUrl,
            namespace: publishNamespace,
            onNamespaceChange: setPublishNamespace,
            onRefreshNamespace: handleRefreshNamespace,
            sources: publishSources,
            onSourceToggle: handleSourceToggle,
            onTimestampToggle: handleTimestampToggle,
            publishStatus,
            onPublish: handlePublish,
            onStop: handleStop,
            error: publishError,
            watchUrl: getWatchUrl(publishRelayUrl, publishNamespace),
          }}
        />

        {/* Main area */}
        {tab === 'watch' ? (
          <VideoPlayer ref={videoRef} hasTracks={hasTracks} mode="watch" />
        ) : showPreview ? (
          <main className="relative flex flex-1 flex-col items-center justify-center overflow-hidden bg-neutral-950 p-4 md:p-6">
            <div
              className="w-full overflow-hidden rounded-xl bg-black shadow-2xl shadow-black/60"
              style={{ aspectRatio: '16/9' }}
            >
              <LocalPreview cameraStream={previewCameraStream} screenStream={activeScreenStream} />
            </div>
          </main>
        ) : (
          <VideoPlayer ref={videoRef} hasTracks={false} mode="publish" />
        )}
      </div>
    </div>
  );
}
