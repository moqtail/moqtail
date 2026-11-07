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

import type { Setup } from '@/model/control/setup'
import { SstsAlgorithms } from '@/model/parameter/setup/ssts_algorithms'

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
