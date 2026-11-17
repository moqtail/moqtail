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
 * QMux wire format: QUIC v1 frames, QUIC variable-length integers and transport
 * parameters, carried in records. Over WebSocket one binary message is one record.
 * Mirrors `libs/moqtail-rs/src/transport/qmux/codec.rs`.
 */

/** QUIC transport error codes QMux reuses. */
export const QmuxErrorCode = {
  FLOW_CONTROL_ERROR: 0x3,
  STREAM_LIMIT_ERROR: 0x4,
  STREAM_STATE_ERROR: 0x5,
  FINAL_SIZE_ERROR: 0x6,
  FRAME_ENCODING_ERROR: 0x7,
  TRANSPORT_PARAMETER_ERROR: 0x8,
  PROTOCOL_VIOLATION: 0xa,
} as const

// These codepoints are provisional and expected to change before the format is final;
// they live only here so a change is one edit. They exceed 2^53, hence bigint.
const QX_TRANSPORT_PARAMETERS = 0x3f5153300d0a0d0an
const QX_PING_REQUEST = 0x348c67529ef8c7bdn
const QX_PING_RESPONSE = 0x348c67529ef8c7ben
const TP_MAX_RECORD_SIZE = 0x0571c59429cd0845n

/** Record size every peer must accept. */
export const DEFAULT_MAX_RECORD_SIZE = 16382
/** Bytes a STREAM frame header takes at most: type, id, offset, length. */
export const STREAM_HEADER_MAX = 1 + 8 + 8 + 8

/** A connection error: the QUIC error code to close with, and why. */
export class QmuxProtocolError extends Error {
  constructor(
    readonly code: number,
    reason: string,
  ) {
    super(reason)
    this.name = 'QmuxProtocolError'
  }
}

const encoding = (reason: string) => new QmuxProtocolError(QmuxErrorCode.FRAME_ENCODING_ERROR, reason)

export function varintLength(v: number | bigint): number {
  const n = BigInt(v)
  if (n < 64n) return 1
  if (n < 16384n) return 2
  if (n < 1073741824n) return 4
  return 8
}

/** Appends QUIC variable-length integers to a growing byte list. */
export class Writer {
  #buf = new Uint8Array(256)
  #len = 0

  get length(): number {
    return this.#len
  }

