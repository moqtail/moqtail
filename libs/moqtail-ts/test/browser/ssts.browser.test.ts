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

/** SSTS: the relay picks the video track of the set, so its switches aren't tested; only that it negotiates and data arrives and plays. */

import { SSTS_ALGORITHMS, ssts } from '@/player'
import { sstsAlgorithms } from '@/player/plugins/ssts/algorithms'
import { ch07, describe, expect, test, warmUp } from './harness/test'

describe('ssts', () => {
  warmUp(ch07)

  test('SSTS is negotiated and the video set delivers and plays', async ({ open }) => {
    const s = await open(ch07.uri(), { player: { plugins: [ssts()] } })
    expect(sstsAlgorithms(s.client.serverSetup!, SSTS_ALGORITHMS), 'the relay negotiated SSTS').not.toEqual([])
    const playing = s.mark()
    await s.played(2)
    expect(s.objectsOf('video').length).toBeGreaterThan(0)
    s.since(playing).expectSmooth()
  })
})
