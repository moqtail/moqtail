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

/** Media doubles for in-source tests in node. Never exported from the player entry. */

/** `TimeRanges` over `[start, end]` pairs in seconds. */
export function timeRanges(ranges: [number, number][]): TimeRanges {
  return { length: ranges.length, start: (i) => ranges[i]![0], end: (i) => ranges[i]![1] }
}

/**
 * A `SourceBuffer` that finishes each operation on the next macrotask. Each append adds
 * one second after the last range; `fail` makes the next append throw or fire `error`.
 */
export class FakeSourceBuffer extends EventTarget {
  updating = false
  ranges: [number, number][] = []
  /** Every call, in the order it started, e.g. `append 3`, `remove 0-1`, `changeType video/mp4`. */
  readonly log: string[] = []
  fail: 'quota' | 'error' | undefined

  get buffered(): TimeRanges {
    return timeRanges(this.ranges)
  }

  appendBuffer(data: Uint8Array): void {
    this.#start(`append ${data.length}`)
    if (this.fail === 'quota') {
      this.fail = undefined
      this.updating = false
      throw new DOMException('full', 'QuotaExceededError')
    }
    const failed = this.fail === 'error'
    this.fail = undefined
    this.#finish(() => {
      if (failed) {
        this.dispatchEvent(new Event('error'))
        return
      }
      const end = this.ranges.at(-1)?.[1] ?? 0
      if (this.ranges.length) this.ranges.at(-1)![1] = end + 1
      else this.ranges.push([0, 1])
    })
  }

  remove(start: number, end: number): void {
    this.#start(`remove ${start}-${end}`)
    this.#finish(() => {
      this.ranges = this.ranges
        .flatMap(([s, e]): [number, number][] => [
          [s, Math.min(e, start)],
          [Math.max(s, end), e],
        ])
        .filter(([s, e]) => e > s)
    })
  }

  changeType(type: string): void {
    if (this.updating) throw new DOMException('busy', 'InvalidStateError')
    this.log.push(`changeType ${type}`)
  }

  #start(entry: string): void {
    if (this.updating) throw new DOMException('busy', 'InvalidStateError')
    this.log.push(entry)
    this.updating = true
  }

  #finish(apply: () => void): void {
    setTimeout(() => {
      apply()
      this.updating = false
      this.dispatchEvent(new Event('updateend'))
    })
  }
}
