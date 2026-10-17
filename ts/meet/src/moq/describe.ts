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

import {
  ControlMessage,
  ControlMessageType,
  FullTrackName,
  KeyValuePair,
  MessageParameterType,
  SetupOptionType,
  Tuple,
} from 'moqtail/model';

export type MessageDirection = 'sent' | 'received';

/** MOQT control messages, or the app's own signalling carried as objects on a MOQT track. */
export type LogEntryKind = 'control' | 'signal';

export type LogEntry = {
  id: number;
  kind: LogEntryKind;
  /** Wall-clock time the message was sent or received, in ms since the epoch. */
  at: number;
  direction: MessageDirection;
  name: string;
  fields: Array<[string, string]>;
  /** A signal this session sent, read back off the shared signalling track. */
  isEcho?: boolean;
};

const MAX_DEPTH = 4;
const MAX_HEX_BYTES = 32;
const utf8 = new TextDecoder('utf-8', { fatal: true });

/** The wire name of a message type: `SubscribeOk` → `SUBSCRIBE_OK`. */
function messageName(msg: ControlMessage): string {
  const type = msg.getType();
  if (type === ControlMessageType.GoAway) return 'GOAWAY';
  const name = ControlMessageType[type] ?? `0x${type.toString(16)}`;
  return name.replace(/([a-z0-9])([A-Z])/g, '$1_$2').toUpperCase();
}

/** Bytes as a quoted string when they are printable text, hex otherwise. */
function formatBytes(bytes: Uint8Array): string {
  try {
    const text = utf8.decode(bytes);
    if (/^[\x20-\x7e]*$/.test(text)) return JSON.stringify(text);
  } catch {
    // Not UTF-8: fall through to hex.
  }
  const hex = Array.from(bytes.subarray(0, MAX_HEX_BYTES), b => b.toString(16).padStart(2, '0'));
  return `0x${hex.join('')}${bytes.length > MAX_HEX_BYTES ? '…' : ''}`;
}

/** A track as `/namespace/path : "name"`, the same way the message log shows it. */
export function trackLabel(fullTrackName: FullTrackName): string {
  return `${fullTrackName.namespace.toUtf8Path()} : ${formatBytes(fullTrackName.name)}`;
}

function formatValue(value: unknown, depth = 0): string {
  if (value === undefined || value === null) return '—';
  if (typeof value === 'bigint' || typeof value === 'number' || typeof value === 'boolean')
    return String(value);
  if (typeof value === 'string') return JSON.stringify(value);
  if (value instanceof Uint8Array) return formatBytes(value);
  if (value instanceof Tuple) return value.toUtf8Path() || '/';
  if (value instanceof FullTrackName) return trackLabel(value);
  if (value instanceof KeyValuePair) {
    const type = Number(value.typeValue);
    return `${SetupOptionType[type] ?? `0x${type.toString(16)}`}=${formatValue(value.value, depth + 1)}`;
  }
  if (depth >= MAX_DEPTH) return '…';
  if (Array.isArray(value)) return `[${value.map(v => formatValue(v, depth + 1)).join(', ')}]`;
  if (typeof value === 'object') {
    const entries = Object.entries(value).filter(([, v]) => typeof v !== 'function');
    // Message parameters carry their wire type on the class.
    const type = (value.constructor as { TYPE?: number }).TYPE;
    const label =
      type === undefined ? '' : (MessageParameterType[type] ?? `0x${type.toString(16)}`);
    if (label && entries.length === 1) return `${label}=${formatValue(entries[0]![1], depth + 1)}`;
    const body = entries.map(([k, v]) => `${k}: ${formatValue(v, depth + 1)}`).join(', ');
    return `${label}{${body}}`;
  }
  return String(value);
}

let nextId = 0;

export function toLogEntry(msg: ControlMessage, direction: MessageDirection): LogEntry {
  const fields = Object.entries(msg)
    .filter(([, v]) => typeof v !== 'function')
    .map(([k, v]): [string, string] => [k, formatValue(v)]);
  return {
    id: nextId++,
    kind: 'control',
    at: Date.now(),
    direction,
    name: messageName(msg),
    fields,
  };
}

/**
 * Splits a signal into its parts. The formats are the ones signalling.ts writes:
 * `{sender}:{senderToken}:join` and `{sender}:{receiver}:{receiverToken}:{welcome|duplicate_user}`.
 */
export function toSignalEntry(
  text: string,
  direction: MessageDirection,
  group: bigint,
  self: { username: string; token: string },
): LogEntry {
  const parts = text.split(':');
  const type = parts[parts.length - 1];
  let name = 'SIGNAL';
  let fields: Array<[string, string]> = [['text', JSON.stringify(text)]];
  let isEcho = false;

  if (type === 'join' && parts.length === 3) {
    const [from, token] = parts as [string, string];
    name = 'JOIN';
    fields = [
      ['from', from],
      ['token', token],
    ];
    isEcho = direction === 'received' && from === self.username && token === self.token;
  } else if ((type === 'welcome' || type === 'duplicate_user') && parts.length === 4) {
    const [from, to, token] = parts as [string, string, string];
    name = type.toUpperCase();
    fields = [
      ['from', from],
      ['to', to],
      ['token', token],
    ];
    isEcho = direction === 'received' && from === self.username;
  }
  fields.push(['group', String(group)]);
  return { id: nextId++, kind: 'signal', at: Date.now(), direction, name, fields, isEcho };
}

export function formatTime(at: number): string {
  const d = new Date(at);
  const pad = (n: number, w = 2) => String(n).padStart(w, '0');
  return `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}.${pad(d.getMilliseconds(), 3)}`;
}
