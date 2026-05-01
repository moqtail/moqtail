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
import type { Status } from '@/types';

const GITHUB_REPO = 'moqtail/moqtail';

const STATUS_CONFIG: Record<Status, { dot: string; label: string }> = {
  idle: { dot: 'bg-neutral-600', label: 'Idle' },
  connecting: { dot: 'bg-yellow-400 animate-pulse', label: 'Connecting…' },
  ready: { dot: 'bg-emerald-400', label: 'Catalog loaded' },
  restarting: { dot: 'bg-yellow-400 animate-pulse', label: 'Starting…' },
  playing: { dot: 'bg-blue-400', label: 'Playing' },
  error: { dot: 'bg-red-400', label: 'Error' },
};

function GitHubIcon() {
  return (
    <svg height="18" viewBox="0 0 16 16" width="18" fill="currentColor" aria-hidden="true">
      <path d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.013 8.013 0 0016 8c0-4.42-3.58-8-8-8z" />
    </svg>
  );
}

function StatusDot({ status }: { status: Status }) {
  const { dot, label } = STATUS_CONFIG[status];
  return (
    <span className="flex items-center gap-1.5 text-xs text-neutral-400 select-none">
      <span className={cn('h-2 w-2 rounded-full', dot)} />
      {label}
    </span>
  );
}

export function Header({ status }: { status: Status }) {
  return (
    <header className="flex h-12 shrink-0 items-center justify-between border-b border-white/6 bg-neutral-950/80 px-4 backdrop-blur-sm md:px-5">
      <div className="flex items-center gap-2.5">
        <img src="/favicon.svg" alt="MOQtail logo" className="h-5 w-5" />
        <a
          href="https://moqtail.dev"
          target="_blank"
          rel="noreferrer"
          className="text-sm font-semibold tracking-tight transition-colors hover:text-neutral-300"
        >
          MOQtail Player
        </a>
        <span className="text-neutral-700 select-none">·</span>
        <StatusDot status={status} />
      </div>
      <div className="flex items-center gap-2 md:gap-3">
        {(import.meta.env.VITE_BUILD_COMMIT || import.meta.env.VITE_BUILD_DATE) && (
          <span className="hidden font-mono text-[10px] text-neutral-600 tabular-nums select-none sm:block">
            {[import.meta.env.VITE_BUILD_COMMIT, import.meta.env.VITE_BUILD_DATE]
              .filter(Boolean)
              .join(' · ')}
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
  );
}
