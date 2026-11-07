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
 * The one client every player shares. The first player to connect creates it, the last
 * one to leave disconnects it, and a lost session is recreated here for all of them.
 */

import { MOQtailClient } from '@/client/client'
import type { MOQtailClientOptions } from '@/client/types'
import type { MOQtailPlayerOptions } from '@/player/player'
import { DEFAULT_RECONNECT_ATTEMPTS } from '@/player/constants'
import { backoffDelay, sleep } from '@/player/util/async'
import { canonicalJson } from '@/player/util/json'
import { logger } from '@/util/logger'
import { PlayerError } from './error'

/** The `client` player options, with the extensions' setup options merged in. */
export type ClientOptions = NonNullable<MOQtailPlayerOptions['client']>

/** What the slot tells each player holding the client. */
export interface SlotMember {
  reconnecting(attempt: number): void
  reconnected(): void
  /** The session couldn't be recovered. */
  fail(error: PlayerError): void
}

interface Slot {
  client: MOQtailClient | undefined
  origin: string | undefined
  key: string | undefined
  /** Resolves to the current client; replaced while a lost session is recreated. */
  connecting: Promise<MOQtailClient> | undefined
  players: Set<SlotMember>
  disposing: Promise<void> | undefined
  /** Stops the retry loop once every player has left. */
  abort: AbortController
}

/** Swapped by tests for a fake client. */
export const factory = {
  createClient: (options: MOQtailClientOptions): MOQtailClient => new MOQtailClient(options),
}

// Symbol.for, so the ESM and CJS builds share one client
const SLOT = Symbol.for('moqtail.client')

function slot(): Slot {
  const global = globalThis as { [SLOT]?: Slot }
  return (global[SLOT] ??= {
    client: undefined,
    origin: undefined,
    key: undefined,
    connecting: undefined,
    players: new Set(),
    disposing: undefined,
    abort: new AbortController(),
  })
}

/** Throws `UNSUPPORTED_ORIGIN` when a client is live or connecting for another origin or setup. */
export function check(origin: string, key: string): void {
  const s = slot()
  if (s.origin === undefined || (s.origin === origin && s.key === key)) return
  const reason = s.origin === origin ? 'with other client options' : `to ${s.origin}`
  throw new PlayerError('UNSUPPORTED_ORIGIN', `The shared client is already connected ${reason}`)
}

/** The current client. Elements reach it through `context.client`. */
export function currentClient(): MOQtailClient {
  const client = slot().client
  if (!client) throw new PlayerError('SESSION_LOST', 'There is no connected client')
  return client
}

/**
 * Joins `member` to the client, creating it if needed. Concurrent players share one
 * attempt, which retries with backoff; if it fails, every waiter rejects and the slot clears.
 */
export function acquire(member: SlotMember, origin: string, options: ClientOptions): Promise<MOQtailClient> {
  const s = slot()
  const key = clientKey(options)
  check(origin, key)
  s.players.add(member)
  if (s.connecting) return s.connecting
  s.origin = origin
  s.key = key
  s.abort = new AbortController()
  const disposing = s.disposing
  const connecting = (async () => {
    await disposing
    return retry(s, origin, options)
  })()
  s.connecting = connecting
  connecting.catch(() => {
    if (s.connecting === connecting) clear(s)
  })
  return connecting
}

/**
 * Removes `member` at once. The last player out disconnects the client once `after` (its
 * element teardown) and any running attempt have settled. A player that connects meanwhile
 * waits for that, then creates a new client.
 */
export async function release(member: SlotMember, after: Promise<unknown>): Promise<void> {
  const s = slot()
  if (!s.players.delete(member) || s.players.size > 0) {
    await after.catch(() => {})
    return
  }
  s.abort.abort(new PlayerError('ABORTED', 'Every player released the client'))
  const connecting = s.connecting
  s.origin = s.key = s.connecting = undefined
  s.players = new Set()
  const disposing = (async () => {
    await Promise.allSettled([after, connecting])
    await s.client?.disconnect().catch((error: unknown) => logger.warn('MOQtailPlayer', 'disconnect failed', error))
    s.client = undefined
  })()
  s.disposing = disposing
  await disposing
  if (s.disposing === disposing) s.disposing = undefined
}

/**
 * Setup parameters as `[type, value]` pairs sorted by type, transport options with sorted
 * keys, and the attempt count. Equal keys mean the same session.
 */
