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
import { Tuple } from 'moqtail';
import { formatMsfUrl, parseMsfUrl, type CatalogElement, type MOQtailPlayer } from 'moqtail/player';
import type { MOQtailVideoElement } from 'moqtail/player/component';
import { Publisher } from '@/lib/publisher';
import type { Track, Status, SourceState, SourceKind, PublishStatus } from '@/types';
import { Header } from '@/components/Header';
import { Sidebar } from '@/components/Sidebar';
import { VideoPlayer } from '@/components/VideoPlayer';
import { LocalPreview } from '@/components/LocalPreview';
import { Advanced } from '@/components/Advanced';
import { followData } from '@/lib/data';
import { logger } from '@/lib/logger';
import { applyLogLevel, parseLogLevel } from '@/lib/utils';

logger.setDefaultLevel('debug');

type Tab = 'watch' | 'publish';

const ADVANCED_KEY = 'moqtail.advanced';
/** How recently a track must have received an object to count as active. */
const ACTIVE_WINDOW_MS = 1000;

function generateNamespace(): string {
  const slug = uniqueNamesGenerator({
    dictionaries: [colors, animals],
    separator: '-',
    length: 2,
    style: 'lowerCase',
  });
  return `moqtail/${slug}`;
}

/** The MSF URL of a namespace's catalog: the player starts from it. Any relay scheme becomes `moqt://`. */
function catalogUrl(relayUrl: string, namespace: string): string {
  const relay = relayUrl.trim().replace(/^[a-z][\w+.-]*:\/\//i, 'moqt://');
  return formatMsfUrl(relay, Tuple.fromUtf8Path(namespace), 'catalog');
}

/** The moqtail.dev relays implement SWITCH_FROM, so renditions switch with switch-from. */
function supportsSwitchFrom(url: string): boolean {
  return URL.parse(url)?.hostname.endsWith('.moqtail.dev') ?? false;
}

function getWatchUrl(relayUrl: string, namespace: string): string {
  const base = window.location.origin + window.location.pathname;
  return `${base}?url=${encodeURIComponent(catalogUrl(relayUrl, namespace))}`;
}

function sortTracks(tracks: readonly Track[]): Track[] {
  return [...tracks].sort((a, b) => {
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

/** Per role, the tracks that received an object within the window, else the one that did last. */
function activeTracksOf(tracks: Track[], lastObject: Map<string, number>, now: number): string[] {
  return ['video', 'audio'].flatMap(role => {
    const seen = tracks.filter(t => t.role === role && lastObject.has(t.name));
    const recent = seen.filter(t => now - lastObject.get(t.name)! < ACTIVE_WINDOW_MS);
    if (recent.length > 0) return recent.map(t => t.name);
    const latest = seen.sort((a, b) => lastObject.get(b.name)! - lastObject.get(a.name)!)[0];
    return latest ? [latest.name] : [];
  });
}

function readAdvanced(): boolean {
  try {
    return localStorage.getItem(ADVANCED_KEY) === 'on';
  } catch {
    return false;
  }
}

const DEFAULT_SOURCES: SourceState[] = [
  { kind: 'camera', enabled: false, available: false, embedTimestamp: false },
  { kind: 'screen', enabled: false, available: false, embedTimestamp: false },
  { kind: 'test', enabled: false, available: true, embedTimestamp: true },
];

export function App() {
  // Playback state
  const [relayUrl, setRelayUrl] = useState('moqt://relay.moqtail.dev');
  const [namespace, setNamespace] = useState('moqtail/testsrc');
  const [status, setStatus] = useState<Status>('idle');
  const [tracks, setTracks] = useState<Track[]>([]);
  const [activeTracks, setActiveTracks] = useState<string[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [player, setPlayer] = useState<MOQtailPlayer>();
  const [catalog, setCatalog] = useState<CatalogElement>();
  const [advanced, setAdvanced] = useState(readAdvanced);

  const videoRef = useRef<MOQtailVideoElement | null>(null);

  // Tab state
  const [tab, setTab] = useState<Tab>('watch');

  // Publish state
  const [publishRelayUrl, setPublishRelayUrl] = useState('moqt://relay.moqtail.dev');
  const [publishNamespace, setPublishNamespace] = useState(generateNamespace);
  const [publishSources, setPublishSources] = useState<SourceState[]>(DEFAULT_SOURCES);
  const [publishStatus, setPublishStatus] = useState<PublishStatus>('idle');
  const [publishError, setPublishError] = useState<string | null>(null);
  const publisherRef = useRef<Publisher | null>(null);

  // Pre-populate watch fields from a ?url=moqt://... query param.
  useEffect(() => {
    const raw = new URLSearchParams(window.location.search).get('url');
    if (!raw) return;
    try {
      const source = parseMsfUrl(raw);
      setRelayUrl(source.url);
      setNamespace(source.namespace.fields.map(f => f.toUtf8()).join('/'));
    } catch {
      logger.warn('app', `ignoring an invalid ?url= "${raw}"`);
    }
  }, []);

  const handleAdvancedChange = useCallback((on: boolean) => {
    setAdvanced(on);
    try {
      localStorage.setItem(ADVANCED_KEY, on ? 'on' : 'off');
    } catch {}
  }, []);

  // A player that can't start (a bad URL, an unreachable relay) is reported here
  useEffect(() => {
    const video = videoRef.current;
    if (!video) return;
    const onError = () => {
      setError(video.error?.message ?? 'Playback failed');
      setStatus('error');
    };
    video.addEventListener('error', onError);
    return () => video.removeEventListener('error', onError);
  }, [tab]);

  // The tracks objects arrive on, so a ladder's shorter GoPs show while they deliver
  const tracksRef = useRef(tracks);
  tracksRef.current = tracks;
  useEffect(() => {
    if (!player) return;
    const lastObject = new Map<string, number>();
    const stop = followData(player, (event, track) => {
      if (track && event.kind === 'object') lastObject.set(track, event.at);
    });
    const id = setInterval(() => {
      const active = activeTracksOf(tracksRef.current, lastObject, performance.now());
      setActiveTracks(previous => (previous.join() === active.join() ? previous : active));
    }, 250);
    return () => {
      stop();
      clearInterval(id);
    };
  }, [player]);

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

  const resetPlayback = useCallback((next: Status) => {
    setStatus(next);
    setError(null);
    setTracks([]);
    setActiveTracks([]);
    setPlayer(undefined);
    setCatalog(undefined);
  }, []);

  const handleConnect = useCallback(() => {
    const video = videoRef.current;
    if (!video) return;

    const qs = new URLSearchParams(window.location.search).get('logLevel');
    const raw = qs ?? (window as any).__moqtailLogLevel ?? 'warn';
    applyLogLevel(parseLogLevel(raw) ?? parseLogLevel('warn')!);

    resetPlayback('connecting');
    const url = catalogUrl(relayUrl, namespace);
    logger.info('app', `connect: ${url}`);
    // Read when the element loads, so it's set before `src`
    video.toggleAttribute('switch-from', supportsSwitchFrom(url));
    if (video.getAttribute('src') === url) video.load();
    else video.src = url;

    const current = video.player;
    if (!current) return; // the element reports why with `error`
    setPlayer(current);
    // The previous player's teardown still emits; only the element's current one counts
    current.on('statechange', state => {
      if (video.player === current) setStatus(state);
    });
    current.on('catalog', element => {
      setCatalog(element);
      setTracks(sortTracks(element.tracks));
    });
  }, [relayUrl, namespace, resetPlayback]);

  const disposePlayer = useCallback(() => {
    videoRef.current?.removeAttribute('src');
    resetPlayback('idle');
  }, [resetPlayback]);

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
    (next: Tab) => {
      if (next === 'publish' && tab === 'watch') disposePlayer();
      setTab(next);
    },
    [tab, disposePlayer],
  );

  // Derived state
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
      <Header status={status} advanced={advanced} onAdvancedChange={handleAdvancedChange} />

      <div className="flex h-full min-h-0 w-full flex-1 grow flex-col md:flex-row">
        <Sidebar
          // Watch tab props
          relayUrl={relayUrl}
          onRelayUrlChange={setRelayUrl}
          namespace={namespace}
          onNamespaceChange={setNamespace}
          status={status}
          tracks={tracks}
          activeTracks={activeTracks}
          onConnect={handleConnect}
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
          <div className="flex min-h-0 min-w-0 flex-1 flex-col">
            <VideoPlayer ref={videoRef} status={status} />
            {advanced && <Advanced videoRef={videoRef} player={player} catalog={catalog} />}
          </div>
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
          <VideoPlayer status="idle" mode="publish" />
        )}
      </div>
    </div>
  );
}
