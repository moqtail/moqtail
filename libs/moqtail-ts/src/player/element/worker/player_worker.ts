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
 * The main thread's side of a player's worker: spawning it, its environment report, the
 * `WorkerContext` of each element allowed to use it, and the routes of worker pads. A worker
 * pad's items live in the worker; its mirror is the pad the chain routes. Its route is set once
 * every connection of the mirror chose: linked when each consumer linked its engine, bridged
 * to the mirror otherwise.
 */

import * as Comlink from 'comlink'
import type { BaseElement } from '@/player/element/base'
import type { InstanceContext } from '@/player/element/context'
import { CapabilityCode } from '@/player/element/define'
import { closeFrame, type Pad, type PadOutput } from '@/player/element/pad'
import { roleOf } from '@/player/element/sink/media_sink'
import { PlayerError } from '@/player/error'
import { within } from '@/player/util/async'
import type { Engine, Link, MainSide, WorkerHost } from './host'
import { registerTransferHandlers } from './transfer'

/** @inline */
export interface ThreadEnvironment {
  /**
   * The player's worker, absent without one. `has` answers from the worker's own report: its
   * global names, and `element:<name>` for each engine it serves.
   */
  readonly worker?: { has(name: string): boolean }
}

/** @inline `context.worker`, for an element whose `threads` allows the worker. */
export interface WorkerContext {
  has(name: string): boolean
  /**
   * Creates the engine `serveWorker` registered as `name`. Calls on it queue until it exists, and a
   * failure to create it reaches `context.fail`.
   */
  create<E extends Engine>(name: string, request?: EngineRequest): Comlink.Remote<E>
  /** Sends the items of `pad`, a sink pad fed by a worker pad, to `engine` in the worker. */
  link(pad: Pad, engine: Comlink.Remote<Engine>): void
  /** Sends them to `receive()` on the main thread instead. Each worker pad the element takes needs one of the two. */
  bridge(pad: Pad): void
  /** Whether `pad` is a worker pad, or a sink pad one feeds. */
  inWorker(pad: Pad): boolean
}

/** @inline */
export interface EngineRequest {
  /** Source pads of the element, carrying `THREAD` `worker`: the engine's `init.outputs`, in order. */
  outputs?: Pad[]
  /** The engine's `init.options`. */
  options?: object
  /** Transferred with `options`, for example an `OffscreenCanvas`. */
  transfer?: Transferable[]
  /** The engine's `init.report`. */
  report?: (report: any) => void
}

export type ElementWorker = WorkerContext & { release(): void }

type Owner = Pick<InstanceContext, 'element' | 'fail'>

interface Created {
  owner: Owner
  name: string
  engine: Comlink.Remote<Engine>
}

interface Output {
  key: number
  engine: number
  routed: boolean
}

const START_TIMEOUT_MS = 10_000

export class PlayerWorker {
  readonly #worker: Worker
  readonly #host: Comlink.Remote<WorkerHost>
  readonly #owners = new Set<Owner>()
  readonly #created = new Map<number, Created>()
  readonly #ids = new WeakMap<object, number>()
  readonly #mirrors = new Map<number, Pad>()
  readonly #outputs = new Map<Pad, Output>()
  readonly #choices = new WeakMap<Pad, Link | 'bridge'>()
  #names = new Set<string>()
  #nextId = 1
  #used = false
  #terminated = false

  constructor(worker: Worker) {
    registerTransferHandlers()
    this.#worker = worker
    this.#host = Comlink.wrap<WorkerHost>(worker)
  }

  get used(): boolean {
    return this.#used
  }

