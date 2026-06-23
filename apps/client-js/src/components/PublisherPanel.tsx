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

import { useState } from 'preact/hooks';
import type { ComponentChildren } from 'preact';
import { cn } from '@/lib/utils';
import type { SourceKind, SourceState, PublishStatus } from '@/types';
import {
  LuCamera,
  LuMonitor,
  LuTerminal,
  LuCheck,
  LuCopy,
  LuExternalLink,
  LuRefreshCw,
  LuLoader,
} from 'react-icons/lu';

const inputCls =
  'w-full rounded-lg bg-neutral-900 border border-neutral-700/80 px-3 py-2 text-sm text-neutral-100 placeholder:text-neutral-600 focus:outline-none focus:border-blue-500 focus:ring-2 focus:ring-blue-500/20 transition-all';

// ─── Small reuse helpers ──────────────────────────────────────────────────────

function Field({ label, children }: { label: string; children: ComponentChildren }) {
  return (
    <div className="space-y-1">
      <label className="block text-[11px] font-semibold tracking-widest text-neutral-500 uppercase">
        {label}
      </label>
      {children}
    </div>
  );
}

function Tooltip({ text, children }: { text: string; children: ComponentChildren }) {
  const [visible, setVisible] = useState(false);
  return (
    <div
      className="relative inline-flex"
      onMouseEnter={() => setVisible(true)}
      onMouseLeave={() => setVisible(false)}
    >
      {children}
      {visible && (
        <div className="pointer-events-none absolute bottom-full left-1/2 z-50 mb-2 -translate-x-1/2 rounded-md border border-neutral-700 bg-neutral-900 px-2.5 py-1.5 text-[11px] leading-snug whitespace-nowrap text-neutral-300 shadow-lg">
          {text}
        </div>
      )}
    </div>
  );
}

// ─── Source row ───────────────────────────────────────────────────────────────

const SOURCE_META: Record<SourceKind, { label: string; icon: ComponentChildren }> = {
  camera: {
    label: 'Camera',
    icon: <LuCamera className="h-4 w-4" />,
  },
  screen: {
    label: 'Screen Share',
    icon: <LuMonitor className="h-4 w-4" />,
  },
  test: {
    label: 'Test Source',
    icon: <LuTerminal className="h-4 w-4" />,
  },
};

function SourceRow({
  source,
  disabled,
  onToggle,
  onTimestampToggle,
}: {
  source: SourceState;
  disabled: boolean;
  onToggle: (kind: SourceKind, enabled: boolean) => void;
  onTimestampToggle: (kind: SourceKind, embed: boolean) => void;
}) {
  const meta = SOURCE_META[source.kind];
  const unavailable = !source.available;
  const isDisabled = disabled || unavailable;

  const row = (
    <div
      className={cn(
        'group flex items-center gap-3 rounded-lg px-3 py-2.5 transition-all select-none',
        isDisabled
          ? cn(
              'cursor-not-allowed opacity-35',
              source.enabled && 'bg-blue-600/10 ring-1 ring-blue-500/30',
            )
          : source.enabled
            ? 'bg-blue-600/10 ring-1 ring-blue-500/30'
            : 'cursor-pointer hover:bg-neutral-800/60',
      )}
      onClick={() => !isDisabled && onToggle(source.kind, !source.enabled)}
    >
      {/* Checkbox visual */}
      <span
        className={cn(
          'flex h-4 w-4 shrink-0 items-center justify-center rounded border transition-colors',
          source.enabled ? 'border-blue-500 bg-blue-500' : 'border-neutral-600 bg-neutral-800/60',
        )}
        aria-hidden="true"
      >
        {source.enabled && <LuCheck className="h-2.5 w-2.5 text-white" aria-hidden="true" />}
      </span>

      {/* Icon */}
      <span className={cn('shrink-0', source.enabled ? 'text-blue-400' : 'text-neutral-500')}>
        {meta.icon}
      </span>

      {/* Label */}
      <span className="flex-1 text-sm text-neutral-200">{meta.label}</span>

      {/* Test source: always-on timestamp badge */}
      {source.kind === 'test' && (
        <span className="shrink-0 rounded border border-blue-500/30 bg-blue-500/10 px-1.5 py-0.5 font-mono text-[10px] text-blue-400">
          +ts
        </span>
      )}
    </div>
  );

  const showTimestampToggle =
    source.enabled && !isDisabled && (source.kind === 'camera' || source.kind === 'screen');

  return (
    <div>
      {unavailable ? (
        <Tooltip text={source.unavailableReason ?? 'Not available'}>{row}</Tooltip>
      ) : (
        row
      )}
      {showTimestampToggle && (
        <div
          className="mt-0.5 ml-10 flex cursor-pointer items-center gap-2 rounded-md px-2 py-1.5 text-[11px] text-neutral-500 transition-colors hover:bg-neutral-800/40 hover:text-neutral-400"
          onClick={e => {
            e.stopPropagation();
            onTimestampToggle(source.kind, !source.embedTimestamp);
          }}
        >
          <span
            className={cn(
              'inline-flex h-3.5 w-3.5 shrink-0 items-center justify-center rounded border transition-colors',
              source.embedTimestamp
                ? 'border-teal-500 bg-teal-500'
                : 'border-neutral-600 bg-neutral-800/60',
            )}
          >
            {source.embedTimestamp && <LuCheck className="h-2 w-2 text-white" aria-hidden="true" />}
          </span>
          Embed timestamp
        </div>
      )}
    </div>
  );
}

// ─── Watch URL row ────────────────────────────────────────────────────────────

