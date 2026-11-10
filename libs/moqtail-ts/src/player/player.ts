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
 * `MOQtailPlayer`: one medium. It declares the chain, connects through the shared
 * client, follows the media sink's state, and forwards control calls to it.
 */

import type { MOQtailClient } from '@/client/client'
import type { MOQtailClientOptions, ObjectOrdering } from '@/client/types'
import type { SetupOptions } from '@/model/parameter/setup_option'
import type { SinkElement, SourceElement, TransformElement } from '@/player/element/base'
import { Chain } from '@/player/element/chain'
import {
  hasInterface,
  type ElementDefinition,
  type Identifiers,
  type SinkElementDefinition,
  type SourceElementDefinition,
} from '@/player/element/define'
import type { TimelineEvent } from '@/player/element/event_timeline'
import {
  ABR,
  MEDIA_SINK,
  MEDIA_SOURCE,
  type MediaSinkElement,
  type MediaSourceElement,
  type Rendition,
} from '@/player/element/interfaces'
import { selectSource } from '@/player/element/negotiation'
import { BUILT_IN_SOURCES } from '@/player/elements'
import type { CatalogElement } from '@/player/elements/catalog/element'
import { MSE, checkOptions } from '@/player/elements/mse'
import { rangeAt } from '@/player/elements/mse/gaps'
import type { MSEOptions } from '@/player/elements/mse/sink'
import { Emitter } from '@/util/emitter'
import { logger } from '@/util/logger'
import { acquire, check, clientKey, currentClient, release, type ClientOptions, type SlotMember } from './client_slot'
import { PlayerError } from './error'
import { parseMsfUrl } from './msf_url'
import {
  activePlugins,
  initPlugins,
  mergeSetupOptions,
  resolveFeatures,
  type AbrController,
  type BufferController,
  type Features,
  type Plugin,
  type PluginInstance,
} from './plugin'
import { canTransition, type PlayerState } from './state'

/** @public */
export interface MOQtailPlayerOptions {
  /** Shared by every player on the client, compared for `UNSUPPORTED_ORIGIN`. */
  client?: {
    setupOptions?: SetupOptions
    transportOptions?: MOQtailClientOptions['transportOptions']
    /** Default 5. */
    reconnectAttempts?: number
  }
  /** Entries made with `definePlugin`. Their setup parameters are shared; their features apply to this player. */
  plugins?: Plugin[]
  preferredLanguage?: string
  eventTimelines?: string[] | 'all'
  /** Default 10 000. */
  catalogTimeoutMs?: number
  /** Default 16 MiB. */
  maxCatalogBytes?: number
  /** How media sources order each request's objects. Default: a 150 ms window. */
  objectOrdering?: ObjectOrdering
}

/** @public The media-ui-extensions stream type (proposal 0010). */
export type StreamType = 'unknown' | 'live' | 'on-demand'

/** @public */
export interface PlayerEvents {
  statechange: (state: PlayerState, previous: PlayerState) => void
  error: (error: PlayerError) => void
  warning: (warning: PlayerError) => void
  playing: () => void
  pause: () => void
  stalled: () => void
  ended: () => void
  /** The catalog element, after the first catalog and every applied update. */
  catalog: (catalog: CatalogElement) => void
  reconnecting: (attempt: number) => void
  reconnected: () => void
  /** The client this player uses, once connected and again before each `reconnected`. */
  client: (client: MOQtailClient) => void
  /** A record of an opted-in event timeline, on arrival (`opts.eventTimelines`). */
  timelineevent: (event: TimelineEvent) => void
  autoplayblocked: () => void
  /** A source switched to another rendition. Not emitted for the one it starts with: read `active` from `getRenditions()`. */
  renditionchange: (source: MediaSourceElement, rendition: Rendition) => void
  /** A source's selected renditions were set (`selectRenditions`). */
  selectionchange: (source: MediaSourceElement) => void
  /** The catalog element's `streamType` changed. */
  streamtypechange: (type: StreamType) => void
}

const SINK_EVENTS: [string, PlayerState][] = [
  ['playing', 'playing'],
  ['pause', 'paused'],
  ['stalled', 'stalled'],
  ['waiting', 'stalled'],
  ['ended', 'ended'],
]
const STATE_EVENTS: Partial<Record<PlayerState, 'playing' | 'pause' | 'stalled' | 'ended'>> = {
  playing: 'playing',
  paused: 'pause',
  stalled: 'stalled',
  ended: 'ended',
}
const STAGES = ['source', 'transform', 'sink'] as const
type Stage = (typeof STAGES)[number]

