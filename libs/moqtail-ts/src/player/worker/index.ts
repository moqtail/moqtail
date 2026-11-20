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

import * as Comlink from 'comlink'
import { checkName } from '@/player/element/define'
import { WorkerHost, type EngineClass } from '@/player/element/worker/host'
import { registerTransferHandlers } from '@/player/element/worker/transfer'
import { PlayoutEngine } from '@/player/elements/playout/engine'
import { Decoding } from '@/player/elements/webcodecs/decoding'

export type { Engine, EngineClass, EngineInit } from '@/player/element/worker/host'

const BUILT_IN_ENGINES: [string, EngineClass][] = [
  ['webcodecs', Decoding],
  ['playout', PlayoutEngine],
]

/**
 * @public
 * Serves the player's worker from the script it runs: the built-in engines, plus `engines` by the
 * name of the element that creates them. Pass the script to the player's `worker` option.
 * @throws TypeError for a name with whitespace, or one a built-in engine has.
 */
export function serveWorker(engines: Record<string, EngineClass> = {}): void {
  const classes = new Map(BUILT_IN_ENGINES)
  for (const [name, engine] of Object.entries(engines)) {
    checkName(`Engine "${name}"`, name)
    if (classes.has(name)) throw new TypeError(`Engine "${name}" is a built-in engine`)
    classes.set(name, engine)
  }
  registerTransferHandlers()
  Comlink.expose(new WorkerHost(classes))
}

if (import.meta.vitest) {
  const { test, expect } = import.meta.vitest
  const { inProcessWorker, PassEngine } = await import('@/player/testing/worker')

  test('serveWorker reports element:<name> for the built-in engines and the caller’s', async () => {
    const host = Comlink.wrap<WorkerHost>(inProcessWorker({ grayscale: PassEngine }))
    const names = await host.environment()
    expect(names).toEqual(expect.arrayContaining(['element:webcodecs', 'element:playout', 'element:grayscale']))
    expect(names).toContain('MessageChannel')
  })

  test('serveWorker throws a TypeError for a name with whitespace or a built-in engine’s', () => {
    expect(() => serveWorker({ 'gray scale': PassEngine })).toThrow(TypeError)
    expect(() => serveWorker({ webcodecs: PassEngine })).toThrow(TypeError)
  })
}
