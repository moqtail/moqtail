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
import { FaGithub } from 'react-icons/fa';

const GITHUB_REPO = 'moqtail/moqtail';

const STATUS_CONFIG: Record<Status, { dot: string; label: string }> = {
  idle: { dot: 'bg-neutral-600', label: 'Idle' },
  connecting: { dot: 'bg-yellow-400 animate-pulse', label: 'Connecting…' },
  ready: { dot: 'bg-emerald-400', label: 'Catalog loaded' },
  restarting: { dot: 'bg-yellow-400 animate-pulse', label: 'Starting…' },
  playing: { dot: 'bg-blue-400', label: 'Playing' },
  error: { dot: 'bg-red-400', label: 'Error' },
};

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
          <FaGithub size={18} aria-hidden="true" />
          <span className="hidden text-xs font-medium sm:block">{GITHUB_REPO}</span>
        </a>
      </div>
    </header>
  );
}
