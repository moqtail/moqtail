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

const GITHUB_REPO = 'moqtail/moqtail';

export const VideoPlayer = forwardRef<
  HTMLVideoElement,
  {
    hasTracks: boolean;
  }
>(({ hasTracks }, videoRef) => {
  return (
    <main className="relative flex flex-1 flex-col items-center justify-center overflow-hidden bg-neutral-950 p-4 md:p-6">
      <video
        ref={videoRef}
        controls
        className={cn(
          'w-full overflow-hidden rounded-xl bg-black shadow-2xl shadow-black/60 transition-opacity duration-300',
          hasTracks ? 'opacity-100' : 'pointer-events-none opacity-0',
        )}
        style={{ aspectRatio: '16/9' }}
      />
      {!hasTracks && (
        <div className="absolute inset-0 flex flex-col items-center justify-center gap-6 px-6 text-center select-none">
          {/* Icon */}
          <div className="flex h-16 w-16 items-center justify-center rounded-2xl border border-white/6 bg-neutral-900 text-neutral-600">
            <svg
              viewBox="0 0 24 24"
              className="h-8 w-8"
              fill="none"
              stroke="currentColor"
              strokeWidth="1.5"
            >
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                d="M5.25 5.653c0-.856.917-1.398 1.667-.986l11.54 6.347a1.125 1.125 0 010 1.972l-11.54 6.347c-.75.412-1.667-.13-1.667-.986V5.653z"
              />
            </svg>
          </div>

          {/* Heading */}
          <p className="text-sm font-medium text-neutral-300">
            Connect to a relay to start playback
          </p>

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
              <span className="font-medium text-neutral-300">Namespace</span> in the sidebar to
              point to your own stream.
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
                <svg
                  viewBox="0 0 16 16"
                  className="h-3.5 w-3.5 shrink-0"
                  fill="currentColor"
                  aria-hidden="true"
                >
                  <path d="M8 9.5a1.5 1.5 0 100-3 1.5 1.5 0 000 3z" />
                  <path d="M8 0a8 8 0 110 16A8 8 0 018 0zM1.5 8a6.5 6.5 0 1013 0 6.5 6.5 0 00-13 0z" />
                </svg>
                Found a problem? Open an issue on GitHub.
              </a>
            </div>
          </div>
        </div>
      )}
    </main>
  );
});

VideoPlayer.displayName = 'VideoPlayer';
