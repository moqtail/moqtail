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

// Number and time formatting for tables and tiles.

const BYTE_UNITS = ['B', 'KB', 'MB', 'GB', 'TB'];

export function bytes(n: number): string {
  let value = n;
  let unit = 0;
  while (value >= 1000 && unit < BYTE_UNITS.length - 1) {
    value /= 1000;
    unit += 1;
  }
  return `${value < 10 && unit > 0 ? value.toFixed(1) : Math.round(value)} ${BYTE_UNITS[unit]}`;
}

export function count(n: number): string {
  return n.toLocaleString('en-US');
}

/** Milliseconds, with the precision that matters at that size. */
export function ms(value: number | null | undefined): string {
  if (value === null || value === undefined) return '–';
  if (value < 1) return `${value.toFixed(2)} ms`;
  if (value < 100) return `${value.toFixed(1)} ms`;
  return `${Math.round(value)} ms`;
}

export function rate(perSecond: number, unit: 'objects' | 'bytes'): string {
  return unit === 'bytes' ? `${bytes(perSecond)}/s` : `${count(Math.round(perSecond))}/s`;
}

/** How long ago a time in milliseconds since the epoch was, such as `3m 20s ago`. */
export function ago(atMs: number, now = Date.now()): string {
  const seconds = Math.max(0, Math.round((now - atMs) / 1000));
  return `${duration(seconds)} ago`;
}

export function duration(totalSeconds: number): string {
  const s = Math.floor(totalSeconds);
  const days = Math.floor(s / 86400);
  const hours = Math.floor((s % 86400) / 3600);
  const minutes = Math.floor((s % 3600) / 60);
  const seconds = s % 60;
  if (days) return `${days}d ${hours}h`;
  if (hours) return `${hours}h ${minutes}m`;
  if (minutes) return `${minutes}m ${seconds}s`;
  return `${seconds}s`;
}

export function clock(atMs: number): string {
  return new Date(atMs).toLocaleTimeString('en-GB', { hour12: false });
}
