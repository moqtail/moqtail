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

export function random60bitId(): bigint {
  const buf = new Uint8Array(8) // 64 bits
  crypto.getRandomValues(buf)
  buf[0] = buf[0]! & 0x0f // mask top 4 bits to get 60 bits total
  let id = 0n
  for (const b of buf) id = (id << 8n) | BigInt(b)
  return id
}

if (import.meta.vitest) {
  const { describe, it, expect } = import.meta.vitest
  describe('random60bitId', () => {
    it('should produce different values on multiple calls', () => {
      const iter = 100
      const ids = Array<bigint>(iter)
      for (let i = 0; i < iter; i++) {
        ids[i] = random60bitId()
        for (let j = 0; j < i; j++) {
          expect(ids[j]).not.toEqual(ids[i])
        }
      }
    })
  })
}