  /** Resolves false, and terminates it, once the worker failed to load or to report in time. */
  async start(signal: AbortSignal): Promise<boolean> {
    const failed = Promise.withResolvers<never>()
    const onError = (event: Event) => {
      failed.reject(event)
      // Past startup, an error no engine caught: every owner is in the one player, so one reports it
      const error = new PlayerError('MEDIA_FAILURE', 'The player worker failed', { cause: event })
      ;[...this.#owners][0]?.fail(error)
    }

    this.#worker.addEventListener('error', onError, { signal })

    try {
      const report = within(
        Promise.race([this.#host.environment(), failed.promise]),
        START_TIMEOUT_MS,
        signal,
        () => new Error('The worker did not report'),
      )
      this.#names = new Set(await report)

      await this.#host.connect(
        Comlink.proxy<MainSide>({
          bridge: (key, item) => this.#bridged(key, item),
          fail: (id, error) => this.#failed(id, error),
        }),
      )
      return true
    } catch {
      this.terminate()
      return false
    }
  }

  environment(): ThreadEnvironment {
    return this.#terminated ? {} : { worker: { has: (name) => this.#has(name) } }
  }

  context(owner: Owner): ElementWorker | undefined {
    if (this.#terminated) return undefined

    this.#owners.add(owner)
    const created: number[] = []

    return {
      has: (name) => this.#has(name),
      create: <E extends Engine>(name: string, request: EngineRequest = {}) => {
        const id = this.#create(owner, name, request)
        created.push(id)
        return this.#created.get(id)!.engine as Comlink.Remote<E>
      },
      link: (pad, engine) => {
        const id = this.#ids.get(engine)
        if (id === undefined) throw new TypeError('link() takes an engine from create()')
        this.#choose(pad, { engine: id, role: roleOf(pad) ?? '' })
      },
      bridge: (pad) => this.#choose(pad, 'bridge'),
      inWorker: (pad) => this.#inWorker(pad.direction === 'src' ? pad : pad.peers[0]),
      release: () => {
        this.#owners.delete(owner)
        for (const id of created) this.#release(id)
      },
    }
  }

  connected(pad: Pad): void {
    if (this.#outputs.has(pad)) this.#route(pad)
  }

  terminate(): void {
    this.#terminated = true
    this.#worker.terminate()
  }

  #has(name: string): boolean {
    return !this.#terminated && this.#names.has(name)
  }

  /**
   * The chain connects a worker pad as soon as it's created, before its element hands it to `create()`,
   * so a consumer checking then goes by its `THREAD`, set only by an element with a `WorkerContext`.
   */
  #inWorker(pad: Pad | undefined): boolean {
    if (!pad) return false
    const thread = pad.capabilities.some((c) => c.code === CapabilityCode.THREAD && c.value === 'worker')
    return this.#outputs.has(pad) || (thread && this.#hasContext(pad.element))
  }

  #create(owner: Owner, name: string, request: EngineRequest): number {
    this.#used = true
    const id = this.#nextId++

    const outputs = (request.outputs ?? []).map((pad) => {
      const key = this.#nextId++
      this.#mirrors.set(key, pad)
      this.#outputs.set(pad, { key, engine: id, routed: false })
      return key
    })

    const { port1, port2 } = new MessageChannel()
    const engine = Comlink.wrap<Engine>(port1)
    this.#ids.set(engine, id)
    this.#created.set(id, { owner, name, engine })
    const options = request.transfer ? Comlink.transfer(request.options ?? {}, request.transfer) : request.options
    const report = request.report && Comlink.proxy(request.report)
    this.#host
      .create(id, name, Comlink.transfer(port2, [port2]), outputs, options, report)
      .catch((error: unknown) => owner.fail(failure(error, name)))

    for (const pad of request.outputs ?? []) this.#route(pad)

    return id
  }

  #choose(pad: Pad, choice: Link | 'bridge'): void {
    this.#choices.set(pad, choice)
    const mirror = pad.peers[0]
    if (mirror && this.#outputs.has(mirror)) this.#route(mirror)
  }

  /** After the chain's current routing step, so every target it connects in one go has chosen. */
  #route(mirror: Pad): void {
    queueMicrotask(() => {
      const output = this.#outputs.get(mirror)
      if (!output || output.routed || mirror.peers.length === 0 || this.#terminated) return

      const choices = mirror.peers.map(
        (sink) => this.#choices.get(sink) ?? (this.#hasContext(sink.element) ? undefined : 'bridge'),
      )
      if (choices.includes(undefined)) return

      output.routed = true
      const links = choices.filter((choice) => choice !== 'bridge' && choice !== undefined)

      void this.#host.route(output.key, links.length === choices.length ? links : 'bridge')
    })
  }

  /** Only an element with a `WorkerContext` can link or make a worker pad. */
  #hasContext(element: BaseElement): boolean {
    return [...this.#owners].some((owner) => owner.element === element)
  }

  #bridged(key: number, item: PadOutput): Promise<void> {
    const mirror = this.#mirrors.get(key)
    if (mirror) return mirror.write(item)

    closeFrame(item)
    return Promise.resolve()
  }

