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

import { useEffect, useRef } from 'preact/hooks';
import { abr, liveBuffer, MOQtailPlayer, ssts } from 'moqtail/player';
import type { Features } from '@/types';

/** The live buffer below the 0.5 s defaults, so they don't floor the 0.25 s target. */
const LIVE_BUFFER = { targetLatency: 0.25, stallThreshold: 0.1, tolerance: 0.05 };
/** Matches `stallThreshold`, else every stall refills 0.5 s before playing again. */
const PLAYOUT = { preroll: 0.1 };

/**
 * Plays `src` on a canvas through the player API, until `<moqtail-video>` can. Each `session`
 * starts a new player; `onPlayer` hears of it with `isCurrent`, false once it's being replaced.
 */
export function CanvasPlayer({
  src,
  session,
  features,
  onPlayer,
  onError,
}: {
  src: string | undefined;
  session: number;
  features: Features;
  onPlayer: (player: MOQtailPlayer, isCurrent: () => boolean) => void;
  onError: (message: string) => void;
}) {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const playerRef = useRef<MOQtailPlayer | undefined>(undefined);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas || !src) return;
    // The plugins `<moqtail-video>` installs by default
    const plugins = features.ssts
      ? [ssts(), liveBuffer(LIVE_BUFFER)]
      : [liveBuffer(LIVE_BUFFER), abr()];
    const player = new MOQtailPlayer({ plugins });
    let current = true;
    playerRef.current = player;
    onPlayer(player, () => current);
    player.on('error', error => {
      if (current) onError(error.message);
    });
    try {
      player
        .source(src, { switchFrom: features.switchFrom && !features.ssts })
        .sink(canvas, PLAYOUT);
      // Failures arrive as `error`, and a blocked autoplay waits for a click on the canvas
      player.start().catch(() => {});
    } catch (error) {
      onError((error as Error).message);
    }
    return () => {
      current = false;
      void player.destroy();
    };
  }, [src, session]);

  // After a blocked autoplay, the click is the user activation the audio needs
  const play = () => playerRef.current?.play().catch(() => {});
  return (
    // A fresh canvas per player, since a worker player keeps the last one's control
    <canvas
      key={`${session}:${src}`}
      ref={canvasRef}
      onClick={play}
      className="block h-full w-full object-contain"
    />
  );
}