export function clientKey(options: ClientOptions): string {
  const setup = (options.setupOptions?.build() ?? [])
    .map((pair) => [pair.typeValue, pair.value] as const)
    .sort((a, b) => Number(a[0] - b[0]))
  return canonicalJson({
    setup,
    transport: options.transportOptions ?? {},
    reconnectAttempts: options.reconnectAttempts ?? DEFAULT_RECONNECT_ATTEMPTS,
  })
}

function clear(s: Slot): void {
  s.client = s.origin = s.key = s.connecting = undefined
  s.players = new Set()
}

/** Creates the client, attaches the slot's listener, then runs the handshake. */
async function open(s: Slot, origin: string, options: ClientOptions): Promise<MOQtailClient> {
  const transport = options.transportOptions
  const client = factory.createClient({
    url: origin,
    ...(options.setupOptions && { setupOptions: options.setupOptions }),
    // A copy, because the client appends its versions to `protocols`
    ...(transport && {
      transportOptions: { ...transport, ...(transport.protocols && { protocols: [...transport.protocols] }) },
    }),
  })
  client.on('sessionterminated', () => reconnect(s, client, origin, options))
  await client.init()
  s.client = client
  return client
}

/** Recreates a lost session for every player, or fails them all with `SESSION_LOST`. */
function reconnect(s: Slot, client: MOQtailClient, origin: string, options: ClientOptions): void {
  if (s.disposing || s.client !== client) return
  // Captured, since `clear()` replaces the set before the failure is reported
  const players = s.players
  const each = (notify: (member: SlotMember) => void) => [...players].forEach(notify)
  const connecting = retry(s, origin, options, (attempt) => each((m) => m.reconnecting(attempt)))
  s.connecting = connecting
  connecting.then(
    () => each((m) => m.reconnected()),
    (error: PlayerError) => {
      if (s.connecting !== connecting) return // released meanwhile
      clear(s)
      each((m) => m.fail(error))
    },
  )
}

/**
 * Up to `reconnectAttempts` attempts with backoff. Time hidden or offline doesn't count.
 * `onAttempt` runs synchronously for the first one, so players learn of a lost session at once.
 */
async function retry(
  s: Slot,
  origin: string,
  options: ClientOptions,
  onAttempt?: (attempt: number) => void,
): Promise<MOQtailClient> {
  const signal = s.abort.signal
  const attempts = options.reconnectAttempts ?? DEFAULT_RECONNECT_ATTEMPTS
  for (let failed = 0; ;) {
    onAttempt?.(failed + 1)
    await untilAvailable(signal)
    try {
      return await open(s, origin, options)
    } catch (cause) {
      signal.throwIfAborted()
      if (!available()) continue
      if (++failed >= attempts) throw new PlayerError('SESSION_LOST', `Could not connect to ${origin}`, { cause })
      await sleep(backoffDelay(failed - 1), signal)
    }
  }
}

function available(): boolean {
  return globalThis.document?.visibilityState !== 'hidden' && globalThis.navigator?.onLine !== false
}

/** Waits for the page to be visible and online, or for the signal to abort. */
async function untilAvailable(signal: AbortSignal): Promise<void> {
  while (!available()) {
    await new Promise<void>((resolve) => {
      const done = () => {
        globalThis.document?.removeEventListener('visibilitychange', done)
        globalThis.removeEventListener?.('online', done)
        signal.removeEventListener('abort', done)
        resolve()
      }
      globalThis.document?.addEventListener('visibilitychange', done)
      globalThis.addEventListener?.('online', done)
      signal.addEventListener('abort', done)
    })
    signal.throwIfAborted()
  }
}

