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
 * The worker's side: the engines it creates by element name, and the outputs of worker pads.
 * An output holds what its engine writes until the main thread sets its route: linked to
 * engines in this worker, or bridged to the pad's mirror on the main thread.
 */

import * as Comlink from 'comlink'
import { cloned, closeFrame, Connection, type PadEvent, type PadItem, type PadOutput } from '@/player/element/pad'

/** @public What an engine's constructor gets, in the worker or from its element on the main thread. */
export interface EngineInit<O = unknown> {
  /** Write to the `outputs` given to `WorkerContext.create`, in order. Each resolves once there's room. */
  readonly outputs: readonly ((item: PadOutput) => Promise<void>)[]
  readonly options: O
  /** Tells the element something, fire and forget. */
  readonly report: (report: any) => void
}

/**
 * @public
 * The part of an element that can run in the worker, driven only by calls. Items arrive in order
 * per pad, from a linked worker pad or forwarded by the element, keyed by the role of their pad.
 */
export interface Engine {
  /** The pad's next item waits for the returned promise. */
  receive(item: PadItem, role: string): void | Promise<void>
  /** In order with the pad's items. An engine with outputs writes on what it passes on. */
  event(event: PadEvent, role: string): void | Promise<void>
}

/** @public */
export type EngineClass = new (init: EngineInit<any>) => Engine

export interface MainSide {
  /** Writes `item` on the output's mirror pad. */
  bridge(output: number, item: PadOutput): Promise<void>
  fail(engine: number, error: unknown): void
}

export interface Link {
  engine: number
  role: string
}

export class WorkerHost {
  readonly #classes: ReadonlyMap<string, EngineClass>
  readonly #engines = new Map<number, Engine>()
  readonly #outputs = new Map<number, Output>()
  #main: Comlink.Remote<MainSide> | undefined

  constructor(classes: ReadonlyMap<string, EngineClass>) {
    this.#classes = classes
  }

  /** The worker's global names, and `element:<name>` for each engine it creates. */
  environment(): string[] {
    const names = new Set<string>()
    for (
      let scope: object | null = globalThis;
      scope && scope !== Object.prototype;
      scope = Object.getPrototypeOf(scope)
    ) {
      for (const name of Object.getOwnPropertyNames(scope)) names.add(name)
    }
    return [...names, ...[...this.#classes.keys()].map((name) => `element:${name}`)]
  }

  connect(main: MainSide): void {
    // Comlink hands a proxied object over as a remote
    this.#main = main as unknown as Comlink.Remote<MainSide>
  }

  create(
    id: number,
    name: string,
    port: MessagePort,
    outputs: number[],
    options: unknown,
    report: ((report: unknown) => void) | undefined,
  ): void {
    const Class = this.#classes.get(name)
    if (!Class) throw new TypeError(`The worker has no engine for "${name}"`)
    const writers = outputs.map((key) => {
      const output = new Output(id, (error) => void this.#main?.fail(id, error))
      this.#outputs.set(key, output)
      return (item: PadOutput) => output.write(item)
    })
    const engine = new Class({ outputs: writers, options, report: report ?? (() => {}) })
    this.#engines.set(id, engine)
    Comlink.expose(engine, port)
  }

  route(key: number, route: 'bridge' | Link[]): void {
    const output = this.#outputs.get(key)
    if (!output) return
    if (route === 'bridge') {
      output.route((item) => this.#main!.bridge(key, item))
      return
    }
    output.route(async (item) => {
      await Promise.all(route.map((link, i) => this.#deliver(link, i === 0 ? item : cloned(item))))
    })
  }

  /** Its element already closed the engine. */
  release(id: number): void {
    this.#engines.delete(id)
    for (const [key, output] of this.#outputs) {
      if (output.producer !== id) continue
      output.close()
      this.#outputs.delete(key)
    }
  }

  async #deliver(link: Link, item: PadOutput): Promise<void> {
    const engine = this.#engines.get(link.engine)
    if (!engine) {
      closeFrame(item)
      return
    }
    try {
      if (item.type === 'event') await engine.event(item.event, link.role)
      else await engine.receive(item, link.role)
    } catch (error) {
      void this.#main?.fail(link.engine, error)
    }
  }
}

/** Holds up to `CONNECTION_CAPACITY` writes, then makes the engine wait, until its route is set. */
class Output {
  readonly producer: number
  readonly #route = Promise.withResolvers<(item: PadOutput) => Promise<void>>()
  readonly #connection: Connection

  constructor(producer: number, onError: (error: unknown) => void) {
    this.producer = producer
    this.#connection = new Connection(async (item) => (await this.#route.promise)(item), onError)
  }

  write(item: PadOutput): Promise<void> {
    return this.#connection.push(item)
  }

  route(deliver: (item: PadOutput) => Promise<void>): void {
    this.#route.resolve(deliver)
  }

  /** An item waiting for the route is dropped too. */
  close(): void {
    this.#route.resolve(async (item) => closeFrame(item))
    this.#connection.close()
  }
}
