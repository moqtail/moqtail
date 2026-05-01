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

import { useState, useRef, useCallback } from 'preact/hooks';
import { Player } from '@/lib/player';
import { Tuple, type CMSFTrack } from 'moqtail';
import MSEBuffer from '@/lib/buffer';
import type { Track, Status } from '@/types';
import { Header } from '@/components/Header';
import { Sidebar } from '@/components/Sidebar';
import { VideoPlayer } from '@/components/VideoPlayer';

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

export function App() {
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

    const player = new Player({
      relayUrl,
      namespace: Tuple.fromUtf8Path(namespace),
      receiveCatalogViaSubscribe: true,
    });
    playerRef.current = player;

    const catalog = await player.initialize();
    const allTracks = sortTracks(catalog.getTracks());
    setTracks(allTracks);

    await player.attachMedia(videoRef.current);
    bufferRef.current = new MSEBuffer(videoRef.current);

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
      if (firstVideo) {
        setSelectedVideo(firstVideo.name);
        setStatus('restarting');
        await player.addMediaTrack(firstVideo.name);
        await player.startMedia();
        setStatus('playing');
      } else {
        setStatus('ready');
      }
    } catch (err) {
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

      setStatus('restarting');
      await disposePlayer();

      try {
        const session = await initializePlaybackSession();
        if (!session) return;

        const { player } = session;

        if (videoTrack) await player.addMediaTrack(videoTrack);
        if (audioTrack) await player.addMediaTrack(audioTrack);

        await player.startMedia();
        setStatus('playing');
      } catch (err) {
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
        // clicking the active track unchecks it; clicking any other switches to it
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

  const hasTracks = tracks.length > 0;

  return (
    <div className="flex h-dvh w-dvw flex-col bg-neutral-950 font-sans text-neutral-100 antialiased">
      <Header status={status} />

      {/* Body */}
      <div className="flex h-full min-h-0 w-full flex-1 grow flex-col md:flex-row">
        <Sidebar
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
        />
        <VideoPlayer ref={videoRef} hasTracks={hasTracks} />
      </div>
    </div>
  );
}
