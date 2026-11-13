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

import type { ComponentChildren } from 'preact';
import type { ApiError } from '../api/client';

export function Card(props: {
  title?: string;
  actions?: ComponentChildren;
  children: ComponentChildren;
}) {
  return (
    <section class="rounded-xl border border-white/6 bg-neutral-900/60 p-4">
      {(props.title || props.actions) && (
        <header class="mb-3 flex items-center justify-between gap-2">
          {props.title && (
            <h2 class="text-[11px] font-semibold tracking-widest text-neutral-500 uppercase">
              {props.title}
            </h2>
          )}
          {props.actions}
        </header>
      )}
      {props.children}
    </section>
  );
}

export function Stat(props: { label: string; value: ComponentChildren; hint?: string }) {
  return (
    <div class="rounded-xl border border-white/6 bg-neutral-900/60 p-3">
      <div class="text-[11px] font-semibold tracking-widest text-neutral-500 uppercase">
        {props.label}
      </div>
      <div class="mt-1 font-mono text-xl font-semibold text-neutral-100 tabular-nums">
        {props.value}
      </div>
      {props.hint && <div class="mt-0.5 text-[11px] text-neutral-500">{props.hint}</div>}
    </div>
  );
}

export function Badge(props: {
  tone?: 'ok' | 'warn' | 'bad' | 'plain';
  children: ComponentChildren;
}) {
  const tones = {
    ok: 'border-emerald-500/30 bg-emerald-500/15 text-emerald-300',
    warn: 'border-amber-500/30 bg-amber-500/15 text-amber-300',
    bad: 'border-red-500/30 bg-red-500/15 text-red-300',
    plain: 'border-neutral-500/30 bg-neutral-500/15 text-neutral-300',
  };
  return (
    <span
      class={`rounded border px-1.5 py-0.5 font-mono text-[10px] ${tones[props.tone ?? 'plain']}`}
    >
      {props.children}
    </span>
  );
}

/** Explains an API error and, where possible, how to fix it. */
export function ErrorNote(props: { error: ApiError }) {
  const { error } = props;
  const hint =
    error.code === 'feature_off'
      ? (error.detail ?? 'This part of the API is turned off on the relay.')
      : error.code === 'unreachable'
        ? "Check the API address, that api.enable is on, and that api.cors_origins lists this page's origin."
        : error.code === 'token_required' || error.code === 'token_rejected'
          ? "Set the relay's token in Settings."
          : error.code === 'qlog_off'
            ? 'Set qlog_folder in the relay configuration to keep qlog traces.'
            : error.detail;
  return (
    <div
      role="alert"
      class="rounded-lg border border-red-500/20 bg-red-500/10 p-3 text-xs text-red-300"
    >
      <div class="font-mono font-medium">{error.code}</div>
      {hint && <div class="mt-1 text-red-300/80">{hint}</div>}
    </div>
  );
}

export function Loading() {
  return <div class="text-xs text-neutral-500">Loading…</div>;
}

/** A table that scrolls sideways on narrow screens. */
export function Table(props: { children: ComponentChildren }) {
  return (
    <div class="overflow-x-auto">
      <table class="w-full border-collapse text-left">{props.children}</table>
    </div>
  );
}

/** A column header that asks the relay to sort by `key`; clicking again reverses. */
export function SortHeader(props: {
  label: string;
  sortKey: string;
  sort: string;
  onSort: (sort: string) => void;
}) {
  const active = props.sort.replace(/^-/, '') === props.sortKey;
  const descending = props.sort.startsWith('-');
  const next = active && descending ? props.sortKey : `-${props.sortKey}`;
  return (
    <th aria-sort={active ? (descending ? 'descending' : 'ascending') : 'none'}>
      <button
        type="button"
        class="cursor-pointer uppercase transition-colors hover:text-neutral-300"
        onClick={() => props.onSort(next)}
      >
        {props.label}
        {active ? (descending ? ' ↓' : ' ↑') : ''}
      </button>
    </th>
  );
}

export function Pager(props: {
  total: number;
  offset: number;
  limit: number;
  onOffset: (offset: number) => void;
  /** What the list holds, such as `open connections`. */
  noun?: string;
}) {
  if (props.total === 0) return null;
  const last = Math.min(props.offset + props.limit, props.total);
  const noun = props.noun ? ` ${props.noun}` : '';
  // One page: just the count.
  if (props.total <= props.limit) {
    return (
      <p class="mt-3 text-xs text-neutral-500">
        {props.total.toLocaleString('en-US')}
        {noun}
      </p>
    );
  }
  return (
    <nav class="mt-3 flex items-center gap-2 text-xs text-neutral-400" aria-label="Pages">
      <button
        type="button"
        class="btn"
        disabled={props.offset === 0}
        onClick={() => props.onOffset(Math.max(0, props.offset - props.limit))}
      >
        Previous
      </button>
      <span>
        {props.offset + 1}–{last} of {props.total.toLocaleString('en-US')}
        {noun}
      </span>
      <button
        type="button"
        class="btn"
        disabled={last >= props.total}
        onClick={() => props.onOffset(props.offset + props.limit)}
      >
        Next
      </button>
    </nav>
  );
}

/** A labelled key–value list. */
export function Facts(props: { rows: [string, ComponentChildren][] }) {
  return (
    <dl class="grid grid-cols-[max-content_1fr] gap-x-4 gap-y-1.5 text-xs">
      {props.rows.map(([label, value]) => (
        <>
          <dt class="text-neutral-500">{label}</dt>
          <dd class="font-mono break-all text-neutral-300 tabular-nums">{value}</dd>
        </>
      ))}
    </dl>
  );
}

/** The IP of `host:port` or `[v6]:port`. */
export function remoteIp(remote: string): string {
  const v6 = /^\[([^\]]+)\]/.exec(remote);
  if (v6) return v6[1];
  const colons = remote.split(':').length - 1;
  return colons === 1 ? remote.slice(0, remote.indexOf(':')) : remote;
}

/** A remote address, its IP linked to its ipinfo.io page. */
export function Remote(props: { remote?: string | null }) {
  if (!props.remote) return <>–</>;
  const ip = remoteIp(props.remote);
  return (
    <a class="link" href={`https://ipinfo.io/${ip}`} target="_blank" rel="noreferrer">
      {props.remote}
    </a>
  );
}