  #reserve(n: number): void {
    if (this.#len + n <= this.#buf.length) return
    let size = this.#buf.length * 2
    while (size < this.#len + n) size *= 2
    const next = new Uint8Array(size)
    next.set(this.#buf.subarray(0, this.#len))
    this.#buf = next
  }

  byte(b: number): void {
    this.#reserve(1)
    this.#buf[this.#len++] = b
  }

  bytes(data: Uint8Array): void {
    this.#reserve(data.length)
    this.#buf.set(data, this.#len)
    this.#len += data.length
  }

  varint(v: number | bigint): void {
    // Fast path for the common case; bigint only for large values and frame types.
    if (typeof v === 'number' && v < 1073741824) {
      if (v < 64) return this.byte(v)
      this.#reserve(4)
      if (v < 16384) {
        this.#buf[this.#len++] = 0x40 | (v >> 8)
        this.#buf[this.#len++] = v & 0xff
        return
      }
      this.#buf[this.#len++] = 0x80 | (v >>> 24)
      this.#buf[this.#len++] = (v >>> 16) & 0xff
      this.#buf[this.#len++] = (v >>> 8) & 0xff
      this.#buf[this.#len++] = v & 0xff
      return
    }
    const n = BigInt(v)
    const len = varintLength(n)
    const prefix = BigInt([0, 0, 1, 0, 2, 0, 0, 0, 3][len]!) << BigInt(len * 8 - 2)
    const value = n | prefix
    this.#reserve(len)
    for (let i = len - 1; i >= 0; i--) this.#buf[this.#len + len - 1 - i] = Number((value >> BigInt(i * 8)) & 0xffn)
    this.#len += len
  }

  finish(): Uint8Array {
    return this.#buf.slice(0, this.#len)
  }
}

class Reader {
  #pos = 0
  constructor(readonly buf: Uint8Array) {}

  get done(): boolean {
    return this.#pos >= this.buf.length
  }

  bigVarint(): bigint {
    const first = this.buf[this.#pos]
    if (first === undefined) throw encoding('truncated varint')
    const len = 1 << (first >> 6)
    if (this.#pos + len > this.buf.length) throw encoding('truncated varint')
    let v = BigInt(first & 0x3f)
    for (let i = 1; i < len; i++) v = (v << 8n) | BigInt(this.buf[this.#pos + i]!)
    this.#pos += len
    return v
  }

  /** A varint that must fit a JavaScript number (ids, offsets, sizes). */
  varint(): number {
    const v = this.bigVarint()
    if (v > BigInt(Number.MAX_SAFE_INTEGER)) throw encoding('value too large')
    return Number(v)
  }

  take(len: number): Uint8Array {
    if (this.#pos + len > this.buf.length) throw encoding('frame runs past the end of the record')
    const out = this.buf.subarray(this.#pos, this.#pos + len)
    this.#pos += len
    return out
  }

  rest(): Uint8Array {
    return this.take(this.buf.length - this.#pos)
  }
}

/** The transport parameters QMux carries. Absent ones take QUIC's default of 0. */
export type TransportParams = {
  maxIdleTimeoutMs: number
  initialMaxData: number
  initialMaxStreamDataBidiLocal: number
  initialMaxStreamDataBidiRemote: number
  initialMaxStreamDataUni: number
  initialMaxStreamsBidi: number
  initialMaxStreamsUni: number
  /** 0: the peer does not accept datagrams. */
  maxDatagramFrameSize: number
  maxRecordSize: number
}

export const defaultTransportParams = (): TransportParams => ({
  maxIdleTimeoutMs: 0,
  initialMaxData: 0,
  initialMaxStreamDataBidiLocal: 0,
  initialMaxStreamDataBidiRemote: 0,
  initialMaxStreamDataUni: 0,
  initialMaxStreamsBidi: 0,
  initialMaxStreamsUni: 0,
  maxDatagramFrameSize: 0,
  maxRecordSize: DEFAULT_MAX_RECORD_SIZE,
})

const PARAM_IDS: Array<[bigint, keyof TransportParams]> = [
  [0x01n, 'maxIdleTimeoutMs'],
  [0x04n, 'initialMaxData'],
  [0x05n, 'initialMaxStreamDataBidiLocal'],
  [0x06n, 'initialMaxStreamDataBidiRemote'],
  [0x07n, 'initialMaxStreamDataUni'],
  [0x08n, 'initialMaxStreamsBidi'],
  [0x09n, 'initialMaxStreamsUni'],
  [0x20n, 'maxDatagramFrameSize'],
  [TP_MAX_RECORD_SIZE, 'maxRecordSize'],
]

/** QUIC parameters that make no sense without QUIC packets; receiving one is an error. */
const PROHIBITED_PARAMS = new Set([0x00n, 0x02n, 0x03n, 0x0an, 0x0bn, 0x0cn, 0x0dn, 0x0en, 0x0fn, 0x10n])

function encodeParams(params: TransportParams, w: Writer): void {
  const defaults = defaultTransportParams()
  for (const [id, key] of PARAM_IDS) {
    const value = params[key]
    if (value === defaults[key]) continue
    w.varint(id)
    w.varint(varintLength(value))
    w.varint(value)
  }
}

function decodeParams(body: Uint8Array): TransportParams {
  const bad = (reason: string) => new QmuxProtocolError(QmuxErrorCode.TRANSPORT_PARAMETER_ERROR, reason)
  const r = new Reader(body)
  const params = defaultTransportParams()
  const seen = new Set<bigint>()
  while (!r.done) {
    let id: bigint, value: Uint8Array
    try {
      id = r.bigVarint()
      value = r.take(r.varint())
    } catch {
      throw bad('truncated parameter')
    }
    if (PROHIBITED_PARAMS.has(id)) throw bad(`parameter 0x${id.toString(16)} is not allowed`)
    const known = PARAM_IDS.find(([known]) => known === id)
    if (!known) continue
    if (seen.has(id)) throw bad(`parameter 0x${id.toString(16)} repeated`)
    seen.add(id)
    const vr = new Reader(value)
    let v: number
    try {
      v = vr.varint()
    } catch {
      throw bad('bad parameter value')
    }
    if (!vr.done) throw bad('parameter value longer than its varint')
    params[known[1]] = v
  }
  if (params.initialMaxStreamsBidi > 2 ** 60 || params.initialMaxStreamsUni > 2 ** 60)
    throw bad('stream limit above 2^60')
  if (params.maxRecordSize < DEFAULT_MAX_RECORD_SIZE) throw bad('max_record_size below the minimum')
  return params
}

export type Frame =
  | { type: 'padding' }
  | { type: 'reset_stream'; id: number; code: number; finalSize: number }
  | { type: 'stop_sending'; id: number; code: number }
  | { type: 'stream'; id: number; offset: number; data: Uint8Array; fin: boolean }
  | { type: 'max_data'; max: number }
  | { type: 'max_stream_data'; id: number; max: number }
  | { type: 'max_streams'; bidi: boolean; max: number }
  | { type: 'data_blocked'; limit: number }
  | { type: 'stream_data_blocked'; id: number; limit: number }
  | { type: 'streams_blocked'; bidi: boolean; limit: number }
  /** `app` false is the transport variant, which also names the offending frame type. */
  | { type: 'connection_close'; app: boolean; code: number; frameType: number; reason: Uint8Array }
  | { type: 'datagram'; data: Uint8Array }
  | { type: 'transport_parameters'; params: TransportParams }
  | { type: 'ping_request'; seq: number }
  | { type: 'ping_response'; seq: number }

export function encodeFrame(frame: Frame, w: Writer): void {
  switch (frame.type) {
    case 'padding':
      return w.byte(0x00)
    case 'reset_stream':
      w.byte(0x04)
      w.varint(frame.id)
      w.varint(frame.code)
      return w.varint(frame.finalSize)
    case 'stop_sending':
      w.byte(0x05)
      w.varint(frame.id)
      return w.varint(frame.code)
    case 'stream':
      // Always with OFF and LEN, so a record can hold several frames.
      w.byte(0x08 | 0x04 | 0x02 | (frame.fin ? 1 : 0))
      w.varint(frame.id)
      w.varint(frame.offset)
      w.varint(frame.data.length)
      return w.bytes(frame.data)
    case 'max_data':
      w.byte(0x10)
      return w.varint(frame.max)
    case 'max_stream_data':
      w.byte(0x11)
      w.varint(frame.id)
      return w.varint(frame.max)
    case 'max_streams':
      w.byte(frame.bidi ? 0x12 : 0x13)
      return w.varint(frame.max)
    case 'data_blocked':
      w.byte(0x14)
      return w.varint(frame.limit)
    case 'stream_data_blocked':
      w.byte(0x15)
      w.varint(frame.id)
      return w.varint(frame.limit)
    case 'streams_blocked':
      w.byte(frame.bidi ? 0x16 : 0x17)
      return w.varint(frame.limit)
    case 'connection_close':
      w.byte(frame.app ? 0x1d : 0x1c)
      w.varint(frame.code)
      if (!frame.app) w.varint(frame.frameType)
      w.varint(frame.reason.length)
      return w.bytes(frame.reason)
    case 'datagram':
      w.byte(0x31)
      w.varint(frame.data.length)
      return w.bytes(frame.data)
    case 'transport_parameters': {
      const body = new Writer()
      encodeParams(frame.params, body)
      w.varint(QX_TRANSPORT_PARAMETERS)
      w.varint(body.length)
      return w.bytes(body.finish())
    }
    case 'ping_request':
      w.varint(QX_PING_REQUEST)
      return w.varint(frame.seq)
    case 'ping_response':
      w.varint(QX_PING_RESPONSE)
      return w.varint(frame.seq)
  }
}

/** Decodes every frame in one record. Payloads are views into `record`. */
export function decodeRecord(record: Uint8Array): Frame[] {
  const r = new Reader(record)
  const frames: Frame[] = []
  while (!r.done) frames.push(decodeFrame(r))
  return frames
}

function decodeFrame(r: Reader): Frame {
  const ty = r.bigVarint()
  if (ty === QX_TRANSPORT_PARAMETERS) return { type: 'transport_parameters', params: decodeParams(r.take(r.varint())) }
  if (ty === QX_PING_REQUEST) return { type: 'ping_request', seq: r.varint() }
  if (ty === QX_PING_RESPONSE) return { type: 'ping_response', seq: r.varint() }
  const t = Number(ty)
  switch (true) {
    case t === 0x00:
      return { type: 'padding' }
    case t === 0x04:
      return { type: 'reset_stream', id: r.varint(), code: r.varint(), finalSize: r.varint() }
    case t === 0x05:
      return { type: 'stop_sending', id: r.varint(), code: r.varint() }
    case t >= 0x08 && t <= 0x0f: {
      const id = r.varint()
      const offset = t & 0x04 ? r.varint() : 0
      const data = t & 0x02 ? r.take(r.varint()) : r.rest()
      return { type: 'stream', id, offset, data, fin: (t & 0x01) !== 0 }
    }
    case t === 0x10:
      return { type: 'max_data', max: r.varint() }
    case t === 0x11:
      return { type: 'max_stream_data', id: r.varint(), max: r.varint() }
    case t === 0x12 || t === 0x13:
      return { type: 'max_streams', bidi: t === 0x12, max: r.varint() }
    case t === 0x14:
      return { type: 'data_blocked', limit: r.varint() }
    case t === 0x15:
      return { type: 'stream_data_blocked', id: r.varint(), limit: r.varint() }
    case t === 0x16 || t === 0x17:
      return { type: 'streams_blocked', bidi: t === 0x16, limit: r.varint() }
    case t === 0x1c || t === 0x1d: {
      const app = t === 0x1d
      const code = r.varint()
      const frameType = app ? 0 : r.varint()
      return { type: 'connection_close', app, code, frameType, reason: r.take(r.varint()) }
    }
    case t === 0x30:
      return { type: 'datagram', data: r.rest() }
    case t === 0x31:
      return { type: 'datagram', data: r.take(r.varint()) }
    default:
      throw encoding(`frame type 0x${ty.toString(16)} is not allowed`)
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest

  const bytesOf = (frame: Frame) => {
    const w = new Writer()
    encodeFrame(frame, w)
    return w.finish()
  }

  describe('QMux codec', () => {
    test('varints match the QUIC specification examples', () => {
      const cases: Array<[bigint, number[]]> = [
        [151288809941952652n, [0xc2, 0x19, 0x7c, 0x5e, 0xff, 0x14, 0xe8, 0x8c]],
        [494878333n, [0x9d, 0x7f, 0x3e, 0x7d]],
        [15293n, [0x7b, 0xbd]],
        [37n, [0x25]],
      ]
      for (const [value, bytes] of cases) {
        const w = new Writer()
        w.varint(value)
        expect([...w.finish()]).toEqual(bytes)
        expect(new Reader(Uint8Array.from(bytes)).bigVarint()).toBe(value)
      }
    })

    test('every frame round-trips', () => {
      const frames: Frame[] = [
        { type: 'padding' },
        { type: 'reset_stream', id: 6, code: 0x2, finalSize: 1000 },
        { type: 'stop_sending', id: 4, code: 0x10 },
        { type: 'stream', id: 2, offset: 70000, data: new TextEncoder().encode('hello'), fin: true },
        { type: 'max_data', max: 1 << 24 },
        { type: 'max_stream_data', id: 3, max: 1 << 20 },
        { type: 'max_streams', bidi: true, max: 100 },
        { type: 'max_streams', bidi: false, max: 1000 },
        { type: 'data_blocked', limit: 5 },
        { type: 'stream_data_blocked', id: 1, limit: 9 },
        { type: 'streams_blocked', bidi: false, limit: 7 },
        { type: 'connection_close', app: true, code: 0x10, frameType: 0, reason: new TextEncoder().encode('bye') },
        { type: 'connection_close', app: false, code: 0xa, frameType: 0x08, reason: new Uint8Array() },
        { type: 'datagram', data: new TextEncoder().encode('dgram') },
        {
          type: 'transport_parameters',
          params: {
            ...defaultTransportParams(),
            maxIdleTimeoutMs: 30000,
            initialMaxData: 1 << 24,
            initialMaxStreamsUni: 1000,
            maxDatagramFrameSize: DEFAULT_MAX_RECORD_SIZE,
          },
        },
        { type: 'ping_request', seq: 3 },
        { type: 'ping_response', seq: 3 },
      ]
      for (const frame of frames) expect(decodeRecord(bytesOf(frame))).toEqual([frame])
    })

    // The same bytes the Rust codec's tests pin, so both sides agree on the wire.
    test('golden bytes shared with the Rust codec', () => {
      const params = bytesOf({ type: 'transport_parameters', params: defaultTransportParams() })
      expect(new TextDecoder('latin1').decode(params.subarray(0, 8))).toBe('\xffQS0\r\n\r\n')
      expect(params[8]).toBe(0)
      // STREAM with OFF, no LEN, FIN: id 0, offset 0, "abc" to the end of the record.
      expect(decodeRecord(Uint8Array.from([0x0d, 0x00, 0x00, 0x61, 0x62, 0x63]))).toEqual([
        { type: 'stream', id: 0, offset: 0, data: new TextEncoder().encode('abc'), fin: true },
      ])
    })

    test('forbidden and truncated frames are encoding errors', () => {
      expect(() => decodeRecord(Uint8Array.from([0x02, 0, 0, 0, 0]))).toThrow(QmuxProtocolError)
      expect(() => decodeRecord(Uint8Array.from([0x0a, 0x00, 0x05, 0x61, 0x62]))).toThrow(/past the end/)
    })

    test('prohibited and too-small parameters are rejected, unknown ones skipped', () => {
      expect(() => decodeParams(Uint8Array.from([0x02, 0x01, 0x00]))).toThrow(/not allowed/)
      const w = new Writer()
      w.varint(TP_MAX_RECORD_SIZE)
      w.varint(2)
      w.varint(1000)
      expect(() => decodeParams(w.finish())).toThrow(/below the minimum/)
      expect(decodeParams(Uint8Array.from([0x3f, 0x01, 0x07]))).toEqual(defaultTransportParams())
    })
  })
}