if (import.meta.vitest) {
  const { describe, test, expect, beforeEach, afterEach, vi } = import.meta.vitest
  const { fakeClients } = await import('@/player/testing/client')
  const { flush } = await import('@/player/testing/elements')
  const { SetupOptions } = await import('@/model/parameter/setup_option')

  const ORIGIN = 'moqt://relay.example.com/app'
  const member = () => {
    const log: string[] = []
    const m: SlotMember & { log: string[] } = {
      log,
      reconnecting: (attempt) => log.push(`reconnecting ${attempt}`),
      reconnected: () => log.push('reconnected'),
      fail: (error) => log.push(`fail ${error.code}`),
    }
    return m
  }
  let fakes: ReturnType<typeof fakeClients>
  beforeEach(() => {
    delete (globalThis as { [SLOT]?: Slot })[SLOT]
    fakes = fakeClients()
  })
  afterEach(() => {
    fakes.restore()
    vi.unstubAllGlobals()
  })

  describe('client slot', () => {
    test('players share one client, disconnected after the last release', async () => {
      const [a, b] = [member(), member()]
      const [first, second] = await Promise.all([acquire(a, ORIGIN, {}), acquire(b, ORIGIN, {})])
      expect(first).toBe(second)
      expect(fakes.clients).toHaveLength(1)
      await release(a, Promise.resolve())
      expect(fakes.clients[0]!.disconnected).toBe(false)
      await release(b, Promise.resolve())
      expect(fakes.clients[0]!.disconnected).toBe(true)
    })

    test('UNSUPPORTED_ORIGIN for another origin or setup, synchronously and on connect', async () => {
      const ssts = { setupOptions: new SetupOptions().addSstsAlgorithms([1]) }
      void acquire(member(), ORIGIN, ssts)
      expect(() => check('moqt://other.example.com/app', clientKey(ssts))).toThrow(
        expect.objectContaining({ code: 'UNSUPPORTED_ORIGIN' }),
      )
      expect(() => acquire(member(), ORIGIN, {})).toThrow(expect.objectContaining({ code: 'UNSUPPORTED_ORIGIN' }))
      // Same setup written differently is the same key
      expect(clientKey({ ...ssts, reconnectAttempts: 5 })).toBe(clientKey(ssts))
    })

    test('a failed connection rejects every waiter and clears the slot', async () => {
      fakes.restore()
      fakes = fakeClients(() => Promise.reject(new Error('refused')))
      const failing = { reconnectAttempts: 1 }
      const waiters = [acquire(member(), ORIGIN, failing), acquire(member(), ORIGIN, failing)]
      for (const waiter of waiters) await expect(waiter).rejects.toMatchObject({ code: 'SESSION_LOST' })
      expect(() => check('moqt://other.example.com/app', clientKey({}))).not.toThrow()
    })

    test('a release during connection disconnects once the attempt settles', async () => {
      fakes.restore()
      const opened = Promise.withResolvers<void>()
      fakes = fakeClients(() => opened.promise)
      const a = member()
      const connecting = acquire(a, ORIGIN, {})
      const released = release(a, Promise.resolve())
      await flush()
      expect(fakes.clients[0]!.disconnected).toBe(false)
      opened.resolve()
      await Promise.all([connecting, released])
      expect(fakes.clients[0]!.disconnected).toBe(true)
    })

    test('a player connecting during disposal waits for it, then gets a new client', async () => {
      const a = member()
      await acquire(a, ORIGIN, {})
      const teardown = Promise.withResolvers<void>()
      void release(a, teardown.promise)
      const next = acquire(member(), 'moqt://other.example.com/app', {})
      await flush()
      expect(fakes.clients).toHaveLength(1)
      teardown.resolve()
      expect(await next).toBe(fakes.clients[1])
      expect(fakes.clients[0]!.disconnected).toBe(true)
    })

    test('a lost session is recreated for every player; attempts wait while the page is hidden', async () => {
      const page = Object.assign(new EventTarget(), { visibilityState: 'visible' })
      vi.stubGlobal('document', page)
      const [a, b] = [member(), member()]
      await Promise.all([acquire(a, ORIGIN, {}), acquire(b, ORIGIN, {})])
      page.visibilityState = 'hidden'
      fakes.clients[0]!.drop()
      await flush()
      expect([fakes.clients.length, a.log]).toEqual([1, ['reconnecting 1']])

      page.visibilityState = 'visible'
      page.dispatchEvent(new Event('visibilitychange'))
      await flush()
      expect(currentClient()).toBe(fakes.clients[1])
      expect(a.log).toEqual(['reconnecting 1', 'reconnected'])
      expect(b.log).toEqual(a.log)
    })

    test('after the last attempt every player fails with SESSION_LOST', async () => {
      fakes.restore()
      fakes = fakeClients((i) => (i === 0 ? Promise.resolve() : Promise.reject(new Error('gone'))))
      const a = member()
      await acquire(a, ORIGIN, { reconnectAttempts: 1 })
      fakes.clients[0]!.drop()
      await flush()
      expect(a.log).toEqual(['reconnecting 1', 'fail SESSION_LOST'])
    })
  })
}
