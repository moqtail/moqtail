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
 * One player on a new `<video>`, playing an MSF URI over the real client, with everything it
 * does recorded (`Records`). Tests get sessions from the `open` fixture in `test.ts`.
 */

import type { MOQtailClient } from '@/client/client'
import type { LocationFilter } from '@/model/parameter/message/location_filter'
import {
  MOQtailPlayer,
  parseMsfUrl,
  type CatalogElement,
  type MOQtailPlayerOptions,
  type MSEOptions,
  type MediaSourceElement,
  type MsfSource,
  type PlayerEvents,
  type PlayerState,
  type Rendition,
} from '@/player'
import { currentClient, factory } from '@/player/client_slot'
import { withRole } from '@/player/elements/catalog/selection'
import { TemplateTimeline } from '@/player/elements/mediatimeline/template'
import { recordClient } from './client'
import { Records, seconds, type Log, type SwitchRecord } from './records'
import { sourceOf, type TestSource } from './sources'

export interface OpenOptions {
  switchFrom?: boolean
  player?: MOQtailPlayerOptions
  sink?: Omit<MSEOptions, 'video'>
  /** Default true: `start()` and wait for `playing`. False: only `init()`. */
  start?: boolean
  /** The element's playback rate once playing, to get through VOD faster. */
  playbackRate?: number
}

const PLAYER_EVENTS: (keyof PlayerEvents)[] = [
  ...(['statechange', 'error', 'warning', 'playing', 'pause', 'stalled', 'ended', 'catalog'] as const),
  ...(['reconnecting', 'reconnected', 'timelineevent', 'autoplayblocked', 'renditionchange'] as const),
]
const DEFAULT_TIMEOUT = 20_000

/** The session whose player is creating SourceBuffers now: sessions start one at a time. */
let creating: Session | undefined
const addSourceBuffer = MediaSource.prototype.addSourceBuffer
MediaSource.prototype.addSourceBuffer = function (this: MediaSource, mimeType: string) {
  const buffer = addSourceBuffer.call(this, mimeType)
  creating?.sourceBuffers.push({ mimeType, buffer })
  return buffer
}

const ranges = (b: TimeRanges): [number, number][] => Array.from({ length: b.length }, (_, i) => [b.start(i), b.end(i)])

export class Session extends Records {
  /** Created by `begin()`. */
  player!: MOQtailPlayer
  readonly video: HTMLVideoElement
  /** The parsed URI. */
  readonly source: MsfSource
  /** The test source the URI names, when it's one in `sources.ts`. */
  readonly content: TestSource | undefined
  /** Every SourceBuffer the sink created, oldest first. The sink recreates them when it rebuilds. */
  readonly sourceBuffers: { mimeType: string; buffer: SourceBuffer }[] = []
  /** The last catalog the player applied. */
  catalog: CatalogElement | undefined
  #client: MOQtailClient | undefined
  #ownSeeks = 0
  /** The element as it was when the session closed, for the timeline. */
  #final: { buffered: [number, number][]; currentTime: number } | undefined
  #finalByRole: Record<'video' | 'audio', [number, number][]> | undefined
  #started = false
  readonly #roles = new Map<string, 'video' | 'audio'>()
  readonly #createClient = factory.createClient

