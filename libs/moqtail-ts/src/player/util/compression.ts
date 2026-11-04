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
 * MSF compression (MSF 5.5, 12.1) of catalog, media timeline, and event timeline payloads.
 * The properties are MSF specific, so they live here and not in the model. Read them with
 * `findProperty`.
 */

import { KeyValuePair } from '@/model/common/pair'

// Even, so the value is a varint (moqt-22 KeyValuePair).
const MSF_COMPRESSION = 0x78

/** Track property: every object of the track is compressed with `algorithm` (MSF 12.1.1). */
export class MsfCompressionTrackProperty {
  static readonly TYPE = MSF_COMPRESSION

  constructor(readonly algorithm: number) {}

  toKeyValuePair(): KeyValuePair {
    return KeyValuePair.tryNewVarInt(MSF_COMPRESSION, this.algorithm)
  }

  static fromKeyValuePair(pair: KeyValuePair): MsfCompressionTrackProperty | undefined {
    if (pair.typeValue !== BigInt(MSF_COMPRESSION) || typeof pair.value !== 'bigint') return undefined
    return new MsfCompressionTrackProperty(Number(pair.value))
  }
}

/** Object property: this object is compressed with `algorithm` (MSF 12.1.2). */
export class MsfCompressionObjectProperty {
  static readonly TYPE = MSF_COMPRESSION

  constructor(readonly algorithm: number) {}

  toKeyValuePair(): KeyValuePair {
    return KeyValuePair.tryNewVarInt(MSF_COMPRESSION, this.algorithm)
  }

  static fromKeyValuePair(pair: KeyValuePair): MsfCompressionObjectProperty | undefined {
    if (pair.typeValue !== BigInt(MSF_COMPRESSION) || typeof pair.value !== 'bigint') return undefined
    return new MsfCompressionObjectProperty(Number(pair.value))
  }
}

/** The algorithms of MSF 12.1, Table 14, that {@link decompress} supports. */
export const isSupportedCompression = (algorithm: number): boolean => algorithm === 0 || algorithm === 1

/**
 * Decompresses a payload: 0 is uncompressed, 1 is GZIP. Output is counted while streaming,
 * so a payload that inflates past `maxBytes` is aborted early.
 * @throws Error for an unsupported algorithm, a corrupt payload, or output over `maxBytes`.
 */
export async function decompress(bytes: Uint8Array, algorithm: number, maxBytes: number): Promise<Uint8Array> {
  if (!isSupportedCompression(algorithm)) throw new Error(`Unsupported MSF compression algorithm ${algorithm}`)
  if (algorithm === 0) return bytes
  const reader = new Blob([bytes as Uint8Array<ArrayBuffer>])
    .stream()
    .pipeThrough(new DecompressionStream('gzip'))
    .getReader()
  const chunks: Uint8Array[] = []
  let size = 0
  for (let read = await reader.read(); !read.done; read = await reader.read()) {
    size += read.value.length
    if (size > maxBytes) {
      await reader.cancel()
      throw new Error(`Decompressed payload is larger than ${maxBytes} bytes`)
    }
    chunks.push(read.value)
  }
  const output = new Uint8Array(size)
  let offset = 0
  for (const chunk of chunks) {
    output.set(chunk, offset)
    offset += chunk.length
  }
  return output
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest
  const { findProperty } = await import('@/model/property/find')
  const text = new TextEncoder().encode('{"version":"draft-01","tracks":[]}'.repeat(100))
  const gzip = async (bytes: Uint8Array) =>
    new Uint8Array(
      await new Response(
        new Blob([bytes as Uint8Array<ArrayBuffer>]).stream().pipeThrough(new CompressionStream('gzip')),
      ).arrayBuffer(),
    )

  describe('decompress', () => {
    test('round trips GZIP and passes uncompressed payloads through', async () => {
      expect(await decompress(await gzip(text), 1, text.length)).toEqual(text)
      expect(await decompress(text, 0, 0)).toBe(text)
    })

    test('aborts past maxBytes, and rejects corrupt payloads and unknown algorithms', async () => {
      await expect(decompress(await gzip(text), 1, text.length - 1)).rejects.toThrow(/larger than/)
      await expect(decompress(text, 1, text.length)).rejects.toThrow()
      await expect(decompress(text, 2, text.length)).rejects.toThrow(/Unsupported/)
    })
  })

  test('the properties are read with findProperty', () => {
    const pair = new MsfCompressionTrackProperty(1).toKeyValuePair()
    expect(findProperty([pair], MsfCompressionTrackProperty)?.algorithm).toBe(1)
    expect(findProperty([pair], MsfCompressionObjectProperty)?.algorithm).toBe(1)
    expect(findProperty([KeyValuePair.tryNewVarInt(0x7a, 1)], MsfCompressionTrackProperty)).toBeUndefined()
  })
}
