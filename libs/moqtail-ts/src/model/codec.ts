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

/**
 * Wire codecs: one read and one write function per field shape. Messages and parameter
 * lists are tables of these, so every value is decoded once, straight into its type.
 */

import { ByteBuffer, FrozenByteBuffer } from './common/byte_buffer'
import type { BaseByteBuffer } from './common/byte_buffer'
import { Location } from './common/location'
import { KeyValuePair } from './common/pair'
import { Tuple } from './common/tuple'
import { FullTrackName } from './data/full_track_name'
import { ProtocolViolationError } from './error/error'

/** @public */
export type Codec<T> = readonly [read: (buf: BaseByteBuffer) => T, write: (buf: ByteBuffer, value: T) => void]

/** One row of a KVP list: its type, the key it decodes to, and how its value is coded. */
export type Entry = readonly [type: number, key: string, codec: Codec<any>, repeated?: 'repeated']

const MAX_REASON_PHRASE_LENGTH = 1024
const utf8 = new TextEncoder()
const fromUtf8 = new TextDecoder('utf-8', { fatal: true })

export const varint: Codec<bigint> = [(buf) => buf.getVI(), (buf, v) => buf.putVI(v)]

export const byte: Codec<number> = [(buf) => buf.getU8(), (buf, v) => buf.putU8(v)]

export const flag: Codec<boolean> = [
  (buf) => {
    const v = buf.getU8()
    if (v > 1) throw new ProtocolViolationError('codec', `flag ${v} is not 0 or 1`)

    return v === 1
  },
  (buf, v) => buf.putU8(v ? 1 : 0),
]

export const bytes: Codec<Uint8Array> = [
  (buf) => buf.getLengthPrefixedBytes(),
  (buf, v) => buf.putLengthPrefixedBytes(v),
]

export const text: Codec<string> = boundedText(Infinity)

/** A reason phrase is UTF-8 of at most 1024 bytes (moqt-22 §1.4.3). */
export const reason: Codec<string> = boundedText(MAX_REASON_PHRASE_LENGTH)

export const tuple: Codec<Tuple> = [(buf) => buf.getTuple(), (buf, v) => buf.putTuple(v)]

export const fullTrackName: Codec<FullTrackName> = [
  (buf) => buf.getFullTrackName(),
  (buf, v) => buf.putFullTrackName(v),
]

export const location: Codec<Location> = [(buf) => buf.getLocation(), (buf, v) => buf.putLocation(v)]

/** A value carried inside a length-prefixed byte string, which it must fill exactly. */
export function wrapped<T>([read, write]: Codec<T>): Codec<T> {
  return [
    (buf) => {
      const inner = new FrozenByteBuffer(buf.getLengthPrefixedBytes())
      const value = read(inner)

      if (inner.remaining > 0) throw new ProtocolViolationError('codec', 'trailing bytes in a parameter value')

      return value
    },
    (buf, v) => {
      const inner = new ByteBuffer()
      write(inner, v)
      buf.putLengthPrefixedBytes(inner.toUint8Array())
    },
  ]
}

/**
 * A KVP list with delta-coded types (moqt-22 §1.4.2), decoded into an object keyed by
 * `entries`. It is count-prefixed, or runs to the end of the message for SETUP options. An
 * unknown type is skipped by its parity: even types carry a varint, odd ones a
 * length-prefixed value; GREASE is one more unknown type (§14). With `extensions`, unknown
 * types are kept under that key as raw pairs instead, and written back. `allowed` limits
 * which types may appear at all.
 */
export function kvps<T extends object>(
  entries: readonly Entry[],
  framing: 'counted' | 'toEnd',
  { allowed, extensions }: { allowed?: readonly number[]; extensions?: string } = {},
): Codec<T> {
  const byType = new Map(entries.map((entry) => [entry[0], entry]))
  const check = (type: number) => {
    if (allowed && !allowed.includes(type)) {
      throw new ProtocolViolationError('codec', `parameter 0x${type.toString(16)} is not allowed here`)
    }
  }

  return [
    (buf) => {
      const out: Record<string, unknown> = {}
      const count = framing === 'counted' ? buf.getNumberVI() : Infinity
      let type = 0n

      for (let i = 0; i < count && (framing === 'counted' || buf.remaining > 0); i++) {
        type += buf.getVI()
        const entry = byType.get(Number(type))
        if (!entry) {
          const pair = KeyValuePair.deserializeValue(buf, type)

          if (extensions) ((out[extensions] ??= []) as KeyValuePair[]).push(pair)
          continue
        }

        check(entry[0])
        const value = entry[2][0](buf)
        if (entry[3]) ((out[entry[1]] ??= []) as unknown[]).push(value)
        else out[entry[1]] = value
      }

      return out as T
    },
    (buf, v) => {
      const items: [type: bigint, write: (buf: ByteBuffer, value: any) => void, value: unknown][] = []

      for (const [type, key, codec, repeated] of entries) {
        const value = (v as Record<string, unknown>)[key]
        if (value === undefined) continue

        check(type)

        for (const item of repeated ? (value as unknown[]) : [value]) items.push([BigInt(type), codec[1], item])
      }

      if (extensions) {
        for (const pair of ((v as Record<string, unknown>)[extensions] ?? []) as KeyValuePair[]) {
          items.push([pair.typeValue, putPairValue, pair])
        }

        items.sort((a, b) => (a[0] < b[0] ? -1 : a[0] > b[0] ? 1 : 0))
      }

      if (framing === 'counted') buf.putVI(items.length)

      let previous = 0n

      for (const [type, write, item] of items) {
        buf.putVI(type - previous)
        write(buf, item)
        previous = type
      }
    },
  ]
}

function putPairValue(buf: ByteBuffer, pair: KeyValuePair): void {
  if (typeof pair.value === 'bigint') buf.putVI(pair.value)
  else buf.putLengthPrefixedBytes(pair.value)
}

/** Length-prefixed UTF-8 of at most `max` bytes. */
export function boundedText(max: number): Codec<string> {
  const check = (encoded: Uint8Array) => {
    if (encoded.length > max) throw new ProtocolViolationError('codec', `text over ${max} bytes`)

    return encoded
  }

  return [
    (buf) => {
      const encoded = check(buf.getLengthPrefixedBytes())

      try {
        return fromUtf8.decode(encoded)
      } catch {
        throw new ProtocolViolationError('codec', 'invalid UTF-8')
      }
    },
    (buf, v) => buf.putLengthPrefixedBytes(check(utf8.encode(v))),
  ]
}
