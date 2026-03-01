/**
 * Copyright 2025 The MOQtail Authors
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

import { Tuple } from '../common/tuple'
import { ByteBuffer, BaseByteBuffer, FrozenByteBuffer } from '../common/byte_buffer'
import { TrackNameError } from '../error/error'

export const MAX_NAMESPACE_TUPLE_COUNT = 32
export const MAX_FULL_TRACK_NAME_LENGTH = 4096

/**
 * Fully-qualified track identifier = hierarchical namespace (tuple) + leaf name bytes.
 *
 * Constraints enforced (throws {@link TrackNameError}):
 * - Namespace tuple field count: 1 .. {@link MAX_NAMESPACE_TUPLE_COUNT} (must not be empty).
 * - Total serialized length (namespace tuple + raw name bytes) less than or equals {@link MAX_FULL_TRACK_NAME_LENGTH} bytes.
 *
 * Namespace input may be:
 * - `string` path with segments separated by `/` (converted via {@link Tuple.fromUtf8Path}). Empty segments are preserved
 *   except leading/trailing slashes are treated as empty fields and will be rejected by the length check if result is 0.
 * - Existing {@link Tuple} instance.
 *
 * Name input may be:
 * - UTF-8 string (encoded)
 * - Raw `Uint8Array` (used directly)
 *
 * Instances are created via {@link FullTrackName.tryNew} (validates) or {@link FullTrackName.deserialize}.
 * Use {@link FullTrackName.serialize} for wire encoding and {@link FullTrackName.toString} for a human friendly
 * diagnostic format: `namespace/segments:hexname` (name rendered as lowercase hex, no 0x prefix).
 *
 * The string form is intended for logs/debug only; do not parse it for protocol operations.
 */
export class FullTrackName {
  private constructor(
    public readonly namespace: Tuple,
    public readonly name: Uint8Array,
  ) {}

  /**
   * Human-readable representation: `\<namespace path joined by '/'\>:\<name as lowercase hex\>`.
   * If the underlying {@link Tuple} exposes `toUtf8Path`, it's used; otherwise the raw fields are joined.
   * This is lossy only in the sense that name bytes are hex encoded; round-tripping requires serialization.
   */
  toString(): string {
    // Namespace as slash-separated UTF-8 path, name as hex
    const nsStr = this.namespace.toUtf8Path ? this.namespace.toUtf8Path() : Array.from(this.namespace.fields).join('/')
    const nameStr = Array.from(this.name)
      .map((b) => b.toString(16).padStart(2, '0'))
      .join('')
    return `${nsStr}:${nameStr}`
  }

  /**
   * Human-readable representation with UTF-8 decoded name: `\<namespace path\>/\<name as UTF-8\>`.
   * Useful for matching against plain string track paths like "watchparty/room-1/user-x/audio".
   */
  toUtf8String(): string {
    const nsStr = this.namespace.toUtf8Path ? this.namespace.toUtf8Path() : Array.from(this.namespace.fields).join('/')
    const nameStr = new TextDecoder().decode(this.name)
    return `${nsStr}/${nameStr}`
  }

  /**
   * Construct a validated full track name.
   *
   * Validation steps:
   * 1. Convert namespace string -\> {@link Tuple} (split on '/') if needed.
   * 2. Reject if namespace tuple field count is 0 or \> {@link MAX_NAMESPACE_TUPLE_COUNT}.
   * 3. Encode name string to UTF-8 if needed.
   * 4. Reject if total serialized length (namespace tuple + name bytes) \> {@link MAX_FULL_TRACK_NAME_LENGTH}.
   *
   * @throws :{@link TrackNameError} on any constraint violation.
   * @example
   * ```ts
   * const full = FullTrackName.tryNew('media/video', 'keyframe')
   * console.log(full.toString()) // media/video:6b65796672616d65
   * ```
   */
  static tryNew(namespace: string | Tuple, name: string | Uint8Array): FullTrackName {
    const nsTuple = typeof namespace === 'string' ? Tuple.fromUtf8Path(namespace) : namespace
    const nsCount = nsTuple.fields.length
    if (nsCount === 0 || nsCount > MAX_NAMESPACE_TUPLE_COUNT) {
      throw new TrackNameError(
        'FullTrackName::tryNew(nsCount)',
        `Namespace cannot be empty or cannot exceed ${MAX_NAMESPACE_TUPLE_COUNT} fields`,
      )
    }
    const nameBytes = typeof name === 'string' ? new TextEncoder().encode(name) : name
    const totalLen = nsTuple.serialize().toUint8Array().length + nameBytes.length
    if (totalLen > MAX_FULL_TRACK_NAME_LENGTH) {
      throw new TrackNameError(
        'FullTrackName::tryNew(totalLen)',
        `Total length cannot exceed ${MAX_FULL_TRACK_NAME_LENGTH}`,
      )
    }
    return new FullTrackName(nsTuple, nameBytes)
  }

  /**
   * Serialize to a frozen buffer: tuple (namespace) followed by length‑prefixed name bytes.
   * Consumers needing raw bytes should call `.toUint8Array()` on the returned {@link FrozenByteBuffer}.
   */
  serialize(): FrozenByteBuffer {
    const buf = new ByteBuffer()
    buf.putTuple(this.namespace)
    buf.putLengthPrefixedBytes(this.name)
    return buf.freeze()
  }

  /**
   * Parse a serialized full track name. Performs the same validations as {@link FullTrackName.tryNew}.
   * The provided buffer's read cursor advances accordingly.
   * @throws :{@link TrackNameError} if constraints are violated.
   */
  static deserialize(buf: BaseByteBuffer): FullTrackName {
    const namespace = buf.getTuple()
    const nsCount = namespace.fields.length
    if (nsCount === 0 || nsCount > MAX_NAMESPACE_TUPLE_COUNT) {
      throw new TrackNameError(
        'FullTrackName::deserialize(nsCount)',
        `Namespace cannot be empty or cannot exceed ${MAX_NAMESPACE_TUPLE_COUNT} fields`,
      )
    }
    const name = buf.getLengthPrefixedBytes()
    const totalLen = namespace.serialize().toUint8Array().length + name.length
    if (totalLen > MAX_FULL_TRACK_NAME_LENGTH) {
      throw new TrackNameError(
        'FullTrackName::deserialize(totalLen)',
        `Total length cannot exceed ${MAX_FULL_TRACK_NAME_LENGTH}`,
      )
    }
    return new FullTrackName(namespace, name)
  }
}