  #failed(id: number, error: unknown): void {
    const created = this.#created.get(id)
    created?.owner.fail(failure(error, created.name))
  }

  #release(id: number): void {
    const created = this.#created.get(id)
    if (!created) return

    this.#created.delete(id)
    created.engine[Comlink.releaseProxy]()

    for (const [pad, output] of this.#outputs) {
      if (output.engine !== id) continue

      this.#outputs.delete(pad)
      this.#mirrors.delete(output.key)
    }

    if (!this.#terminated) void this.#host.release(id)
  }
}

/** The `worker` option, else the default script next to the ESM build. Undefined where neither can run. */
export function spawnWorker(worker?: () => Worker): PlayerWorker | undefined {
  if (worker) return new PlayerWorker(worker())
  if (typeof Worker === 'undefined') return undefined

  try {
    return new PlayerWorker(new Worker(new URL('./player/worker-default.mjs', import.meta.url), { type: 'module' }))
  } catch {
    return undefined
  }
}

function failure(error: unknown, name: string): PlayerError {
  if (error instanceof PlayerError) return error
  return new PlayerError('MEDIA_FAILURE', `"${name}" failed on an item`, { element: name, cause: error })
}

if (import.meta.vitest) {
  const { describe, test, expect, afterEach, vi } = import.meta.vitest
  const { Location } = await import('@/model/common/location')
  const { CONNECTION_CAPACITY } = await import('@/player/element/pad')
  const { collectingSink, flush, settle, testChain, testSource } = await import('@/player/testing/elements')
  const { inProcessWorker, RecordEngine, workerSink, workerTransform } = await import('@/player/testing/worker')
  type TestSource = import('@/player/testing/elements').TestSource
  type CollectingSink = import('@/player/testing/elements').CollectingSink
  type WorkerSink = import('@/player/testing/worker').WorkerSink
  type EngineClass = import('./host').EngineClass

  afterEach(() => {
    RecordEngine.created.length = 0
    RecordEngine.gate = undefined
  })

  const media = (n: number): PadOutput => ({
    type: 'packet',
    packet: { track: 'v1', location: new Location(0n, BigInt(n)), properties: [], payload: new Uint8Array([n]) },
  })
  const objects = (items: readonly PadOutput[]) =>
    items.map((item) => (item.type === 'packet' ? Number(item.packet.location.object) : item.type))

  async function workerChain(...sinks: ('main' | 'worker')[]) {
    const worker = new PlayerWorker(inProcessWorker())
    expect(await worker.start(new AbortController().signal)).toBe(true)

    const { chain, failures } = testChain(undefined, undefined, undefined, undefined, worker)
    let source: TestSource | undefined
    const consumers: (CollectingSink | WorkerSink)[] = []

    chain.addSource(testSource({ script: (s) => ((source = s), s.addPad(), s.emit('nomorepads')) }), {})
    chain.addTransform(workerTransform)

    for (const sink of sinks) {
      if (sink === 'main') chain.addSink(collectingSink({ setup: (k) => consumers.push(k) }))
      else chain.addSink({ ...workerSink, init: (c) => workerSink.init(c).then((k) => (consumers.push(k), k)) })
    }

    await chain.init(new AbortController().signal)

    const pad = source!.pads[0]!
    let written = 0

    /** Writes `count` packets one after the other, as a source does. */
    const write = async (count: number) => {
      for (let n = 0; n < count; n++) {
        await pad.write(media(n))
        written++
      }
    }

    return { chain, failures, consumers, write, written: () => written }
  }

  describe('PlayerWorker', () => {
    test('a worker pad holds what its engine writes until its consumer links, then items reach the engine in the worker, in order', async () => {
      const w = await workerChain('worker')
      const sink = w.consumers[0] as WorkerSink

      const writes = w.write(3 * CONNECTION_CAPACITY)
      await settle()

      // Held in the worker, and back up through the transform's input to the source
      expect(w.written()).toBeLessThan(3 * CONNECTION_CAPACITY)

      sink.link()
      await writes
      await vi.waitUntil(() => RecordEngine.created[0]?.items.length === 3 * CONNECTION_CAPACITY)

      expect(objects(RecordEngine.created[0]!.items)).toEqual([...Array(3 * CONNECTION_CAPACITY).keys()])
      expect([sink.received, w.failures]).toEqual([[], []])

      await w.chain.destroy()
    })

    test('a consumer that knows nothing about workers is bridged, with backpressure across', async () => {
      const w = await workerChain('main')
      const sink = w.consumers[0] as CollectingSink
      const gate = Promise.withResolvers<void>()
      sink.slow = () => gate.promise

      const writes = w.write(5 * CONNECTION_CAPACITY)
      await settle()

      expect(w.written()).toBeLessThan(5 * CONNECTION_CAPACITY)

      gate.resolve()
      await writes
      await vi.waitUntil(() => sink.items().length === 5 * CONNECTION_CAPACITY)

      expect(objects(sink.items())).toEqual([...Array(5 * CONNECTION_CAPACITY).keys()])
      expect(sink.items()[0]).toMatchObject({ packet: { location: expect.any(Location) } })

      await w.chain.destroy()
    })

    test('a consumer that links next to one that does not bridges both, and the linked one forwards from the main thread', async () => {
      const w = await workerChain('worker', 'main')
      const [linked, bridged] = w.consumers as [WorkerSink, CollectingSink]

      linked.link()
      await w.write(4)
      await vi.waitUntil(() => RecordEngine.created[0]?.items.length === 4)

      expect([objects(linked.received), objects(bridged.items())]).toEqual([
        [0, 1, 2, 3],
        [0, 1, 2, 3],
      ])
      expect(objects(RecordEngine.created[0]!.items)).toEqual([0, 1, 2, 3])

      await flush()
      await w.chain.destroy()
    })

    test('an engine whose constructor throws fails its element, and close() on it still settles', async () => {
      const Throws = class {
        constructor() {
          throw new Error('no engine')
        }
      } as unknown as EngineClass

      const worker = new PlayerWorker(inProcessWorker({ 'worker-throw': Throws }))
      await worker.start(new AbortController().signal)
      const failures: PlayerError[] = []
      const context = worker.context({ element: undefined, fail: (error) => failures.push(error) })!

      const engine = context.create<Engine & { close(): void }>('worker-throw')
      await vi.waitUntil(() => failures.length > 0)
      await engine.close()

      expect(failures).toMatchObject([{ code: 'MEDIA_FAILURE', element: 'worker-throw' }])
    })

    test('an error the worker throws after startup fails the player', async () => {
      const port = inProcessWorker()
      const worker = new PlayerWorker(port)
      await worker.start(new AbortController().signal)
      const failures: PlayerError[] = []
      worker.context({ element: undefined, fail: (error) => failures.push(error) })

      port.dispatchEvent(new Event('error'))

      expect(failures).toMatchObject([{ code: 'MEDIA_FAILURE', message: 'The player worker failed' }])
    })

    test('an element whose threads needs the worker fails init without one', async () => {
      const { chain } = testChain()
      chain.addSource(testSource({ script: (s) => (s.addPad(), s.emit('nomorepads')) }), {})
      chain.addSink(workerSink)

      await expect(chain.init(new AbortController().signal)).rejects.toMatchObject({
        code: 'UNSUPPORTED_SOURCE',
        element: 'worker-sink',
      })
    })
  })
}
