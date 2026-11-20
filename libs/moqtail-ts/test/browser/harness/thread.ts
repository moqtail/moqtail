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
 * Where a run plays: engines in the worker by default, on the main thread with
 * `PREFER_WORKER=false npm run test:browser`. Players get the test worker entry, as an app passes its own.
 */

import { inject, onTestFinished } from 'vitest'
import { PlayerWorker } from '@/player/element/worker/player_worker'
import type { MOQtailPlayerOptions } from '@/player/player'

declare module 'vitest' {
  interface ProvidedContext {
    preferWorker: boolean
  }
}

export const preferWorker = inject('preferWorker')

export function spawnTestWorker(): Worker {
  return new Worker(new URL('./worker.ts', import.meta.url), { type: 'module' })
}

export function playerOptions(): MOQtailPlayerOptions {
  return { preferWorker, worker: spawnTestWorker }
}

/** A started worker for `testChain` in a worker run, terminated after the test. */
export async function chainWorker(): Promise<PlayerWorker | undefined> {
  if (!preferWorker) return undefined
  const worker = new PlayerWorker(spawnTestWorker())
  onTestFinished(() => worker.terminate())
  if (!(await worker.start(new AbortController().signal))) throw new Error('The test worker did not start')
  return worker
}
