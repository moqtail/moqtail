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

import { cn } from '@/lib/utils';
import { forwardRef } from 'preact/compat';
import { LuSun, LuPlay, LuCircleDot } from 'react-icons/lu';

const GITHUB_REPO = 'moqtail/moqtail';

function PublishIdleCard() {
  return (
    <div className="absolute inset-0 flex flex-col items-center justify-center gap-6 px-6 text-center select-none">
      {/* Icon */}
      <div className="flex h-16 w-16 items-center justify-center rounded-2xl border border-white/6 bg-neutral-900 text-neutral-600">
        <LuSun className="h-8 w-8" />
      </div>

      <p className="text-sm font-medium text-neutral-300">
        Configure sources to start broadcasting
      </p>

      <div className="w-full max-w-sm space-y-3 rounded-xl border border-white/6 bg-neutral-900/60 p-4 text-left backdrop-blur-sm">
        <p className="text-xs leading-relaxed text-neutral-400">
          Select a source on the left and click{' '}
          <span className="font-medium text-neutral-300">Go Live</span> to broadcast over{' '}
          <a
            href="https://datatracker.ietf.org/doc/draft-ietf-moq-transport/"
            target="_blank"
            rel="noreferrer"
            className="text-blue-400 underline decoration-blue-400/30 underline-offset-2 transition-colors hover:text-blue-300"
          >
            MOQT
          </a>
          .
        </p>
        <ul className="space-y-1.5 text-xs text-neutral-500">
          <li className="flex items-start gap-2">
            <span className="mt-0.5 h-1.5 w-1.5 shrink-0 rounded-full bg-violet-500" />
            Camera and screen share are composited into a single video track (PiP when both active)
          </li>
          <li className="flex items-start gap-2">
            <span className="mt-0.5 h-1.5 w-1.5 shrink-0 rounded-full bg-teal-500" />
            Catalog is published every second in CMSF format — compatible with the Watch tab
          </li>
          <li className="flex items-start gap-2">
            <span className="mt-0.5 h-1.5 w-1.5 shrink-0 rounded-full bg-blue-500" />
            Test source emits a live clock overlay for quick end-to-end verification
          </li>
        </ul>
        <div className="border-t border-white/6 pt-3">
          <a
            href={`https://github.com/${GITHUB_REPO}/issues`}
            target="_blank"
            rel="noreferrer"
            className="inline-flex items-center gap-1.5 text-xs text-neutral-500 transition-colors hover:text-neutral-300"
          >
            <LuCircleDot className="h-3.5 w-3.5 shrink-0" aria-hidden="true" />
            Found a problem? Open an issue on GitHub.
          </a>
        </div>
      </div>
    </div>
  );
}

function WatchIdleCard() {
  return (
    <div className="absolute inset-0 flex flex-col items-center justify-center gap-6 px-6 text-center select-none">
      {/* Icon */}
      <div className="flex h-16 w-16 items-center justify-center rounded-2xl border border-white/6 bg-neutral-900 text-neutral-600">
        <LuPlay className="h-8 w-8" />
      </div>

      {/* Heading */}
      <p className="text-sm font-medium text-neutral-300">Connect to a relay to start playback</p>

      {/* Info card */}
      <div className="w-full max-w-sm space-y-3 rounded-xl border border-white/6 bg-neutral-900/60 p-4 text-left backdrop-blur-sm">
        <p className="text-xs leading-relaxed text-neutral-400">
          A minimal{' '}
          <a
            href="https://datatracker.ietf.org/doc/draft-ietf-moq-transport/"
            target="_blank"
            rel="noreferrer"
            className="text-blue-400 underline decoration-blue-400/30 underline-offset-2 transition-colors hover:text-blue-300"
          >
            MOQT
          </a>{' '}
          player built on the{' '}
          <a
            href={`https://github.com/${GITHUB_REPO}`}
            target="_blank"
            rel="noreferrer"
            className="text-blue-400 underline decoration-blue-400/30 underline-offset-2 transition-colors hover:text-blue-300"
          >
            MOQtail library
          </a>
          . The source for this player lives in{' '}
          <a
            href={`https://github.com/${GITHUB_REPO}/tree/main/apps/client-js`}
            target="_blank"
            rel="noreferrer"
            className="font-mono text-[11px] text-neutral-300 underline decoration-neutral-600 underline-offset-2 transition-colors hover:text-white"
          >
            /apps/client-js
          </a>{' '}
          in the MOQtail repository.
        </p>
        <p className="text-xs leading-relaxed text-neutral-400">
          Modify the <span className="font-medium text-neutral-300">Relay URL</span> and{' '}
          <span className="font-medium text-neutral-300">Namespace</span> in the sidebar to point to
          your own stream.
        </p>
        <p className="text-xs leading-relaxed text-neutral-400">
          Players with more advanced features are available in{' '}
          <a
            href="https://moqtail.dev/demo"
            target="_blank"
            rel="noreferrer"
            className="text-blue-400 underline decoration-blue-400/30 underline-offset-2 transition-colors hover:text-blue-300"
          >
            MOQtail demos
          </a>
          .
        </p>
        <div className="border-t border-white/6 pt-3">
          <a
            href={`https://github.com/${GITHUB_REPO}/issues`}
            target="_blank"
            rel="noreferrer"
            className="inline-flex items-center gap-1.5 text-xs text-neutral-500 transition-colors hover:text-neutral-300"
          >
            <LuCircleDot className="h-3.5 w-3.5 shrink-0" aria-hidden="true" />
            Found a problem? Open an issue on GitHub.
          </a>
        </div>
      </div>
    </div>
  );
}

export const VideoPlayer = forwardRef<
  HTMLVideoElement,
  {
    hasTracks: boolean;
    mode?: 'watch' | 'publish';
  }
>(({ hasTracks, mode = 'watch' }, videoRef) => {
  const showPublishIdle = mode === 'publish' && !hasTracks;
  const showWatchIdle = mode === 'watch' && !hasTracks;

  return (
    <main className="relative flex flex-1 flex-col items-center justify-center overflow-hidden bg-neutral-950 p-4 md:p-6">
      <video
        ref={videoRef}
        controls
        className={cn(
          'w-full overflow-hidden rounded-xl bg-black shadow-2xl shadow-black/60 transition-opacity duration-300',
          hasTracks && mode === 'watch' ? 'opacity-100' : 'pointer-events-none opacity-0',
        )}
        style={{ aspectRatio: '16/9' }}
      />
      {showPublishIdle && <PublishIdleCard />}
      {showWatchIdle && <WatchIdleCard />}
    </main>
  );
});

VideoPlayer.displayName = 'VideoPlayer';