function WatchUrlRow({ url }: { url: string }) {
  const [copied, setCopied] = useState(false);

  const handleCopy = () => {
    navigator.clipboard.writeText(url).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    });
  };

  return (
    <div className="flex items-stretch overflow-hidden rounded-lg border border-teal-500/20 bg-teal-500/5 text-xs">
      <span className="flex min-w-0 flex-1 items-center truncate px-3 py-2 font-mono text-teal-300/70">
        {url || '—'}
      </span>
      <div className="flex shrink-0 border-l border-teal-500/20">
        <button
          type="button"
          onClick={handleCopy}
          title="Copy watch URL"
          className="flex items-center gap-1.5 px-3 py-2 text-teal-400 transition-colors hover:bg-teal-500/10 hover:text-teal-300"
        >
          {copied ? <LuCheck className="h-3.5 w-3.5" /> : <LuCopy className="h-3.5 w-3.5" />}
          <span className="text-[10px] font-medium">{copied ? 'Copied' : 'Copy'}</span>
        </button>
        <button
          type="button"
          onClick={() => url && window.open(url, '_blank')}
          title="Open in new tab"
          disabled={!url}
          className="border-l border-teal-500/20 px-2.5 text-teal-400 transition-colors hover:bg-teal-500/10 hover:text-teal-300 disabled:opacity-30"
        >
          <LuExternalLink className="h-3.5 w-3.5" />
        </button>
      </div>
    </div>
  );
}

// ─── Re-roll icon ─────────────────────────────────────────────────────────────

function RerollButton({ onClick }: { onClick: () => void }) {
  return (
    <button
      type="button"
      onClick={onClick}
      title="Generate new namespace"
      className="flex h-9 w-9 shrink-0 cursor-pointer items-center justify-center rounded-lg border border-neutral-700/80 bg-neutral-900 text-neutral-400 transition-colors hover:border-neutral-600 hover:text-neutral-200"
    >
      <LuRefreshCw className="h-3.5 w-3.5" />
    </button>
  );
}

// ─── Main component ───────────────────────────────────────────────────────────

export interface PublisherPanelProps {
  relayUrl: string;
  onRelayUrlChange: (url: string) => void;
  namespace: string;
  onNamespaceChange: (ns: string) => void;
  onRefreshNamespace: () => void;
  sources: SourceState[];
  onSourceToggle: (kind: SourceKind, enabled: boolean) => void;
  onTimestampToggle: (kind: SourceKind, embed: boolean) => void;
  publishStatus: PublishStatus;
  onPublish: () => void;
  onStop: () => void;
  error: string | null;
  watchUrl: string;
}

export function PublisherPanel({
  relayUrl,
  onRelayUrlChange,
  namespace,
  onNamespaceChange,
  onRefreshNamespace,
  sources,
  onSourceToggle,
  onTimestampToggle,
  publishStatus,
  onPublish,
  onStop,
  error,
  watchUrl,
}: PublisherPanelProps) {
  const isConnecting = publishStatus === 'connecting';
  const isPublishing = publishStatus === 'publishing';
  const isActive = isConnecting || isPublishing;
  const hasActiveSource = sources.some(s => s.enabled);

  return (
    <div className="flex flex-col gap-4 overflow-auto p-4">
      {/* Relay URL */}
      <Field label="Relay URL">
        <input
          type="url"
          value={relayUrl}
          onInput={e => onRelayUrlChange((e.target as HTMLInputElement).value)}
          placeholder="https://relay.example.com:443"
          disabled={isActive}
          className={inputCls}
        />
      </Field>

      {/* Namespace */}
      <Field label="Namespace">
        <div className="flex gap-2">
          <input
            type="text"
            value={namespace}
            onInput={e => onNamespaceChange((e.target as HTMLInputElement).value)}
            placeholder="moqtail/blue-fox"
            disabled={isActive}
            className={cn(inputCls, 'flex-1')}
          />
          <RerollButton onClick={onRefreshNamespace} />
        </div>
      </Field>

      {/* Sources */}
      <Field label="Sources">
        <div className="space-y-1.5">
          {sources.map(source => (
            <SourceRow
              key={source.kind}
              source={source}
              disabled={isActive}
              onToggle={onSourceToggle}
              onTimestampToggle={onTimestampToggle}
            />
          ))}
        </div>
      </Field>

      {/* Go Live / Stop */}
      <button
        type="button"
        onClick={isActive ? onStop : onPublish}
        disabled={isConnecting || (!isActive && !hasActiveSource)}
        className={cn(
          'flex w-full items-center justify-center gap-2 rounded-lg px-3 py-2 text-sm font-medium transition-colors disabled:cursor-not-allowed disabled:opacity-40',
          isActive
            ? 'cursor-pointer bg-red-600 hover:bg-red-500 active:bg-red-700'
            : 'cursor-pointer bg-green-600 hover:bg-green-500 active:bg-green-700',
          isConnecting && 'cursor-not-allowed',
        )}
      >
        {isConnecting && <LuLoader className="h-4 w-4 animate-spin" aria-hidden="true" />}
        {isConnecting ? 'Connecting…' : isPublishing ? 'Stop' : 'Go Live'}
      </button>

      {/* Watch URL (only when publishing) */}
      {isPublishing && (
        <div className="space-y-1.5">
          <p className="text-[10px] font-semibold tracking-widest text-neutral-600 uppercase">
            Watch URL
          </p>
          <WatchUrlRow url={watchUrl} />
        </div>
      )}

      {/* Error */}
      {publishStatus === 'error' && error && (
        <p className="rounded-lg border border-red-500/20 bg-red-500/10 px-3 py-2 text-xs leading-relaxed text-red-400">
          {error}
        </p>
      )}
    </div>
  );
}
