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

export function getTransportPriority(priority: number): number {
  // Round to nearest integer and clamp between 0-255
  priority = Math.max(0, Math.min(255, Math.round(priority)))
  // Invert then map over 0-Number.MAX_SAFE_INTEGER so 0->MAX_SAFE_INTEGER and 255->0
  if (priority === 0) return Number.MAX_SAFE_INTEGER
  if (priority === 255) return 0
  // Due to floating-point precision below formula returns Number.MAX_SAFE_INTEGER + 1 for 0
  return Math.round((255 - priority) * (Number.MAX_SAFE_INTEGER / 255))
}

if (import.meta.vitest) {
  const { describe, it, expect } = import.meta.vitest
  describe('getTransportPriority', () => {
    it('should map 0 to MAX_SAFE_INTEGER and 255 to 0', () => {
      expect(getTransportPriority(0)).toBe(Number.MAX_SAFE_INTEGER)
      expect(getTransportPriority(255)).toBe(0)
    })
    it('should be monotonic decreasing as priority increases', () => {
      let last = getTransportPriority(0)
      for (let p = 1; p <= 255; ++p) {
        const val = getTransportPriority(p)
        expect(val).toBeLessThan(last)
        last = val
      }
    })
    it('should clamp out-of-range values', () => {
      expect(getTransportPriority(-100)).toBe(Number.MAX_SAFE_INTEGER)
      expect(getTransportPriority(300)).toBe(0)
    })
    it('should round fractional priorities', () => {
      expect(getTransportPriority(0.4)).toBe(Number.MAX_SAFE_INTEGER)
      expect(getTransportPriority(254.6)).toBe(getTransportPriority(255))
    })
  })
}
