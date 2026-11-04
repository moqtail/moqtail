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
 * Sender-side track switching (draft-wilaw-moq-moqt-ssts): the relay picks the track of a
 * switching set. Listing `SSTS` in `opts.extensions` offers it at setup; the catalog then loads
 * `SstsSource` for each media switching set when the peer accepted it.
 */

import type { Setup } from '../../../model/control/setup'
import { SetupOptions } from '../../../model/parameter/setup_option'
import { SstsAlgorithms } from '../../../model/parameter/setup/ssts_algorithms'
import { defineExtension } from '../../extension'
import { Ssts } from './source'

/** The algorithms offered at setup: 0 (the draft's default) and demo-abr's 0xff01. */
export const SSTS_ALGORITHMS: readonly bigint[] = [0n, 0xff01n]

/**
 * The negotiated algorithms: ours that the peer's SETUP lists, in our order. Empty when it
 * lists none or omits the option, which prohibits SSTS (SSTS 3).
 */
export function sstsAlgorithms(serverSetup: Setup): bigint[] {
  const offered = serverSetup.setupOptions.map((pair) => SstsAlgorithms.fromKeyValuePair(pair)).find((o) => o)
  return SSTS_ALGORITHMS.filter((a) => offered?.algorithms.includes(a))
}

/** @public The SSTS extension, for `opts.extensions`. */
export const SSTS = defineExtension({
  name: 'ssts',
  setupOptions: new SetupOptions().addSstsAlgorithms([...SSTS_ALGORITHMS]),
  switchingSetSource: Ssts,
  negotiated: (serverSetup) => sstsAlgorithms(serverSetup).length > 0,
})

export { SstsSource } from './source'

if (import.meta.vitest) {
  const { test, expect } = import.meta.vitest
  const { Setup } = await import('../../../model/control/setup')
  const setup = (algorithms?: bigint[]) =>
    new Setup(algorithms ? new SetupOptions().addSstsAlgorithms(algorithms).build() : [])

  test('sstsAlgorithms and SSTS.negotiated: the common algorithms in our order', () => {
    expect(sstsAlgorithms(setup([0xff01n, 7n, 0n]))).toEqual([0n, 0xff01n])
    expect(sstsAlgorithms(setup([7n]))).toEqual([])
    expect(sstsAlgorithms(setup([]))).toEqual([])
    expect(sstsAlgorithms(setup())).toEqual([])
    expect(SSTS.negotiated!(setup([0xff01n]))).toBe(true)
    expect(SSTS.negotiated!(setup([7n]))).toBe(false)
  })
}
