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

import { BaseByteBuffer, ByteBuffer, FrozenByteBuffer } from './byte_buffer'
import { Tuple } from './tuple'

/**
 * @public
 * Directs the peer to retry a request at a different URI and/or for a different Full
 * Track Name (§10.6.1). Carried by a REQUEST_ERROR whose code is `REDIRECT`.
 */
export class Redirect {
  /** Undefined means reuse the current session's URI. */
  readonly connectUri: string | undefined

  constructor(
    connectUri: string | undefined,
    /** Empty, together with an empty {@link Redirect.trackName}, means reuse the original request's track. */
    public readonly trackNamespace: Tuple,
    /** Empty for namespace-scoped requests, which have no track name to redirect. */
    public readonly trackName: Uint8Array,
  ) {
    this.connectUri = connectUri ? connectUri : undefined
  }

  /** True when the redirect names no track of its own, so the original request's track carries over. */
  get keepsOriginalTrack(): boolean {
    return this.trackNamespace.fields.length === 0 && this.trackName.length === 0
  }

  serialize(): FrozenByteBuffer {
    const buf = new ByteBuffer()
    if (this.connectUri) {
      buf.putLengthPrefixedBytes(new TextEncoder().encode(this.connectUri))
    } else {
      buf.putVI(0)
    }
    buf.putTuple(this.trackNamespace)
    buf.putLengthPrefixedBytes(this.trackName)
    return buf.freeze()
  }

  static deserialize(buf: BaseByteBuffer): Redirect {
    const uriBytes = buf.getLengthPrefixedBytes()
    const connectUri = uriBytes.length === 0 ? undefined : new TextDecoder().decode(uriBytes)
    const trackNamespace = buf.getTuple()
    const trackName = buf.getLengthPrefixedBytes()
    return new Redirect(connectUri, trackNamespace, trackName)
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest

  describe('Redirect', () => {
    test('round-trips a URI and a track', () => {
      const redirect = new Redirect(
        'moqt://other.example',
        Tuple.fromUtf8Path('room1/audio'),
        new TextEncoder().encode('track-9'),
      )
      const frozen = redirect.serialize()
      const parsed = Redirect.deserialize(frozen)

      expect(parsed.connectUri).toBe('moqt://other.example')
      expect(parsed.trackNamespace.equals(redirect.trackNamespace)).toBe(true)
      expect(parsed.trackName).toEqual(redirect.trackName)
      expect(parsed.keepsOriginalTrack).toBe(false)
      expect(frozen.remaining).toBe(0)
    })

    test('round-trips an empty URI and an empty track', () => {
      const redirect = new Redirect('', new Tuple(), new Uint8Array())
      expect(redirect.connectUri).toBeUndefined()

      const frozen = redirect.serialize()
      const parsed = Redirect.deserialize(frozen)

      expect(parsed.connectUri).toBeUndefined()
      expect(parsed.trackNamespace.fields.length).toBe(0)
      expect(parsed.trackName.length).toBe(0)
      // Both lengths zero: the redirected request reuses the original track (§10.6.1).
      expect(parsed.keepsOriginalTrack).toBe(true)
      expect(frozen.remaining).toBe(0)
    })
  })
}
