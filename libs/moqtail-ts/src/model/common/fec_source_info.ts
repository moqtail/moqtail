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

import { KeyValuePair, isBytes } from './pair'
import { ProtocolViolationError } from '../error/error'

/**
 * @public
 * MoQT object extension Type for FEC Source Info (draft-ramadan-moq-fec).
 *
 * The value is **odd** on purpose. Under the MoQT key-value parity rule an even
 * Type carries a single varint Value, so a length-prefixed even Type desyncs any
 * conformant parser or re-encoding relay on the first FEC-tagged object. Because
 * `0x11` is odd, {@link KeyValuePair} already reads it as a length-prefixed byte
 * string and this extension needs no parser special case.
 */
export const FEC_SOURCE_INFO_TYPE = 0x11n

/** FEC Source Info Value length. Exactly this; any other length is an error. */
export const FEC_SOURCE_INFO_LEN = 4

/** Maximum value of SS_ID (32 bits). */
export const FEC_U32_MAX = 0xffffffff

/**
 * @public
 * FEC Source Info object extension (draft-ramadan-moq-fec).
 *
 * Value layout: `SS_ID(u32 BE)`, exactly {@link FEC_SOURCE_INFO_LEN} bytes. SS_ID is
 * the source symbol's sequence number — the same value and definition the Source FEC
 * Payload ID trailer carries on the MMTP path, so one identifier covers both paths.
 *
 * The field is **fixed width, not a QUIC varint**, for two reasons. MoQT permits
 * non-minimal varint encodings, so a varint would vary the extension's own length
 * without varying its meaning. And the varint encoding is a property of the session,
 * which MoQT redefined at draft-17, so a varint would decode differently on two drafts.
 *
 * The byte layout is pinned by `vectors/fec-source-info/cases.json`, which is shared
 * with libmmt so both engines are checked against the same hand-authored bytes.
 */
export class FecSourceInfo {
  static readonly TYPE = FEC_SOURCE_INFO_TYPE

  constructor(
    /** Source symbol sequence number (32 bits). */
    public readonly ssId: number,
  ) {
    if (!Number.isInteger(ssId) || ssId < 0 || ssId > FEC_U32_MAX) {
      throw new ProtocolViolationError('FecSourceInfo', `ssId out of range: ${ssId}`)
    }
  }

  /** Encodes the extension Value bytes. */
  encode(): Uint8Array {
    const buf = new Uint8Array(FEC_SOURCE_INFO_LEN)
    new DataView(buf.buffer).setUint32(0, this.ssId, false)
    return buf
  }

  /**
   * Decodes the extension Value bytes. Fails closed on any length other than
   * {@link FEC_SOURCE_INFO_LEN}.
   */
  static decode(value: Uint8Array): FecSourceInfo {
    if (value.length !== FEC_SOURCE_INFO_LEN) {
      throw new ProtocolViolationError(
        'FecSourceInfo.decode',
        `Value must be ${FEC_SOURCE_INFO_LEN} bytes, got ${value.length}`,
      )
    }
    return new FecSourceInfo(new DataView(value.buffer, value.byteOffset, value.length).getUint32(0, false))
  }

  /** Wraps this extension as an object property, ready for any object's property list. */
  toKeyValuePair(): KeyValuePair {
    return KeyValuePair.tryNewBytes(FEC_SOURCE_INFO_TYPE, this.encode())
  }

  /**
   * Decodes an object property known to be FEC Source Info.
   * @throws ProtocolViolationError if the pair is not type {@link FEC_SOURCE_INFO_TYPE}
   *   or does not carry a byte-string Value.
   */
  static fromKeyValuePair(pair: KeyValuePair): FecSourceInfo {
    if (pair.typeValue !== FEC_SOURCE_INFO_TYPE) {
      throw new ProtocolViolationError(
        'FecSourceInfo.fromKeyValuePair',
        `expected type ${FEC_SOURCE_INFO_TYPE}, got ${pair.typeValue}`,
      )
    }
    if (!isBytes(pair)) {
      throw new ProtocolViolationError('FecSourceInfo.fromKeyValuePair', 'FEC Source Info Value must be a byte string')
    }
    return FecSourceInfo.decode(pair.value)
  }

