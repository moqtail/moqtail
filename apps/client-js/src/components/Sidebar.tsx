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

import { useState, useRef, useEffect } from 'preact/hooks';
import type { ComponentChildren } from 'preact';
import { cn } from '@/lib/utils';
import type { Track, Status, Presets } from '@/types';
import presets from '@/presets.json';

const inputCls =
  'w-full rounded-lg bg-neutral-900 border border-neutral-700/80 px-3 py-2 text-sm text-neutral-100 placeholder:text-neutral-600 focus:outline-none focus:border-blue-500 focus:ring-2 focus:ring-blue-500/20 transition-all';

function PresetDropdown({
  presets,
  relayUrl,
  namespace,
  onSelect,
}: {
  presets: Presets;
  relayUrl: string;
  namespace: string;
  onSelect: (relayUrl: string, ns: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement | null>(null);
  const buttonRef = useRef<HTMLButtonElement | null>(null);
  const [menuStyle, setMenuStyle] = useState<{
    top: number;
    left: number;
    width: number;
    maxHeight: number;
  } | null>(null);

  const allOptions = presets.relays.flatMap(relay => relay.namespaces.map(ns => ({ relay, ns })));
  const selected =
    allOptions.find(o => o.relay.url === relayUrl && o.ns.name === namespace) ?? null;

  function updateMenuPosition() {
    const button = buttonRef.current;
    if (!button) return;

    const rect = button.getBoundingClientRect();
    const margin = 8;
    const gap = 4;
    const idealMenuHeight = 320;
    const minMenuHeight = 140;

    const spaceBelow = window.innerHeight - rect.bottom - margin;
    const spaceAbove = rect.top - margin;
    const openUp = spaceBelow < 220 && spaceAbove > spaceBelow;
    const available = openUp ? spaceAbove : spaceBelow;
    const maxHeight = Math.max(minMenuHeight, Math.min(idealMenuHeight, available));

    const width = Math.min(rect.width, window.innerWidth - margin * 2);
    const left = Math.max(margin, Math.min(rect.left, window.innerWidth - width - margin));
    const top = openUp ? rect.top - maxHeight - gap : rect.bottom + gap;

    setMenuStyle({ top, left, width, maxHeight });
  }

  useEffect(() => {
    if (!open) return;

    updateMenuPosition();

    function handleOutside(e: MouseEvent) {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false);
    }
    function handleLayoutChange() {
      updateMenuPosition();
    }

    document.addEventListener('mousedown', handleOutside);
    window.addEventListener('resize', handleLayoutChange);
    window.addEventListener('scroll', handleLayoutChange, true);
    return () => {
      document.removeEventListener('mousedown', handleOutside);
      window.removeEventListener('resize', handleLayoutChange);
      window.removeEventListener('scroll', handleLayoutChange, true);
    };
  }, [open]);

  return (
    <div ref={ref} className="relative">
      <button
        ref={buttonRef}
        type="button"
        onClick={() => {
          setOpen(o => !o);
        }}
        className={cn(
          inputCls,
          'flex cursor-pointer items-center justify-between gap-2 text-left',
          open && 'border-blue-500 ring-2 ring-blue-500/20',
        )}
      >
        <span className={selected ? 'text-neutral-100' : 'text-neutral-600'}>
          {selected ? selected.ns.label : '— select preset —'}
        </span>
        <svg
          className={cn(
            'h-4 w-4 shrink-0 text-neutral-500 transition-transform duration-150',
            open && 'rotate-180',
          )}
          viewBox="0 0 20 20"
          fill="currentColor"
          aria-hidden="true"
        >
          <path
            fillRule="evenodd"
            d="M5.22 8.22a.75.75 0 011.06 0L10 11.94l3.72-3.72a.75.75 0 111.06 1.06l-4.25 4.25a.75.75 0 01-1.06 0L5.22 9.28a.75.75 0 010-1.06z"
            clipRule="evenodd"
          />
        </svg>
      </button>

      {open && menuStyle && (
        <div
          className="fixed z-30 overflow-auto rounded-lg border border-neutral-700/80 bg-neutral-900 shadow-2xl shadow-black/60"
          style={{
            top: menuStyle.top,
            left: menuStyle.left,
            width: menuStyle.width,
            maxHeight: menuStyle.maxHeight,
          }}
        >
          {allOptions.map(({ relay, ns }) => {
            const isSelected = relay.url === relayUrl && ns.name === namespace;
            return (
              <button
                key={`${relay.url}|${ns.name}`}
                type="button"
                onClick={() => {
                  onSelect(relay.url, ns.name);
                  setOpen(false);
                }}
                className={cn(
                  'w-full cursor-pointer px-3 py-2.5 text-left transition-colors',
                  isSelected ? 'bg-blue-600/20' : 'hover:bg-neutral-800',
                )}
              >
                <div className="flex items-center justify-between gap-2">
                  <span className="text-sm font-medium text-neutral-100">{ns.label}</span>
                  <div className="flex shrink-0 items-center gap-1">
                    {ns.videoTracks > 0 && (
                      <span className="rounded border border-violet-500/30 bg-violet-500/15 px-1.5 py-0.5 font-mono text-[10px] text-violet-300">
                        {ns.videoTracks}V
                      </span>
                    )}
                    {ns.audioTracks > 0 && (
                      <span className="rounded border border-teal-500/30 bg-teal-500/15 px-1.5 py-0.5 font-mono text-[10px] text-teal-300">
                        {ns.audioTracks}A
                      </span>
                    )}
                  </div>
                </div>
                <div className="mt-0.5 truncate font-mono text-[10px] text-neutral-500">
                  {relay.url}
                  <br />
                  {ns.name}
                </div>
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
}

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

function Checkbox({
  checked,
  disabled,
  onChange,
}: {
  checked: boolean;
  disabled: boolean;
  onChange: (checked: boolean) => void;
}) {
  return (
    <span className="flex shrink-0 items-center">
      {/* Real input — kept 1 px so it remains in the accessibility tree without position:absolute */}
      <input
        type="checkbox"
        checked={checked}
        disabled={disabled}
        className="size-px overflow-hidden opacity-0"
        onChange={e => onChange((e.target as HTMLInputElement).checked)}
      />
      {/* Visual indicator */}
      <span
        aria-hidden="true"
        className={cn(
          'flex h-4 w-4 shrink-0 items-center justify-center rounded border transition-colors',
          checked ? 'border-blue-500 bg-blue-500' : 'border-neutral-600 bg-neutral-800/60',
        )}
      >
        {checked && (
          <svg
            className="h-2.5 w-2.5 text-white"
            viewBox="0 0 10 10"
            fill="none"
            stroke="currentColor"
            strokeWidth="2"
          >
            <polyline points="2,5 4,8 8,2" />
          </svg>
        )}
      </span>
    </span>
  );
}

function TrackRow({
  track,
  checked,
  disabled,
  onChange,
}: {
  track: Track;
  checked: boolean;
  disabled: boolean;
  onChange: (track: Track, checked: boolean) => void;
}) {
  const selectable = !disabled;

  return (
    <label
      className={cn(
        'group flex items-center gap-3 rounded-lg px-3 py-2.5 transition-all select-none',
        selectable
          ? checked
            ? 'cursor-pointer bg-blue-600/15 ring-1 ring-blue-500/40'
            : 'cursor-pointer hover:bg-neutral-800/70'
          : 'cursor-not-allowed opacity-30',
      )}
    >
      <Checkbox checked={checked} disabled={disabled} onChange={val => onChange(track, val)} />
      <span className="min-w-0 flex-1 truncate font-mono text-xs leading-5 text-neutral-200">
        {track.name}
      </span>
      <div className="flex shrink-0 items-center gap-2">
        {track.bitrate ? (
          <span className="text-[10px] text-neutral-500 tabular-nums">
            {Math.round(track.bitrate / 1000)} kbps
          </span>
        ) : null}
        {track.width && track.height ? (
          <span className="text-[10px] text-neutral-500 tabular-nums">
            {track.width}&#x00d7;{track.height}
          </span>
        ) : null}
        {track.codec ? (
          <span className="font-mono text-[10px] text-neutral-500">
            {track.codec.split('.')[0]}
          </span>
        ) : null}
      </div>
    </label>
  );
}

function TrackGroup({
  title,
  color,
  tracks,
  selectedVideo,
  selectedAudio,
  disabled,
  onChange,
}: {
  title: string;
  color: string;
  tracks: Track[];
  selectedVideo: string | null;
  selectedAudio: string | null;
  disabled: boolean;
  onChange: (track: Track, checked: boolean) => void;
}) {
  if (tracks.length === 0) return null;
  return (
    <div>
      <div className="mb-1 flex items-center gap-2 px-1">
        <span className={cn('h-1.5 w-1.5 rounded-full', color)} />
        <span className="text-[11px] font-semibold tracking-widest text-neutral-500 uppercase">
          {title}
        </span>
        <span className="text-[10px] text-neutral-600">({tracks.length})</span>
      </div>
      <div className="space-y-0.5">
        {tracks.map(track => {
          const isSelected = track.name === selectedVideo || track.name === selectedAudio;
          return (
            <TrackRow
              key={track.name}
              track={track}
              checked={isSelected}
              disabled={disabled}
              onChange={onChange}
            />
          );
        })}
      </div>
    </div>
  );
}

export function Sidebar({
  relayUrl,
  onRelayUrlChange,
  namespace,
  onNamespaceChange,
  status,
  tracks,
  selectedVideo,
  selectedAudio,
  onConnect,
  onTrackChange,
  error,
}: {
  relayUrl: string;
  onRelayUrlChange: (url: string) => void;
  namespace: string;
  onNamespaceChange: (ns: string) => void;
  status: Status;
  tracks: Track[];
  selectedVideo: string | null;
  selectedAudio: string | null;
  onConnect: () => void;
  onTrackChange: (track: Track, checked: boolean) => void;
  error: string | null;
}) {
  const isBusy = status === 'connecting' || status === 'restarting';
  const hasTracks = tracks.length > 0;
  const videoTracks = tracks.filter(t => t.role === 'video');
  const audioTracks = tracks.filter(t => t.role === 'audio');

  return (
    <aside className="order-last flex max-h-full flex-col overflow-auto border-t border-white/6 bg-neutral-950 md:order-first md:w-72 md:border-t-0 md:border-r">
      {/* Connection */}
      <form
        className="space-y-3 border-b border-white/6 p-4"
        onSubmit={e => {
          e.preventDefault();
          if (!isBusy && relayUrl && namespace) onConnect();
        }}
      >
        {presets.relays.length > 0 && (
          <Field label="Preset">
            <PresetDropdown
              presets={presets}
              relayUrl={relayUrl}
              namespace={namespace}
              onSelect={(url, ns) => {
                onRelayUrlChange(url);
                onNamespaceChange(ns);
              }}
            />
          </Field>
        )}
        <div className="grid grid-cols-2 gap-3 md:grid-cols-1">
          <Field label="Relay URL">
            <input
              type="url"
              value={relayUrl}
              onInput={e => onRelayUrlChange((e.target as HTMLInputElement).value)}
              placeholder="https://relay.example.com:443"
              class={inputCls}
            />
          </Field>
          <Field label="Namespace">
            <input
              type="text"
              value={namespace}
              onInput={e => onNamespaceChange((e.target as HTMLInputElement).value)}
              placeholder="org/channel"
              class={inputCls}
            />
          </Field>
        </div>
        <button
          type="submit"
          disabled={isBusy || !relayUrl || !namespace}
          className="w-full cursor-pointer rounded-lg bg-blue-600 px-3 py-2 text-sm font-medium transition-colors hover:bg-blue-500 active:bg-blue-700 disabled:cursor-not-allowed disabled:opacity-40"
        >
          {status === 'connecting' ? 'Connecting…' : 'Connect'}
        </button>
        {status === 'error' && error && (
          <p className="rounded-lg border border-red-500/20 bg-red-500/10 px-3 py-2 text-xs leading-relaxed text-red-400">
            {error}
          </p>
        )}
      </form>

      {/* Tracks */}
      {hasTracks && (
        <div className="space-y-4 p-3">
          <TrackGroup
            title="Video"
            color="bg-violet-400"
            tracks={videoTracks}
            selectedVideo={selectedVideo}
            selectedAudio={selectedAudio}
            disabled={isBusy}
            onChange={onTrackChange}
          />
          <TrackGroup
            title="Audio"
            color="bg-teal-400"
            tracks={audioTracks}
            selectedVideo={selectedVideo}
            selectedAudio={selectedAudio}
            disabled={isBusy}
            onChange={onTrackChange}
          />
        </div>
      )}
    </aside>
  );
}
