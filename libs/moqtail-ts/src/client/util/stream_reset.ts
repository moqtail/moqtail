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

import { MOQtailError, StreamResetCode } from '../../model/error'

/**
 * @public
 * The peer reset this stream, or told us to stop sending on it, with a code.
 */
export class PeerStreamResetError extends MOQtailError {
  constructor(
    public context: string,
    public code: StreamResetCode,
  ) {
    super(`[${context}] peer reset the stream with ${StreamResetCode[code]} (0x${code.toString(16)})`)
  }
}

/**
 * Stand-in for `WebTransportError` on runtimes that do not define it. Carries the
 * same two fields so {@link streamResetCodeOf} reads either shape.
 */
class StreamResetError extends Error {
  readonly source = 'stream'
  constructor(readonly streamErrorCode: StreamResetCode) {
    super(`stream reset with code ${streamErrorCode}`)
    this.name = 'StreamResetError'
  }
}

/**
 * @public
 * The reason to hand `writer.abort()` or `reader.cancel()` so the stream is torn down
 * with `code`.
 *
 * WebTransport carries a numeric RESET_STREAM / STOP_SENDING code only when the reason
 * is a `WebTransportError` with `source: 'stream'`; any other value — a string, say —
 * resets with 0, which the peer reads as INTERNAL_ERROR.
 */
export function streamResetReason(code: StreamResetCode): Error {
  const ctor = globalThis.WebTransportError as unknown as (new (...args: unknown[]) => Error) | undefined
  const message = `stream reset with code ${code}`

  if (typeof ctor === 'function') {
    const candidates: Array<() => Error> = [
      () => new ctor({ streamErrorCode: code, message }),
      () => new ctor(message, { source: 'stream', streamErrorCode: code }),
    ]
    for (const build of candidates) {
      try {
        const error = build()
        if (streamResetCodeOf(error) === code) return error
      } catch {
        // Wrong signature for this runtime; try the next.
      }
    }
  }
  return new StreamResetError(code)
}

/**
 * @public
 * The code the peer reset a stream with, or `undefined` if `error` is not a stream
 * reset or carries a code this draft does not define.
 */
export function streamResetCodeOf(error: unknown): StreamResetCode | undefined {
  const code = (error as { streamErrorCode?: unknown } | null | undefined)?.streamErrorCode
  if (typeof code !== 'number') return undefined
  try {
    return StreamResetCode.tryFrom(code)
  } catch {
    return undefined
  }
}

/**
 * @public
 * Re-types a failed read: a peer reset becomes a {@link PeerStreamResetError} naming
 * the code, anything else is passed through untouched.
 */
export function asStreamResetError(context: string, error: unknown): unknown {
  const code = streamResetCodeOf(error)
  return code === undefined ? error : new PeerStreamResetError(context, code)
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest

  describe('streamResetReason', () => {
    test('round-trips a code through the reason it builds', () => {
      const reason = streamResetReason(StreamResetCode.DeliveryTimeout)
      expect((reason as { streamErrorCode?: number }).streamErrorCode).toBe(0x2)
      expect(streamResetCodeOf(reason)).toBe(StreamResetCode.DeliveryTimeout)
    })

    const withWebTransportError = (ctor: unknown, fn: () => void) => {
      const original = globalThis.WebTransportError
      globalThis.WebTransportError = ctor as typeof globalThis.WebTransportError
      try {
        fn()
      } finally {
        globalThis.WebTransportError = original
      }
    }

    test('uses the spec constructor, which takes a single init dictionary', () => {
      class SpecWebTransportError extends Error {
        readonly source = 'stream'
        readonly streamErrorCode: number | null
        constructor(init: { streamErrorCode?: number; message?: string } = {}) {
          if (typeof init !== 'object' || init === null)
            throw new TypeError("Failed to construct 'WebTransportError': not a WebTransportErrorInit")
          super(init.message ?? '')
          this.streamErrorCode = init.streamErrorCode ?? null
        }
      }
      withWebTransportError(SpecWebTransportError, () => {
        const reason = streamResetReason(StreamResetCode.TooFarBehind)
        expect(reason).toBeInstanceOf(SpecWebTransportError)
        expect(streamResetCodeOf(reason)).toBe(StreamResetCode.TooFarBehind)
      })
    })

    test('falls back to the legacy (message, options) constructor', () => {
      class LegacyWebTransportError extends Error {
        readonly source: string
        readonly streamErrorCode: number | null
        constructor(message: string, options: { source?: string; streamErrorCode?: number | null } = {}) {
          if (typeof message !== 'string') throw new TypeError('message must be a string')
          super(message)
          this.source = options.source ?? 'stream'
          this.streamErrorCode = options.streamErrorCode ?? null
        }
      }
      withWebTransportError(LegacyWebTransportError, () => {
        const reason = streamResetReason(StreamResetCode.TooFarBehind)
        expect(reason).toBeInstanceOf(LegacyWebTransportError)
        expect(streamResetCodeOf(reason)).toBe(StreamResetCode.TooFarBehind)
      })
    })

    test('rejects a constructor that drops the code, rather than resetting with 0', () => {
      class LossyWebTransportError extends Error {
        readonly streamErrorCode: number | null = null
      }
      withWebTransportError(LossyWebTransportError, () => {
        expect(streamResetCodeOf(streamResetReason(StreamResetCode.TooFarBehind))).toBe(StreamResetCode.TooFarBehind)
      })
    })
  })

  describe('streamResetCodeOf', () => {
    test('reads a code off a peer WebTransportError shape', () => {
      expect(streamResetCodeOf({ source: 'stream', streamErrorCode: 0x4 })).toBe(StreamResetCode.GoingAway)
    })

    test('returns undefined for a non-reset error or an unknown code', () => {
      expect(streamResetCodeOf(new Error('boom'))).toBeUndefined()
      expect(streamResetCodeOf(undefined)).toBeUndefined()
      expect(streamResetCodeOf({ streamErrorCode: 0x8 })).toBeUndefined()
    })
  })
}
