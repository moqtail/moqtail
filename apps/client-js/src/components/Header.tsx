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
  ready: { dot: 'bg-emerald-400', label: 'Ready' },
  playing: { dot: 'bg-blue-400', label: 'Playing' },
  paused: { dot: 'bg-neutral-400', label: 'Paused' },
  stalled: { dot: 'bg-yellow-400 animate-pulse', label: 'Buffering…' },
  reconnecting: { dot: 'bg-yellow-400 animate-pulse', label: 'Reconnecting…' },
  ended: { dot: 'bg-neutral-400', label: 'Ended' },
  error: { dot: 'bg-red-400', label: 'Error' },
  destroyed: { dot: 'bg-neutral-600', label: 'Idle' },
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

function AdvancedToggle({ on, onChange }: { on: boolean; onChange: (on: boolean) => void }) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={on}
      onClick={() => onChange(!on)}
      className="hidden cursor-pointer items-center gap-2 text-xs font-medium text-neutral-400 transition-colors select-none hover:text-neutral-100 md:flex"
    >
      Advanced
      <span
        aria-hidden="true"
        className={cn(
          'relative h-4 w-7 rounded-full transition-colors duration-200',
          on ? 'bg-blue-500' : 'bg-neutral-700',
        )}
      >
        <span
          className={cn(
            'absolute top-0.5 left-0.5 h-3 w-3 rounded-full bg-white shadow transition-transform duration-200',
            on && 'translate-x-3',
          )}
        />
      </span>
    </button>
  );
}

export function Header({
  status,
  advanced,
  onAdvancedChange,
}: {
  status: Status;
  advanced: boolean;
  onAdvancedChange: (on: boolean) => void;
}) {
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
      <div className="flex items-center gap-2 md:gap-4">
        {(import.meta.env.VITE_BUILD_COMMIT || import.meta.env.VITE_BUILD_DATE) && (
          <span className="hidden font-mono text-[10px] text-neutral-600 tabular-nums select-none sm:block">
            {[import.meta.env.VITE_BUILD_COMMIT, import.meta.env.VITE_BUILD_DATE]
              .filter(Boolean)
              .join(' · ')}
          </span>
        )}
        <AdvancedToggle on={advanced} onChange={onAdvancedChange} />
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