/** @public */
export class MOQtailPlayer extends Emitter<PlayerEvents> {
  readonly options: Readonly<MOQtailPlayerOptions>
  readonly #clientOptions: ClientOptions
  readonly #clientKey: string
  /** `registerElement` definitions. */
  readonly #definitions: ElementDefinition[] = []
  readonly #plugins: readonly PluginInstance[]
  /** What `source()` adds; its element is picked in `init()`, once the active plugins are known. */
  readonly #declaredSources: { identifiers: Identifiers; options: object }[] = []
  readonly #chain: Chain
  /** Aborted by `destroy()`; its reason is what every later call rejects with. */
  readonly #abort = new AbortController()
  readonly #member: SlotMember = {
    reconnecting: (attempt) => {
      this.#sessionLost = true
      // `ended` only moves to reconnecting when a seek back needs the session
      if (this.#state !== 'ended' && canTransition(this.#state, 'reconnecting')) {
        this.#resume = this.#state
        this.#setState('reconnecting')
      }
      this.emit('reconnecting', attempt)
    },
    reconnected: () => {
      this.#sessionLost = false
      this.#useClient(currentClient())
      if (this.#state === 'reconnecting') this.#setState(this.#resume)
      this.emit('reconnected')
    },
    fail: (error) => {
      if (this.#state === 'ended') this.#lostError = error
      else this.#fail(error)
    },
  }
  #state: PlayerState = 'idle'
  /** The active plugins' features, from the first SERVER_SETUP. Kept across reconnects. */
  #features: Features = resolveFeatures([])
  #client: MOQtailClient | undefined
  /** Where `reconnecting` returns to; the media sink updates it meanwhile. */
  #resume: PlayerState = 'idle'
  #stage: Stage = 'source'
  #origin: string | undefined
  #sessionLost = false
  /** A session failure that arrived while `ended`; a seek that needs the session reports it. */
  #lostError: PlayerError | undefined
  #error: PlayerError | undefined
  #streamType: StreamType = 'unknown'
  #negotiated = false
  #bufferAttached = false
  #abrAttached = false
  #initializing: Promise<void> | undefined
  #starting: Promise<void> | undefined
  #destroying: Promise<void> | undefined

  /**
   * Runs each plugin's `init` and `setup`.
   * @throws PlayerError `UNSUPPORTED_ORIGIN` when two plugins set a setup parameter differently.
   * @throws TypeError for a plugin listed twice, or a config its `init` rejects.
   */
  constructor(options: MOQtailPlayerOptions = {}) {
    super()
    this.options = options
    this.#plugins = initPlugins(options.plugins ?? [])
    const setupOptions = mergeSetupOptions(options.client?.setupOptions, this.#plugins)
    this.#clientOptions = { ...options.client, ...(setupOptions && { setupOptions }) }
    this.#clientKey = clientKey(this.#clientOptions)
    this.#chain = new Chain({
      player: this,
      client: currentClient,
      definitions: () => this.#sourceDefinitions(),
      switchSources: () => this.#features.switchSources,
      warn: (warning) => {
        if (!this.#abort.signal.aborted && this.#state !== 'error') this.emit('warning', warning)
      },
      fail: (error) => this.#fail(error),
    })
    this.on('streamtypechange', (type) => {
      this.#streamType = type
      this.#control()
    })
    // A catalog update may add a source that provides ABR; a selection may pick a rendition manually
    this.on('catalog', () => this.#control())
    this.on('selectionchange', () => this.#control())
  }

  get state(): PlayerState {
    return this.#state
  }

  get sources(): readonly SourceElement[] {
    return this.#chain.sources
  }

  /** Sources that provide `MEDIA_SOURCE`, except those loaded by another media source. */
  get mediaSources(): readonly (SourceElement & MediaSourceElement)[] {
    return this.#chain.sources.filter(
      (s): s is SourceElement & MediaSourceElement =>
        hasInterface(s, MEDIA_SOURCE) && !this.#chain.holdersOf(s).some((h) => hasInterface(h, MEDIA_SOURCE)),
    )
  }

  /** The first sink in chain order that provides `MEDIA_SINK`. */
  get mediaSink(): (SinkElement & MediaSinkElement) | undefined {
    return this.#chain.sinks.find((s) => hasInterface(s, MEDIA_SINK))
  }

  /** The shared client while this player holds it; a reconnect replaces it (see the `client` event). */
  get client(): MOQtailClient | undefined {
    return this.#client
  }

  /** The active plugins' buffer controller while it's attached. */
  get bufferController(): BufferController | undefined {
    return this.#bufferAttached ? this.#features.bufferController : undefined
  }

  /** The active plugins' ABR controller while it's attached. */
  get abrController(): AbrController | undefined {
    return this.#abrAttached ? this.#features.abrController : undefined
  }

  /**
   * Whether `source` switches renditions automatically: it provides `ABR`, or an ABR controller can
   * drive it, since the stream is live and no media source provides `ABR`. A rendition picked
   * manually detaches the controller but leaves this true, so selecting every rendition again resumes it.
   */
  isAbrAvailable(source: SourceElement): boolean {
    return hasInterface(source, ABR) || (this.#features.abrController !== undefined && this.#abrDrivable())
  }

  /** Registers an element on this player only. Call it before `source()`. */
  registerElement(definition: ElementDefinition): this {
    this.#order('source', 'registerElement')
    if (this.#origin !== undefined) throw new TypeError('registerElement() must be called before source()')
    this.#definitions.push(definition)
    return this
  }

  /**
   * Adds a top-level source for a moqt URI. `init()` picks its element, once the peer's SETUP
   * decided which plugins are active, and gives it `{ ...options, source }`.
   * @throws PlayerError `UNSUPPORTED_SOURCE` for a URI that can't be used here, `UNSUPPORTED_ORIGIN`
   * for another origin or client setup; the player stays `idle`.
   */
  source(uri: string, options: { switchFrom?: boolean; [key: string]: unknown } = {}): this {
    this.#order('source', 'source')
    const source = parseMsfUrl(uri)
    if (source.connection === 'q')
      throw new PlayerError('UNSUPPORTED_SOURCE', 'connection=q is not available in browsers')
    if (this.#origin !== undefined && this.#origin !== source.url) {
      throw new PlayerError('UNSUPPORTED_ORIGIN', `This player already uses ${this.#origin}`)
    }
    check(source.url, this.#clientKey)
    const identifiers = { ...source } // a type literal, so it fits `Identifiers`' index signature
    this.#declaredSources.push({ identifiers, options })
    this.#origin = source.url
    return this
  }

  transform(element: TransformElement): this {
    this.#order('transform', 'transform')
    this.#chain.addTransform(element)
    this.#stage = 'transform'
    return this
  }

  /** A video element gets the `MSE` sink with `options`. */
  sink(video: HTMLVideoElement, options?: Omit<MSEOptions, 'video'>): this
  sink(definition: SinkElementDefinition, options?: Record<string, unknown>): this
  sink(target: HTMLVideoElement | SinkElementDefinition, options: Record<string, unknown> = {}): this {
    this.#order('sink', 'sink')
    if (typeof HTMLVideoElement !== 'undefined' && target instanceof HTMLVideoElement) {
      const sinkOptions = { ...options, video: target }
      checkOptions(sinkOptions)
      this.#chain.addSink(MSE, sinkOptions)
    } else if ('init' in target && target.type === 'sink') this.#chain.addSink(target, options)
    else throw new TypeError('sink() takes an HTMLVideoElement or a sink element definition')
    this.#stage = 'sink'
    return this
  }

  /** Connects and negotiates the chain; rejects with `UNSUPPORTED_SOURCE` when no element accepts a source. Idempotent. */
  init(): Promise<void> {
    if (this.#abort.signal.aborted) return Promise.reject(this.#abort.signal.reason)
    const origin = this.#origin
    if (origin === undefined) return Promise.reject(new TypeError('source() must be called before init()'))
    return (this.#initializing ??= this.#init(origin))
  }

  /** `init()`, then `play()`. Idempotent. */
  start(): Promise<void> {
    if (this.#abort.signal.aborted || this.#origin === undefined) return this.init()
    return (this.#starting ??= this.init().then(() => this.play()))
  }

  /** A `NotAllowedError` moves the player to `paused` and emits `autoplayblocked`. */
  async play(): Promise<void> {
    const sink = this.#sink()
    try {
      await sink.play()
    } catch (error) {
      if (error instanceof Error && error.name === 'NotAllowedError') {
        this.#follow('paused')
        this.emit('autoplayblocked')
      }
      throw error
    }
  }

  async pause(): Promise<void> {
    await this.#sink().pause()
  }

  /** Seconds. Resolves or rejects as the media sink's `seek()` does. */
  async seek(mediaTime: number): Promise<void> {
    const sink = this.#sink()
    if (this.#state === 'ended' && this.#sessionLost && rangeAt(mediaTime, sink.buffered) === -1) {
      this.#resume = 'ended'
      this.#setState('reconnecting')
      if (this.#lostError) {
        this.#fail(this.#lostError)
        throw this.#lostError
      }
    }
    await sink.seek(mediaTime)
  }

  setPlaybackRate(rate: number): void {
    this.#sink().setPlaybackRate(rate)
  }

  /** Aborts in-flight work, destroys sinks then sources, and releases the client last. Callable in any state. */
  destroy(): Promise<void> {
    return (this.#destroying ??= this.#destroy())
  }

  async #init(origin: string): Promise<void> {
    this.#setState('connecting')
    try {
      const client = await acquire(this.#member, origin, this.#clientOptions)
      this.#abort.signal.throwIfAborted()
      this.#useClient(client)
      this.#features = resolveFeatures(activePlugins(this.#plugins, client.serverSetup))
      for (const { identifiers, options } of this.#declaredSources) {
        const definition = selectSource(this.#sourceDefinitions(), identifiers)
        if (!definition) throw new PlayerError('UNSUPPORTED_SOURCE', `No element accepts track "${identifiers.track}"`)
        this.#chain.addSource(definition, identifiers, options)
      }
      const outcome = await this.#chain.init(this.#abort.signal)
      if (this.#error) throw this.#error // failed while negotiating, e.g. the session couldn't be recovered
      this.#watchSink()
      this.#setState(outcome)
      this.#negotiated = true
      this.#control()
    } catch (cause) {
      const error =
        cause instanceof PlayerError
          ? cause
          : new PlayerError('UNSUPPORTED_SOURCE', 'An element failed to initialise', { cause })
      this.#fail(error)
      throw error
    }
  }

  async #destroy(): Promise<void> {
    this.#abort.abort(new PlayerError('ABORTED', 'The player was destroyed'))
    this.#client = undefined
    this.#control()
    const teardown = this.#chain.destroy()
    const released = release(this.#member, teardown)
    try {
      await teardown
    } finally {
      await released
      this.#setState('destroyed')
    }
  }

  /** Built-in sources last, so they lose probe ties. */
  #sourceDefinitions(): SourceElementDefinition[] {
    return [...this.#definitions, ...this.#features.elements, ...BUILT_IN_SOURCES].filter(
      (d): d is SourceElementDefinition => d.type === 'source',
    )
  }

  /** Builder calls go source → transform → sink, all before `init()`. Mistakes are programming errors. */
  #order(stage: Stage, method: string): void {
    if (this.#abort.signal.aborted) throw this.#abort.signal.reason
    if (this.#initializing) throw new TypeError(`${method}() can't be called after init()`)
    if (STAGES.indexOf(stage) < STAGES.indexOf(this.#stage)) {
      throw new TypeError(`${method}() can't be called after ${this.#stage}()`)
    }
  }

  #sink(): SinkElement & MediaSinkElement {
    if (this.#abort.signal.aborted) throw this.#abort.signal.reason
    const sink = this.mediaSink
    if (!sink) throw new PlayerError('NO_MEDIA_SINK', 'The player has no media sink')
    return sink
  }

  #watchSink(): void {
    const sink = this.mediaSink
    if (!sink) return
    for (const [event, state] of SINK_EVENTS) {
      sink.addEventListener(
        event,
        () => {
          // The element waits before it first plays and during every seek, and pauses right before `ended`
          if (event === 'waiting' && (sink.seeking || this.#state === 'ready')) return
          if (event === 'pause' && sink.ended) return
          this.#follow(state)
        },
        { signal: this.#abort.signal },
      )
    }
  }

  /**
   * Attaches the buffer controller once a negotiated stream is live, and the ABR controller while
   * it's also drivable and no rendition is picked manually; detaches each otherwise.
   */
  #control(): void {
    const sink = this.mediaSink
    if (!sink) return
    const buffer = this.#features.bufferController
    const live = this.#live()
    if (buffer && live !== this.#bufferAttached) {
      this.#bufferAttached = live
      if (live) buffer.attach(this, sink)
      else buffer.detach()
    }
    const abr = this.#features.abrController
    const drive = this.#abrDrivable() && !this.mediaSources.some(isPinned)
    if (abr && drive !== this.#abrAttached) {
      this.#abrAttached = drive
      if (drive) abr.attach(this, sink)
      else abr.detach()
    }
  }

  #live(): boolean {
    return this.#negotiated && this.#streamType === 'live' && !this.#abort.signal.aborted
  }

  /** Live, and no media source switches renditions itself. */
  #abrDrivable(): boolean {
    return this.#live() && !this.mediaSources.some((s) => hasInterface(s, ABR))
  }

  #useClient(client: MOQtailClient): void {
    this.#client = client
    this.emit('client', client)
  }

  /** A state the media sink reports. Taken only where the table allows it; while reconnecting it's where we return to. */
  #follow(next: PlayerState): void {
    if (this.#abort.signal.aborted) return
    if (this.#state === 'reconnecting') this.#resume = next
    else if (canTransition(this.#state, next)) this.#setState(next)
  }

  /** Emits `error`, moves to `error`, and pauses the media sink. Elements stay and their signals don't abort. */
  #fail(error: PlayerError): void {
    if (this.#abort.signal.aborted) return
    if (!canTransition(this.#state, 'error')) {
      logger.warn('MOQtailPlayer', `error while ${this.#state}`, error)
      return
    }
    this.#error = error
    this.emit('error', error)
    this.#setState('error')
    this.mediaSink?.pause().catch(() => {})
  }

  #setState(next: PlayerState): void {
    const previous = this.#state
    if (!canTransition(previous, next, this.#resume)) {
      const message = `Illegal player state transition ${previous} → ${next}`
      if (import.meta.vitest) throw new Error(message)
      logger.error('MOQtailPlayer', message)
      return
    }
    this.#state = next
    this.emit('statechange', next, previous)
    const event = STATE_EVENTS[next]
    if (event) this.emit(event)
  }
}

/** A rendition picked manually: one of several selected, as `SstsSource` pins one. */
function isPinned(source: MediaSourceElement): boolean {
  const renditions = source.getRenditions()
  return renditions.length > 1 && renditions.filter((r) => r.selected).length === 1
}

if (import.meta.vitest) {
  const { describe, test, expect, beforeEach, afterEach } = import.meta.vitest
  const { fakeClients } = await import('@/player/testing/client')
  const { abrPlugin, bufferPlugin, testSource, fakeMediaSink, flush } = await import('@/player/testing/elements')
  const { definePlugin } = await import('./plugin')
  const { CapabilityCode } = await import('@/player/element/define')
  const { SetupOptions } = await import('@/model/parameter/setup_option')
  const { Setup } = await import('@/model/control/setup')
  type Feature = import('./plugin').Feature
  type Sink = import('@/player/testing/elements').FakeMediaSink
  type Source = import('@/player/testing/elements').TestSource

  const URL = 'moqt://relay.example.com/app#msf:ns--catalog'
  const media = testSource({ name: 'media', script: (s) => (s.addPad(), s.emit('nomorepads')) })
  const make = (options: MOQtailPlayerOptions = {}) =>
    new MOQtailPlayer(options).registerElement(media).source(URL).sink(fakeMediaSink)
  const record = (player: MOQtailPlayer) => {
    const log: string[] = []
    player.on('statechange', (state) => log.push(state))
    return log
  }
  const code = (code: string) => expect.objectContaining({ code })
  let fakes: ReturnType<typeof fakeClients>
  beforeEach(() => {
    delete (globalThis as Record<symbol, unknown>)[Symbol.for('moqtail.client')]
    fakes = fakeClients()
  })
  afterEach(() => fakes.restore())

  describe('MOQtailPlayer', () => {
    test('source() fails synchronously and leaves the player idle', () => {
      const player = new MOQtailPlayer()
      expect(() => player.source('https://example.com')).toThrow(code('UNSUPPORTED_SOURCE'))
      expect(() => player.source(`${URL}&connection=q`)).toThrow(code('UNSUPPORTED_SOURCE'))
      player.registerElement(media).source(URL)
      expect(() => player.source('moqt://other.example.com/app#msf:ns--catalog')).toThrow(code('UNSUPPORTED_ORIGIN'))
      expect(player.state).toBe('idle')
    })

    test('builder calls out of order are TypeErrors', () => {
      const player = make()
      expect(() => player.source(URL)).toThrow(TypeError)
      expect(() => player.transform({} as TransformElement)).toThrow(TypeError)
      expect(() => player.registerElement(media)).toThrow(TypeError)
      expect(() => player.sink({} as HTMLVideoElement)).toThrow(TypeError)
      void player.init()
      expect(() => player.sink(fakeMediaSink)).toThrow(TypeError)
    })

    test('init and start are idempotent, and the state follows the sink and the session', async () => {
      const player = make()
      const log = record(player)
      expect(player.init()).toBe(player.init())
      expect(player.start()).toBe(player.start())
      await player.start()
      const sink = player.mediaSink as Sink
      expect([sink.calls, fakes.clients.length]).toEqual([['play'], 1])
      sink.dispatch('waiting') // waiting before the first `playing` isn't a stall
      sink.dispatch('playing')
      sink.seeking = true
      sink.dispatch('waiting') // nor is waiting for a seek
      sink.seeking = false
      for (const event of ['pause', 'waiting', 'playing']) sink.dispatch(event)
      fakes.clients[0]!.drop()
      sink.dispatch('waiting') // ran dry while reconnecting: that's where it returns to
      await flush()
      sink.ended = true
      sink.dispatch('pause') // the element pauses right before `ended`
      sink.dispatch('ended')
      await player.seek(0)
      sink.dispatch('playing')
      await player.destroy()
      expect(log).toEqual([
        ...['connecting', 'ready', 'playing', 'paused', 'stalled', 'playing', 'reconnecting', 'stalled'],
        ...['ended', 'playing', 'destroyed'],
      ])
      expect(fakes.clients[1]!.disconnected).toBe(true)
    })

    test('fatal errors reject init or move to error and pause; a lost session while ended waits for a seek', async () => {
      // Nothing to play at all, such as an empty isComplete catalog: ended, not an error
      const nothing = new MOQtailPlayer().registerElement(testSource({ script: (s) => s.emit('nomorepads') }))
      await nothing.source(URL).sink(fakeMediaSink).init()
      expect(nothing.state).toBe('ended')

      const loc = [{ direction: 'out' as const, code: CapabilityCode.PACKAGING, value: 'loc' }]
      const unlinked = testSource({ script: (s) => (s.addPad(loc), s.emit('nomorepads')) })
      const empty = new MOQtailPlayer().registerElement(unlinked)
      const errors: PlayerError[] = []
      empty.on('error', (e) => errors.push(e))
      await expect(empty.source(URL).sink(fakeMediaSink).init()).rejects.toMatchObject({ code: 'NO_MEDIA_SINK' })
      expect([empty.state, errors]).toEqual(['error', [code('NO_MEDIA_SINK')]])

      // Picked once the plugins are known, so it's init() that rejects
      const unplayable = new MOQtailPlayer().source('moqt://relay.example.com/app#msf:ns--video').sink(fakeMediaSink)
      await expect(unplayable.init()).rejects.toMatchObject({ code: 'UNSUPPORTED_SOURCE' })
      await unplayable.destroy()

      const player = make()
      await player.init()
      const source = player.sources[0] as Source
      source.dropPad(source.pads[0]!, true)
      expect([player.state, (player.mediaSink as Sink).calls]).toEqual(['error', ['pause']])
      await Promise.all([nothing.destroy(), empty.destroy(), player.destroy()])

      fakes.restore()
      fakes = fakeClients((i) => (i === 0 ? Promise.resolve() : Promise.reject(new Error('gone'))))
      const ended = make({ client: { reconnectAttempts: 1 } })
      await ended.init()
      ;(ended.mediaSink as Sink).dispatch('ended')
      fakes.clients[0]!.drop()
      await flush()
      expect(ended.state).toBe('ended')
      await expect(ended.seek(0)).rejects.toMatchObject({ code: 'SESSION_LOST' })
      expect(ended.state).toBe('error')
      await ended.destroy()
    })

    test('a session lost for good while connecting rejects init()', async () => {
      fakes.restore()
      fakes = fakeClients((i) => (i === 0 ? Promise.resolve() : Promise.reject(new Error('gone'))))
      const late = Promise.withResolvers<void>()
      const slow = testSource({ script: (s) => void (s.addPad(), late.promise.then(() => s.emit('nomorepads'))) })
      const player = new MOQtailPlayer({ client: { reconnectAttempts: 1 } }).registerElement(slow).source(URL)
      const init = player.sink(fakeMediaSink).init()
      await flush()
      fakes.clients[0]!.drop()
      await flush()
      late.resolve()
      await expect(init).rejects.toMatchObject({ code: 'SESSION_LOST' })
      expect(player.state).toBe('error')
      await player.destroy()
    })

    test('destroy() works in every state, and every later call rejects', async () => {
      const idle = make()
      const connecting = make()
      const pending = connecting.init()
      await idle.destroy()
      await connecting.destroy()
      await expect(pending).rejects.toMatchObject({ code: 'ABORTED' })
      for (const player of [idle, connecting]) {
        expect(player.state).toBe('destroyed')
        expect(() => player.source(URL)).toThrow(code('ABORTED'))
        await expect(player.play()).rejects.toMatchObject({ code: 'ABORTED' })
        await expect(player.init()).rejects.toMatchObject({ code: 'ABORTED' })
      }
      const reconnecting = make()
      await reconnecting.init()
      fakes.clients.at(-1)!.drop()
      expect(reconnecting.state).toBe('reconnecting')
      await reconnecting.destroy()
      expect(reconnecting.state).toBe('destroyed')
    })

    test('control calls need a media sink; a blocked autoplay pauses and rejects start()', async () => {
      const bare = new MOQtailPlayer()
      await expect(bare.play()).rejects.toMatchObject({ code: 'NO_MEDIA_SINK' })
      expect(() => bare.setPlaybackRate(2)).toThrow(code('NO_MEDIA_SINK'))

      const player = make()
      let blocked = 0
      player.on('autoplayblocked', () => blocked++)
      await player.init()
      const sink = player.mediaSink as Sink
      sink.playResult = () => Promise.reject(new DOMException('gesture needed', 'NotAllowedError'))
      await expect(player.start()).rejects.toMatchObject({ name: 'NotAllowedError' })
      expect([player.state, blocked]).toEqual(['paused', 1])
      player.setPlaybackRate(1.5)
      await player.pause()
      expect(sink.calls).toEqual(['play', 'rate 1.5', 'pause'])
      await player.destroy()
    })

    test('the buffer controller is attached once a negotiated stream is live, and detached on-demand and on destroy()', async () => {
      const calls: string[] = []
      const buffer: BufferController = { attach: () => calls.push('attach'), detach: () => calls.push('detach') }
      const player = make({ plugins: [bufferPlugin(buffer)] })
      const plain = make()
      player.emit('streamtypechange', 'live')
      plain.emit('streamtypechange', 'live')
      expect(player.bufferController).toBeUndefined()
      await Promise.all([player.init(), plain.init()])
      expect([player.bufferController, plain.bufferController]).toEqual([buffer, undefined])
      player.emit('streamtypechange', 'on-demand')
      expect(player.bufferController).toBeUndefined()
      player.emit('streamtypechange', 'live')
      await Promise.all([player.destroy(), plain.destroy()])
      expect(calls).toEqual(['attach', 'detach', 'attach', 'detach'])
    })

    test('the ABR controller is attached while live, and detached on-demand, on destroy(), and while a source provides ABR', async () => {
      const calls: string[] = []
      const controller: AbrController = { attach: () => calls.push('attach'), detach: () => calls.push('detach') }
      const player = make({ plugins: [abrPlugin(controller)] })
      const switching = testSource({ name: 'switching', media: true, abr: true, script: (s) => s.emit('nomorepads') })
      const withAbr = new MOQtailPlayer({ plugins: [abrPlugin(controller)] })
        .registerElement(switching)
        .source(URL)
        .sink(fakeMediaSink)
      for (const p of [player, withAbr]) p.emit('streamtypechange', 'live')
      await Promise.all([player.init(), withAbr.init()])
      expect([player.abrController, withAbr.abrController]).toEqual([controller, undefined])
      // It drives no source of its own, but stays available to the one that switches itself
      expect([player.isAbrAvailable(player.sources[0]!), withAbr.isAbrAvailable(withAbr.sources[0]!)]).toEqual([
        true,
        true,
      ])
      player.emit('streamtypechange', 'on-demand')
      expect([player.abrController, player.isAbrAvailable(player.sources[0]!)]).toEqual([undefined, false])
      player.emit('streamtypechange', 'live')
      await Promise.all([player.destroy(), withAbr.destroy()])
      expect(calls).toEqual(['attach', 'detach', 'attach', 'detach'])
    })

    test('two players share one client and its reconnects; a different setup is UNSUPPORTED_ORIGIN', async () => {
      const [a, b] = [make(), make()]
      const other = make({ client: { reconnectAttempts: 2 } }) // checked on connect, nothing is live yet
      await Promise.all([a.init(), b.init()])
      await expect(other.init()).rejects.toMatchObject({ code: 'UNSUPPORTED_ORIGIN' })
      expect(() => new MOQtailPlayer().registerElement(media).source('moqt://x.example.com/a#msf:ns--catalog')).toThrow(
        code('UNSUPPORTED_ORIGIN'),
      )
      const events: string[] = []
      for (const [name, player] of [
        ['a', a],
        ['b', b],
      ] as const) {
        player.on('reconnecting', (n) => events.push(`${name} reconnecting ${n}`))
        player.on('reconnected', () => events.push(`${name} reconnected`))
        player.on('client', () => events.push(`${name} client`))
      }
      expect(a.client).toBe(fakes.clients[0])
      fakes.clients[0]!.drop()
      await flush()
      expect(events).toEqual([
        ...['a reconnecting 1', 'b reconnecting 1'],
        ...['a client', 'a reconnected', 'b client', 'b reconnected'],
      ])
      expect(a.client).toBe(fakes.clients[1])
      await a.destroy()
      expect(fakes.clients[1]!.disconnected).toBe(false)
      await b.destroy()
      expect(fakes.clients[1]!.disconnected).toBe(true)
      await other.destroy()
    })

    test("plugins: conflicting setup throws, setup joins the key, and only active plugins' elements are picked, top-level ones too", async () => {
      const offering = (name: string, algorithm: number, features: () => Feature[] = () => []) =>
        definePlugin({
          name,
          setup: (_, setup) => {
            setup.addSstsAlgorithms([algorithm])
            return true
          },
          negotiated: (_, serverSetup) => serverSetup.setupOptions.length > 0,
          features,
        })()
      expect(() => new MOQtailPlayer({ plugins: [offering('one', 1), offering('two', 2)] })).toThrow(
        code('UNSUPPORTED_ORIGIN'),
      )

      // Loads what the probe picks for a video track, as the catalog does
      const loadsVideo = (name: string, priority = 100) =>
        testSource({
          name,
          priority,
          probe: (ids) => ids.track === 'catalog',
          script: async (s) => {
            const picked = s.ctx.probe({ track: 'video' })
            if (picked) await s.ctx.load(picked)
            s.addPad()
            s.emit('nomorepads')
          },
        })
      const top = loadsVideo('top')
      const video = testSource({ name: 'video', probe: (ids) => ids.track === 'video' })
      const plugins = () => [
        offering('plugin', 1, () => [
          { type: 'element', item: loadsVideo('plugin-top', 50) },
          { type: 'element', item: video },
        ]),
      ]
      // The fake relay sends no setup parameters: the plugin is inactive, even for the top-level track
      const inactive = new MOQtailPlayer({ plugins: plugins() }).registerElement(top).source(URL).sink(fakeMediaSink)
      await inactive.init()
      expect(inactive.sources.map((s) => s.definition.name)).toEqual(['top'])
      await inactive.destroy()

      fakes.restore()
      fakes = fakeClients(undefined, new Setup(new SetupOptions().addSstsAlgorithms([1]).build()))
      const player = new MOQtailPlayer({ plugins: plugins() }).registerElement(top).source(URL).sink(fakeMediaSink)
      await player.init()
      expect(player.sources.map((s) => s.definition.name).sort()).toEqual(['plugin-top', 'video'])
      expect(() => new MOQtailPlayer().registerElement(top).source(URL)).toThrow(code('UNSUPPORTED_ORIGIN'))
      await player.destroy()
    })
  })
}