  constructor(readonly uri: string) {
    const log: Log = { states: [], events: [], requests: [], data: [], objects: [], gaps: [], switches: [] }
    super(log, 0, (track) => this.#role(track))
    this.source = parseMsfUrl(uri)
    this.content = sourceOf(this.source.namespace.toUtf8Path().replace(/^\//, ''))
    this.video = document.createElement('video')
    this.video.muted = true
    document.body.append(this.video)
    this.video.addEventListener('seeking', () => {
      if (this.#started && this.#ownSeeks === 0)
        this.log.gaps.push({ kind: 'jump', detail: `to ${this.video.currentTime.toFixed(2)}`, at: performance.now() })
    })
  }

  /** Creates the player and starts it. Called by `open`. */
  async begin(options: OpenOptions = {}): Promise<void> {
    this.player = new MOQtailPlayer(options.player)
    creating = this
    for (const name of PLAYER_EVENTS) {
      this.player.on(name, (...args: unknown[]) => {
        this.log.events.push({ name, args, at: performance.now() } as never)
      })
    }
    this.player.on('statechange', (state, previous) => {
      this.log.states.push({ state, previous, at: performance.now() })
      if (state === 'playing') this.#started = true
    })
    this.player.on('catalog', (catalog) => (this.catalog = catalog))
    // Hooks the client before it connects; a client another session opened is hooked after init.
    factory.createClient = (clientOptions) => this.#attach(this.#createClient(clientOptions))
    this.player.source(this.uri, options.switchFrom === undefined ? {} : { switchFrom: options.switchFrom })
    this.player.sink(this.video, options.sink)
    try {
      await this.player.init()
      if (!this.#client) this.#attach(currentClient())
      if (options.start === false) return
      // start() resolves with play(), which waits for media at currentTime; bound it by the state instead.
      let failed: unknown
      this.player.start().catch((error: unknown) => (failed = error))
      await this.until(() => failed !== undefined || this.player.state === 'playing', 'state playing')
      if (failed !== undefined) throw failed
      if (options.playbackRate) this.video.playbackRate = options.playbackRate
    } finally {
      factory.createClient = this.#createClient
    }
  }

  get client(): MOQtailClient {
    if (!this.#client) throw new Error('The session has no client yet')
    return this.#client
  }

  /** Now, for `since()`. */
  mark(): number {
    return performance.now()
  }

  /** The records from `mark` on. */
  since(mark: number): Records {
    return new Records(this.log, mark, (track) => this.#role(track))
  }

  get videoSource(): MediaSourceElement {
    return this.#mediaSource('video')
  }

  get audioSource(): MediaSourceElement {
    return this.#mediaSource('audio')
  }

  /**
   * Buffered `[start, end]` pairs: `video.buffered` (where audio and video overlap), or one
   * role's newest SourceBuffer.
   */
  buffered(role?: 'video' | 'audio'): [number, number][] {
    if (!role) return this.#final?.buffered ?? ranges(this.video.buffered)
    const source = this.sourceBuffers.findLast((b) => b.mimeType.startsWith(role))
    if (!source) return []
    try {
      return ranges(source.buffer.buffered)
    } catch {
      return [] // removed from its MediaSource
    }
  }

  /** The end of the last buffered range. */
  liveEdge(): number {
    return this.buffered().at(-1)?.[1] ?? 0
  }

  /** Seconds buffered ahead of `currentTime`. */
  latency(): number {
    return this.liveEdge() - this.video.currentTime
  }

  /** The `template` timeline of the role's active track, as the catalog gives it. */
  timelineOf(role: 'video' | 'audio' = 'video'): TemplateTimeline | undefined {
    const active = this.#mediaSource(role)
      .getRenditions()
      .find((r) => r.active)
    const template = this.catalog?.tracks.find((t) => t.name === active?.id)?.template
    return template && new TemplateTimeline(template)
  }

  /**
   * The group of the role's active track holding `seconds` of media time: from its catalog
   * `template`, else from the source's GoP with group 0 at 0 s.
   */
  groupAt(seconds: number, role: 'video' | 'audio' = 'video'): bigint {
    const timeline = this.timelineOf(role)
    if (timeline) return timeline.timeToLocation(seconds * 1000)!.group
    const gop = this.content?.gop
    if (gop === undefined || role !== 'video') throw new Error(`No template or GoP for ${role} of ${this.uri}`)
    return BigInt(Math.floor(seconds / gop))
  }

  /** Seconds between keyframes of `track`, from the catalog's `keyframeSpacing`, else the source's GoP. */
  keyframeInterval(track: string): number {
    const entry = this.catalog?.tracks.find((t) => t.name === track)
    if (entry?.keyframeSpacing && entry.framerate) return entry.keyframeSpacing / entry.framerate
    if (this.content?.gop !== undefined) return this.content.gop
    throw new Error(`No keyframe interval known for ${track}`)
  }

  /** Polls `condition` every 50 ms. Fails early when the player errors. */
  async until(condition: () => boolean, what: string, timeout = DEFAULT_TIMEOUT): Promise<void> {
    const deadline = performance.now() + timeout
    while (!condition()) {
      if (this.player.state === 'error') throw new Error(`Player error while waiting for ${what}: ${this.#lastError()}`)
      if (performance.now() > deadline) throw new Error(`Timed out after ${seconds(timeout)} waiting for ${what}`)
      await new Promise((resolve) => setTimeout(resolve, 50))
    }
  }

  /** The arguments of the next `event` that passes `predicate`. */
  next<K extends keyof PlayerEvents>(
    event: K,
    predicate: (...args: Parameters<PlayerEvents[K]>) => boolean = () => true,
    timeout = DEFAULT_TIMEOUT,
  ): Promise<Parameters<PlayerEvents[K]>> {
    const { promise, resolve } = Promise.withResolvers<Parameters<PlayerEvents[K]>>()
    const listener = ((...args: Parameters<PlayerEvents[K]>) => {
      if (predicate(...args)) resolve(args)
    }) as PlayerEvents[K]
    this.player.on(event, listener)
    return withTimeout(promise, timeout, event).finally(() => this.player.off(event, listener))
  }

  state(state: PlayerState, timeout = DEFAULT_TIMEOUT): Promise<void> {
    return this.until(() => this.player.state === state, `state ${state}`, timeout)
  }

  /** Waits until `currentTime` has advanced `duration` seconds. */
  played(duration: number, timeout = DEFAULT_TIMEOUT + duration * 2000): Promise<void> {
    const target = this.video.currentTime + duration
    return this.until(() => this.video.currentTime >= target, `currentTime ${target.toFixed(2)}`, timeout)
  }

  ended(timeout?: number): Promise<void> {
    return this.state('ended', timeout)
  }

  /** `player.seek()`. Its `seeking` events aren't gaps. */
  async seek(mediaTime: number, timeout = DEFAULT_TIMEOUT): Promise<void> {
    this.#ownSeeks++
    try {
      await withTimeout(this.player.seek(mediaTime), timeout, `seek to ${mediaTime.toFixed(2)} to complete`)
    } finally {
      this.#ownSeeks--
    }
  }

  /** Seeks `offset` seconds from the live edge (negative is behind it). Returns the target. */
  async seekLive(offset: number): Promise<number> {
    const target = this.liveEdge() + offset
    await this.seek(target)
    return target
  }

  /**
   * Switches the video source and waits until the target is on screen. In a fast switching set
   * a rendition is a whole GoP ladder: `entered` is the track of it the switch requested first.
   */
  async switchTo(target: string | ((renditions: Rendition[]) => Rendition | undefined)): Promise<SwitchRecord> {
    const source = this.videoSource
    const renditions = source.getRenditions()
    const from = renditions.find((r) => r.active)?.id ?? '?'
    const to = typeof target === 'string' ? renditions.find((r) => r.id === target) : target(renditions)
    if (!to)
      throw new Error(
        `No rendition ${typeof target === 'string' ? target : 'matched'} in ${renditions.map((r) => r.id)}`,
      )
    const start = performance.now()
    const playhead = this.video.currentTime
    // Together, so a rejected switch doesn't leave the event wait unhandled.
    await Promise.all([source.switch(to.id), this.next('renditionchange', (_, r) => r.id === to.id)])
    const ms = performance.now() - start
    await this.until(() => this.video.videoHeight === to.height, `${to.id} on screen`)
    const entered = this.since(start).requestsOf('video')[0]?.track ?? to.id
    const record = { from, to: to.id, entered, ms, shownMs: performance.now() - start, playhead, at: start }
    this.log.switches.push(record)
    return record
  }

  /** A readable account of the session, printed when a test fails. */
  timeline(): string {
    const t = (ms: number) => seconds(ms - (this.log.states[0]?.at ?? 0))
    const lastObjects = new Map<string, string>()
    for (const o of this.log.objects) lastObjects.set(o.track, `${o.group}.${o.object}`)
    return [
      `session ${this.uri}`,
      `  states: ${this.log.states.map((s) => `${t(s.at)} ${s.state}`).join(', ')}`,
      `  requests: ${this.log.requests.map((r) => `${t(r.at)} ${r.kind}#${r.requestId ?? '-'} ${r.track} ${filter(r.locationFilter)}${r.error ? ` REJECTED ${r.error}` : ''}`).join(', ')}`,
      `  switches: ${this.log.switches.map((s) => `${s.from} > ${s.to}${s.entered !== s.to ? ` (via ${s.entered})` : ''} ${s.ms.toFixed(0)} ms`).join(', ')}`,
      `  gaps: ${this.log.gaps.map((g) => `${t(g.at)} ${g.kind} ${g.detail}`).join(', ')}`,
      `  errors: ${this.events.filter((e) => e.name === 'error' || e.name === 'warning').map((e) => String(e.args[0]))}`,
      `  buffered: ${this.buffered().map(([s, e]) => `${s.toFixed(2)}-${e.toFixed(2)}`)} at ${(this.#final?.currentTime ?? this.video.currentTime).toFixed(2)}`,
      `  video buffered: ${(this.#finalByRole?.video ?? this.buffered('video')).map(([s, e]) => `${s.toFixed(2)}-${e.toFixed(2)}`)}`,
      `  audio buffered: ${(this.#finalByRole?.audio ?? this.buffered('audio')).map(([s, e]) => `${s.toFixed(2)}-${e.toFixed(2)}`)}`,
      `  groups: ${[...new Set(this.log.objects.map((o) => o.track))].map((track) => `${track} ${this.groups(track).join(',')}`).join('; ')}`,
      `  last objects: ${[...lastObjects].map(([track, loc]) => `${track} ${loc}`).join(', ')}`,
    ].join('\n')
  }

  /** Destroys the player and checks it gave the element back as it was. */
  async close(): Promise<void> {
    factory.createClient = this.#createClient
    this.#final = { buffered: this.buffered(), currentTime: this.video.currentTime }
    this.#finalByRole = { video: this.buffered('video'), audio: this.buffered('audio') }
    if (creating === this) creating = undefined
    this.#learnRoles()
    await this.player.destroy()
    const detached = this.video.src === '' && this.video.srcObject === null
    this.video.remove()
    if (!detached) throw new Error(`destroy() left the video element attached: ${this.uri}`)
  }

  #attach(client: MOQtailClient): MOQtailClient {
    this.#client = client
    recordClient(client, this.log)
    return client
  }

  #mediaSource(role: 'video' | 'audio'): MediaSourceElement {
    const isVideo = (s: MediaSourceElement) => s.getRenditions().some((r) => r.height !== undefined)
    const source = this.player.mediaSources.find((s) => isVideo(s) === (role === 'video'))
    if (!source) throw new Error(`No ${role} media source in ${this.uri}`)
    return source
  }

  #role(track: string): 'video' | 'audio' | undefined {
    if (!this.#roles.has(track)) this.#learnRoles()
    return this.#roles.get(track)
  }

  /** From the catalog, as its element infers them; remembered, so roles still resolve once the player is destroyed. */
  #learnRoles(): void {
    for (const track of this.catalog?.tracks ?? []) {
      const role = withRole(track).role
      if (role === 'video' || role === 'audio') this.#roles.set(track.name, role)
    }
  }

  #lastError(): string {
    return String(this.events.findLast((e) => e.name === 'error')?.args[0] ?? 'unknown')
  }
}

/** `promise`, or a rejection naming `what` after `timeout` ms. */
function withTimeout<T>(promise: Promise<T>, timeout: number, what: string): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined
  const expired = new Promise<never>((_, reject) => {
    timer = setTimeout(() => reject(new Error(`Timed out after ${seconds(timeout)} waiting for ${what}`)), timeout)
  })
  return Promise.race([promise, expired]).finally(() => clearTimeout(timer))
}

function filter(f: LocationFilter | undefined): string {
  if (!f) return ''
  return `${f.startGroup ?? ''}${f.startObject !== undefined ? `.${f.startObject}` : ''}-${f.endGroup ?? ''}`
}
