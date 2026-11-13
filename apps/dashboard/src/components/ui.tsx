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
    <section class="rounded-lg border border-zinc-200 bg-white p-4 dark:border-zinc-800 dark:bg-zinc-900">
      {(props.title || props.actions) && (
        <header class="mb-3 flex items-center justify-between gap-2">
          {props.title && (
            <h2 class="text-sm font-semibold text-zinc-700 dark:text-zinc-300">{props.title}</h2>
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
    <div class="rounded-lg border border-zinc-200 bg-white p-3 dark:border-zinc-800 dark:bg-zinc-900">
      <div class="text-xs text-zinc-500 dark:text-zinc-400">{props.label}</div>
      <div class="mt-1 text-xl font-semibold tabular-nums">{props.value}</div>
      {props.hint && <div class="mt-0.5 text-xs text-zinc-500">{props.hint}</div>}
    </div>
  );
}

export function Badge(props: {
  tone?: 'ok' | 'warn' | 'bad' | 'plain';
  children: ComponentChildren;
}) {
  const tones = {
    ok: 'bg-emerald-100 text-emerald-800 dark:bg-emerald-900/40 dark:text-emerald-300',
    warn: 'bg-amber-100 text-amber-800 dark:bg-amber-900/40 dark:text-amber-300',
    bad: 'bg-red-100 text-red-800 dark:bg-red-900/40 dark:text-red-300',
    plain: 'bg-zinc-100 text-zinc-700 dark:bg-zinc-800 dark:text-zinc-300',
  };
  return (
    <span class={`rounded px-1.5 py-0.5 text-xs font-medium ${tones[props.tone ?? 'plain']}`}>
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
        ? "Check the API address, that api_enable is on, and that api_cors_origins lists this page's origin."
        : error.code === 'token_required' || error.code === 'token_rejected'
          ? "Set the relay's token in Settings."
          : error.code === 'qlog_off'
            ? 'Set qlog_folder in the relay configuration to keep qlog traces.'
            : error.detail;
  return (
    <div
      role="alert"
      class="rounded-lg border border-red-300 bg-red-50 p-3 text-sm text-red-800 dark:border-red-900 dark:bg-red-950 dark:text-red-200"
    >
      <div class="font-medium">{error.code}</div>
      {hint && <div class="mt-1">{hint}</div>}
    </div>
  );
}

export function Loading() {
  return <div class="text-sm text-zinc-500">Loading…</div>;
}

/** A table that scrolls sideways on narrow screens. */
export function Table(props: { children: ComponentChildren }) {
  return (
    <div class="overflow-x-auto">
      <table class="w-full border-collapse text-left text-sm tabular-nums [&_td]:border-t [&_td]:border-zinc-100 [&_td]:px-2 [&_td]:py-1.5 dark:[&_td]:border-zinc-800 [&_th]:px-2 [&_th]:py-1.5 [&_th]:text-xs [&_th]:font-medium [&_th]:text-zinc-500">
        {props.children}
      </table>
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
      <button type="button" class="hover:underline" onClick={() => props.onSort(next)}>
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
}) {
  if (props.total <= props.limit) return null;
  const last = Math.min(props.offset + props.limit, props.total);
  return (
    <nav class="mt-2 flex items-center gap-2 text-sm" aria-label="Pages">
      <button
        type="button"
        class="rounded border px-2 disabled:opacity-40"
        disabled={props.offset === 0}
        onClick={() => props.onOffset(Math.max(0, props.offset - props.limit))}
      >
        Previous
      </button>
      <span>
        {props.offset + 1}–{last} of {props.total}
      </span>
      <button
        type="button"
        class="rounded border px-2 disabled:opacity-40"
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
    <dl class="grid grid-cols-[max-content_1fr] gap-x-4 gap-y-1 text-sm">
      {props.rows.map(([label, value]) => (
        <>
          <dt class="text-zinc-500">{label}</dt>
          <dd class="break-all tabular-nums">{value}</dd>
        </>
      ))}
    </dl>
  );
}
