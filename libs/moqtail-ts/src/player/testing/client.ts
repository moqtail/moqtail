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

/** A relay-free stand-in for `MOQtailClient`. Never exported from the player entry. */

import type { MOQtailClient } from '../../client/client'
import type { ClientEvents, MOQtailClientOptions } from '../../client/types'
import { Emitter } from '../../util/emitter'
import { factory } from '../client_slot'

/** Covers the session lifecycle; request methods are added by the milestones that need them. */
export class FakeClient extends Emitter<ClientEvents> {
  disconnected = false
  #terminated = false

  constructor(
    readonly options: MOQtailClientOptions,
    readonly opened: Promise<void>,
  ) {
    super()
  }

  init(): Promise<void> {
    return this.opened
  }

  async disconnect(reason?: unknown): Promise<void> {
    this.disconnected = true
    this.drop(reason)
  }

  /** Ends the session, as a lost connection does. */
  drop(reason?: unknown): void {
    if (this.#terminated) return
    this.#terminated = true
    this.emit('sessionterminated', reason)
  }
}

/**
 * Makes the client slot create `FakeClient`s. `open` decides how each `init()` settles,
 * by creation index. Returns the clients created so far and a restore function.
 */
export function fakeClients(open: (index: number) => Promise<void> = () => Promise.resolve()) {
  const clients: FakeClient[] = []
  const saved = factory.createClient
  factory.createClient = (options) => {
    const client = new FakeClient(options, open(clients.length))
    clients.push(client)
    return client as unknown as MOQtailClient
  }
  return { clients, restore: () => (factory.createClient = saved) }
}
