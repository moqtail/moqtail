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
 * `test` with an `open(uri, options)` fixture: a started, recording `Session` per call.
 * Every session is closed after the test, and a failed test prints their timelines.
 */

import { test as base } from 'vitest'
import { Session, type OpenOptions } from './session'

export const test = base.extend<{ open: (uri: string, options?: OpenOptions) => Promise<Session> }>({
  open: async ({ onTestFailed }, use) => {
    const sessions: Session[] = []
    onTestFailed(() => sessions.forEach((s) => console.error(s.timeline())))
    await use(async (uri, options) => {
      const session = new Session(uri)
      sessions.push(session)
      await session.begin(options)
      return session
    })
    for (const session of sessions) await session.close()
  },
})

export { describe, expect } from 'vitest'
export { Session, type OpenOptions } from './session'
export * from './sources'
export { warmUp } from './warm'
