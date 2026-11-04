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

/** ISO-BMFF box builders for tests. Never exported from the player entry. */

export const u32 = (n: number) => {
  const out = new Uint8Array(4)
  new DataView(out.buffer).setUint32(0, n)
  return out
}

export const u64 = (n: bigint) => {
  const out = new Uint8Array(8)
  new DataView(out.buffer).setBigUint64(0, n)
  return out
}

export function box(type: string, ...children: Uint8Array[]): Uint8Array {
  const body = children.flatMap((c) => [...c])
  return new Uint8Array([...u32(8 + body.length), ...new TextEncoder().encode(type), ...body])
}

export const fullBox = (type: string, version: number, ...children: Uint8Array[]) =>
  box(type, new Uint8Array([version, 0, 0, 0]), ...children)

/** A `moof` whose `tfdt` is at `time`, as a media object payload. */
export const fragment = (time: number) => box('moof', box('traf', fullBox('tfdt', 0, u32(time))))