  /**
   * Finds the FEC Source Info extension in an object's property list, or returns
   * `null` when the object carries none. Use on `SubgroupObject`, `Datagram` and
   * `FetchObject`, each of which carries `properties` as a plain KVP list.
   */
  static fromProperties(properties: readonly KeyValuePair[] | null | undefined): FecSourceInfo | null {
    const pair = properties?.find((p) => p.typeValue === FEC_SOURCE_INFO_TYPE)
    return pair ? FecSourceInfo.fromKeyValuePair(pair) : null
  }
}

if (import.meta.vitest) {
  const { describe, it, expect } = import.meta.vitest
  const { readFileSync } = await import('node:fs')

  const vectors = JSON.parse(
    readFileSync(new URL('../../../../../vectors/fec-source-info/cases.json', import.meta.url), 'utf8'),
  ) as {
    extensionType: number
    valueLength: number
    cases: { name: string; ssId: number; value: string }[]
    invalidValueLengths: number[]
  }

  const hex = (b: Uint8Array) => Array.from(b, (x) => x.toString(16).padStart(2, '0')).join('')

  describe('FecSourceInfo', () => {
    it('uses the codepoint the shared vectors pin', () => {
      expect(FEC_SOURCE_INFO_TYPE).toBe(BigInt(vectors.extensionType))
    })

    it('is an odd Type, so the parity rule alone gives it a byte-string Value', () => {
      expect(FEC_SOURCE_INFO_TYPE % 2n).toBe(1n)
    })

    it('uses the Value length the shared vectors pin', () => {
      expect(FEC_SOURCE_INFO_LEN).toBe(vectors.valueLength)
    })

    for (const c of vectors.cases) {
      it(`encodes ${c.name} to the pinned bytes`, () => {
        expect(hex(new FecSourceInfo(c.ssId).encode())).toBe(c.value)
      })

      it(`decodes ${c.name} back to its SS_ID`, () => {
        const bytes = Uint8Array.from(c.value.match(/../g)!.map((h) => parseInt(h, 16)))
        expect(FecSourceInfo.decode(bytes).ssId).toBe(c.ssId)
      })

      it(`round-trips ${c.name} through an object property list`, () => {
        const info = new FecSourceInfo(c.ssId)
        const found = FecSourceInfo.fromProperties([KeyValuePair.tryNewVarInt(0, 10), info.toKeyValuePair()])
        expect(found?.ssId).toBe(c.ssId)
      })
    }

    // 8 and 12 were the SBN/ESI[/ObjectLength] widths of the retired format version 1.
    // They are refused, not decoded: there is no compatibility shim.
    for (const len of vectors.invalidValueLengths) {
      it(`refuses a ${len}-byte Value`, () => {
        expect(() => FecSourceInfo.decode(new Uint8Array(len))).toThrow(ProtocolViolationError)
      })
    }

    it('survives a wire round trip through the generic KVP codec', () => {
      const info = new FecSourceInfo(1234)
      const buf = info.toKeyValuePair().serialize()
      expect(FecSourceInfo.fromKeyValuePair(KeyValuePair.deserialize(buf)).ssId).toBe(1234)
    })

    it('reports no extension when the object carries none', () => {
      expect(FecSourceInfo.fromProperties(null)).toBeNull()
      expect(FecSourceInfo.fromProperties([KeyValuePair.tryNewVarInt(0, 10)])).toBeNull()
    })

    it('refuses a pair of the wrong Type', () => {
      expect(() => FecSourceInfo.fromKeyValuePair(KeyValuePair.tryNewBytes(0x13, new Uint8Array(4)))).toThrow(
        ProtocolViolationError,
      )
    })

    it('refuses an SS_ID wider than 32 bits', () => {
      expect(() => new FecSourceInfo(FEC_U32_MAX + 1)).toThrow(ProtocolViolationError)
      expect(() => new FecSourceInfo(-1)).toThrow(ProtocolViolationError)
    })
  })
}
