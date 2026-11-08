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
 * switching set. Listing `ssts()` in `opts.plugins` offers it at setup; once the peer accepts
 * it, the catalog loads `SstsSource` for each media switching set.
 */

import { definePlugin } from '@/player/plugin'
import { SSTS_ALGORITHMS, sstsAlgorithms } from './algorithms'
import { sstsSource } from './source'

export { SstsSource } from './source'
export { SSTS_ALGORITHMS } from './algorithms'

/** @public */
export interface SstsConfig {
  /** Offered at setup, most preferred first; the source starts on the first negotiated one. Default `SSTS_ALGORITHMS`. */
  algorithms?: readonly bigint[]
}

/** @public The SSTS plugin, for `opts.plugins`. */
export const ssts = definePlugin({
  name: 'ssts',
  init: ({ algorithms = SSTS_ALGORITHMS }: SstsConfig = {}) => {
    // Unknown ids are fine: only the relay knows what it runs
    if (new Set(algorithms).size !== algorithms.length) throw new TypeError('SSTS algorithms must not repeat')
    return { algorithms }
  },
  setup: ({ algorithms }, clientSetup) => {
    // An empty list offers nothing, which prohibits SSTS (SSTS 3)
    if (algorithms.length === 0) return false
    clientSetup.addSstsAlgorithms([...algorithms])
    return true
  },
  negotiated: ({ algorithms }, serverSetup) => sstsAlgorithms(serverSetup, algorithms).length > 0,
  features: ({ algorithms }) => [{ type: 'switch-source', item: sstsSource(algorithms) }],
})

if (import.meta.vitest) {
  const { test, expect } = import.meta.vitest
  const { Setup } = await import('@/model/control/setup')
  const { SetupOptions } = await import('@/model/parameter/setup_option')
  const { activePlugins, initPlugins } = await import('@/player/plugin')
  const setup = (algorithms?: bigint[]) =>
    new Setup(algorithms ? new SetupOptions().addSstsAlgorithms(algorithms).build() : [])
  const offered = (...config: Parameters<typeof ssts>) =>
    initPlugins([ssts(...config)]).map((p) => p.setupOptions.build().length)

  test('sstsAlgorithms: the common algorithms in our order', () => {
    expect(sstsAlgorithms(setup([0xff01n, 7n, 0n]), SSTS_ALGORITHMS)).toEqual([0n, 0xff01n])
    expect(sstsAlgorithms(setup([0xff01n, 7n, 0n]), [0xff01n, 0n])).toEqual([0xff01n, 0n])
    expect(sstsAlgorithms(setup([7n]), SSTS_ALGORITHMS)).toEqual([])
    expect(sstsAlgorithms(setup([]), SSTS_ALGORITHMS)).toEqual([])
    expect(sstsAlgorithms(setup(), SSTS_ALGORITHMS)).toEqual([])
  })

  test('ssts(): offers its algorithms, is active when the peer shares one, and an empty list drops it', () => {
    expect([offered(), offered({ algorithms: [0xff02n] }), offered({ algorithms: [] })]).toEqual([[1], [1], []])
    expect(() => ssts({ algorithms: [1n, 1n] })).not.toThrow() // checked per player, in init
    expect(() => initPlugins([ssts({ algorithms: [1n, 1n] })])).toThrow(TypeError)
    const plugins = initPlugins([ssts({ algorithms: [0xff01n] })])
    expect([activePlugins(plugins, setup([0xff01n])).length, activePlugins(plugins, setup([0n])).length]).toEqual([
      1, 0,
    ])
  })
}
