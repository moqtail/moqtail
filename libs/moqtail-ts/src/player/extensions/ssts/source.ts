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
 * `SstsSource`: one switching set under SSTS. One child media source per track, of the
 * definition the probe picked for the set (`child`), subscribed into the set through request
 * hooks; the relay picks which track streams, and this source forwards whichever arrives.
 * Nothing here is specific to a packaging.
 */

import type { MOQtailClient } from '../../../client/client'
import type { SubscribeOptions } from '../../../client/types'
import { SwitchingSetAssignment } from '../../../model/parameter/message/switching_set_assignment'
import { SourceElement, type BaseElement } from '../../element/base'
import type { ElementContext } from '../../element/context'
import { CapabilityCode, defineElement } from '../../element/define'
import {
  MEDIA_SOURCE,
  type CatalogTrack,
  type MediaSourceElement,
  type Rendition,
  type RequestHooks,
  type SwitchingSetOptions,
} from '../../element/interfaces'
import type { Pad, PadItem } from '../../element/pad'
import { PlayerError } from '../../error'
import { formatMsfUrl } from '../../msf_url'
import { bitrateOf, renditionId, trackKey } from '../../elements/cmaf/track'
import { Forwarder, type Child } from './forward'
import { sstsAlgorithms } from './index'

/** The next switching set ID per client: players can share one, and equal IDs would merge their sets. */
const nextSetId = new WeakMap<MOQtailClient, bigint>()

/** @public The switching set source of the `SSTS` extension. */
export class SstsSource extends SourceElement implements MediaSourceElement {
  readonly pad: Pad
  /** The negotiated algorithms (`sstsAlgorithms`), read at init. */
  readonly algorithms: readonly bigint[]
  #algorithm: bigint
  readonly #children = new Map<string, Child>()
  #tracks: readonly CatalogTrack[]
  #selected: Set<string>
  /** Automatic switching paused: `activate` is 0. */
  #paused = false
  readonly #forward: Forwarder
  #setId: { client: MOQtailClient; id: bigint } | undefined
  #algorithmCalls = 0
  #algorithmLock = Promise.resolve()

  constructor(
    context: ElementContext,
    readonly options: SwitchingSetOptions,
  ) {
    super(context)
    const first = context.tracks[0]!
    this.#tracks = context.tracks
    this.#selected = new Set(context.tracks.map((t) => this.#id(t)))
    this.algorithms = sstsAlgorithms(context.client.serverSetup)
    // The draft has no selection rule (SSTS 3): its default algorithm, else our first.
    this.#algorithm = this.algorithms.includes(0n) ? 0n : (this.algorithms[0] ?? 0n)
    this.pad = this.createPad('src', [
      { direction: 'out', code: CapabilityCode.PACKAGING, value: first.packaging },
      { direction: 'out', code: CapabilityCode.ROLE, value: first.role ?? 'video' },
    ])
    const pad = this.pad
    this.#forward = new Forwarder({
      context,
      pad,
      child: options.child,
      selected: () => [...this.#children.values()].filter((c) => this.#isSelected(c)),
      childOf: (element) => [...this.#children.values()].find((c) => c.element === element),
      sinkPadOf: (child) => this.pads.find((p) => p.direction === 'sink' && p.peers[0]?.element === child.element),
      renditionChanged: (track) => this.context.player.emit('renditionchange', this, this.#rendition(track)),
    })
    pad.onEvent((event) => event.type === 'seek' && (void this.#forward.seek(event.mediaTime), true))
    this.on('padadded', (sink) => {
      if (sink.direction === 'sink') sink.onEvent((event) => this.#forward.onChildEvent(sink.peers[0]?.element, event))
    })
    this.on('padremoved', (sink) => sink.direction === 'sink' && this.#onChildRemoved(sink))
    this.emit('nomorepads')
  }

  get algorithm(): bigint {
    return this.#algorithm
  }

  /** @internal Loads a child per track, in parallel: the publisher activates once all are assigned (SSTS 6.3). */
  async start(): Promise<void> {
    await Promise.all(this.#tracks.map((t) => this.#load(t)))
  }

  /** The chain only offers pads of elements this one loaded: take them all. */
  override negotiate(): boolean {
    return true
  }

  override receive(pad: Pad, item: PadItem): Promise<void> {
    return this.#forward.receive(pad.peers[0]?.element, item)
  }

  getRenditions(): Rendition[] {
    return this.#tracks.map((t) => this.#rendition(t))
  }

  /** The relay chooses the track under SSTS. */
  async switch(): Promise<void> {
    throw new PlayerError('UNSUPPORTED_SOURCE', 'Under SSTS the relay chooses the track', { element: 'ssts' })
  }

  /** Changes the switching set (SSTS 6.2): a deselected track is unsubscribed, a selected one subscribed again. */
  async selectRenditions(renditionIds: string[]): Promise<void> {
    const before = this.#selected
    this.#selected = new Set(renditionIds)
    await Promise.all(
      [...this.#children.values()].map((child) => {
        const id = this.#id(child.track)
        if (before.has(id) === this.#selected.has(id)) return
        if (!this.#selected.has(id)) delete child.subscription
        return child.element?.selectRenditions(this.#selected.has(id) ? [id] : [])
      }),
    )
  }

  /** New tracks get a child, selected unless exactly one is selected now; removed ones are released. */
  setTracks(tracks: readonly CatalogTrack[]): void {
    const keepSelection = this.#selected.size === 1
    const next = new Set(tracks.map(trackKey))
    for (const [key, child] of this.#children) {
      if (next.has(key)) continue
      this.#children.delete(key)
      if (child.element) void this.context.release(child.element)
    }
    for (const track of tracks) {
      if (this.#children.has(trackKey(track))) continue
      if (!keepSelection) this.#selected.add(this.#id(track))
      void this.#load(track).catch(() => {})
    }
    this.#tracks = tracks
  }

  /** Pauses automatic switching: `activate` 0 on the set (SSTS 6.2). */
  pauseSwitching(): Promise<void> {
    this.#paused = true
    return this.#updateOne()
  }

  /** Resumes automatic switching: `activate` back to the set size. */
  resumeSwitching(): Promise<void> {
    this.#paused = false
    return this.#updateOne()
  }

  /**
   * Moves the whole set to another negotiated algorithm with a REQUEST_UPDATE per subscription;
   * a rejected update rolls the others back, so the set never carries two algorithms.
   * With concurrent calls the latest wins and earlier ones reject with `ABORTED`.
   */
  async setAlgorithm(id: bigint): Promise<void> {
    if (!this.algorithms.includes(id)) {
      throw new PlayerError('UNSUPPORTED_SOURCE', `SSTS algorithm ${id} wasn't negotiated`, { element: 'ssts' })
    }
    const call = ++this.#algorithmCalls
    const previous = this.#algorithmLock
    const done = Promise.withResolvers<void>()
    this.#algorithmLock = done.promise
    try {
      await previous
      if (call !== this.#algorithmCalls) throw new PlayerError('ABORTED', 'A later setAlgorithm superseded this one')
      if (id === this.#algorithm) return
      const before = this.#algorithm
      this.#algorithm = id
      const children = this.#subscribed()
      const results = await Promise.allSettled(children.map((c) => this.#update(c)))
      const failed = results.find((r): r is PromiseRejectedResult => r.status === 'rejected')
      if (!failed) return
      this.#algorithm = before
      await Promise.allSettled(
        children.filter((_, i) => results[i]!.status === 'fulfilled').map((c) => this.#update(c)),
      )
      throw failed.reason
    } finally {
      done.resolve()
    }
  }

  async #load(track: CatalogTrack): Promise<void> {
    const child: Child = { track, properties: {}, eos: false }
    this.#children.set(trackKey(track), child)
    const { switchFrom, ranges } = this.options
    let element: BaseElement
    try {
      element = await this.context.load(this.options.child, {
        tracks: [track],
        options: {
          ...(switchFrom !== undefined && { switchFrom }),
          ...(ranges && { ranges }),
          requests: this.#hooks(child),
        },
      })
    } catch (error) {
      // A child that never loaded can't count toward `activate`, or the relay would wait for it.
      this.#children.delete(trackKey(track))
      throw error
    }
    child.element = element as unknown as BaseElement & MediaSourceElement
    // shortcut: an unselected new track subscribes once before it's taken out; a load option could skip that.
    if (!this.#selected.has(this.#id(track))) await child.element.selectRenditions([])
  }

  /** Every SUBSCRIBE of the set carries its assignment, and no FORWARD: the publisher owns it (SSTS 6.3). */
  #hooks(child: Child): RequestHooks {
    return {
      subscribe: (track, { forward: _, ...options }) => ({
        ...options,
        parameters: [...(options.parameters ?? []), this.#assignment(track)],
      }),
      subscribed: (_, result, options) => {
        const replaced = child.subscription !== undefined
        child.subscription = { result, options }
        // The publisher dropped the old one from its count (SSTS 6.3): re-send the set size.
        if (replaced) void this.#update(child).catch(() => {})
      },
    }
  }

  #assignment(track: CatalogTrack): SwitchingSetAssignment {
    const activate = this.#paused ? 0n : BigInt([...this.#children.values()].filter((c) => this.#isSelected(c)).length)
    const kbps = BigInt(Math.round(bitrateOf(track) / 1000))
    return new SwitchingSetAssignment(this.#setIdFor(this.context.client), this.#algorithm, kbps, 1n, activate, 0)
  }

  /** Unique per client, and taken again on the client after a reconnect. */
  #setIdFor(client: MOQtailClient): bigint {
    if (this.#setId?.client !== client) {
      const id = nextSetId.get(client) ?? 0n
      nextSetId.set(client, id + 1n)
      this.#setId = { client, id }
    }
    return this.#setId.id
  }

  /** A REQUEST_UPDATE with only the assignment (no priority, no forward). */
  #update(child: Child): Promise<void> {
    const subscription = child.subscription
    if (!subscription) return Promise.resolve()
    return this.context.client.subscribeUpdate({
      subscriptionRequestId: subscription.result.requestId,
      parameters: [this.#assignment(child.track)],
    })
  }

  /** `activate` changes for the whole set through one of its subscriptions. */
  async #updateOne(): Promise<void> {
    const [child] = this.#subscribed()
    if (child) await this.#update(child)
  }

  #subscribed(): Child[] {
    return [...this.#children.values()].filter((c) => c.subscription && this.#isSelected(c))
  }

  #isSelected(child: Child): boolean {
    return this.#selected.has(this.#id(child.track))
  }

  /** A child that removes its pad is dropped; with none left, so is the source pad. */
  #onChildRemoved(sink: Pad): void {
    const from = sink.peers[0]?.element
    if (this.#forward.seekChildRemoved(from)) return
    for (const [key, child] of this.#children) if (child.element === from) this.#children.delete(key)
    if (this.#children.size === 0 && this.pads.includes(this.pad)) this.removePad(this.pad)
    else this.#forward.endIfDone()
  }

  #rendition(track: CatalogTrack): Rendition {
    return {
      id: this.#id(track),
      src: formatMsfUrl(this.context.catalog?.url ?? '', track.namespace, track.nameBytes),
      bitrate: bitrateOf(track),
      codec: track.codec ?? '',
      ...(track.width !== undefined && { width: track.width }),
      ...(track.height !== undefined && { height: track.height }),
      ...(track.framerate !== undefined && { frameRate: track.framerate }),
      active: this.#forward.arriving?.track === track,
      selected: this.#selected.has(this.#id(track)),
    }
  }

  #id(track: CatalogTrack): string {
    return renditionId(track, this.context.catalog)
  }
}

/**
 * The `switchingSetSource` of `SSTS`. Never probed; the catalog element loads it for a set with
 * `{ child }`, the definition the probe picked. Pads carry their own capabilities, so it lists none.
 */
export const Ssts = defineElement({
  type: 'source',
  name: 'ssts',
  capabilities: [],
  provides: [MEDIA_SOURCE],
  probe: () => 0,
  init: async (context, options?: SwitchingSetOptions) => {
    if (!options?.child.provides?.includes(MEDIA_SOURCE)) {
      throw new PlayerError('UNSUPPORTED_SOURCE', 'An SSTS child must be a media source', { element: 'ssts' })
    }
    if (context.tracks.length === 0) {
      throw new PlayerError('UNSUPPORTED_SOURCE', 'The SSTS source needs at least one track', { element: 'ssts' })
    }
    const source = new SstsSource(context, options)
    await source.start()
    return source
  },
})

if (import.meta.vitest) {
  const { test, expect, vi, afterEach } = import.meta.vitest
  const { Setup } = await import('../../../model/control/setup')
  const { SetupOptions } = await import('../../../model/parameter/setup_option')
  const { PublishDoneStatusCode } = await import('../../../model/control/constant')
  const { Cmaf } = await import('../../elements/cmaf')
  const { cmafTrack, fakeCatalog, requested, twoSecondTimeline } = await import('../../testing/cmaf')
  const { FakeClient, mediaObject } = await import('../../testing/client')
  const { collectingSink, flush, packet, testChain, testSource } = await import('../../testing/elements')
  type Sink = import('../../testing/elements').CollectingSink
  type UpdateOptions = import('../../../client/types').SubscribeUpdateOptions
  afterEach(() => void vi.useRealTimers())

  /** SSTS sources over `child` (CMAF by default), one per set, on one `FakeClient`. */
  async function sstsChain(
    sets: CatalogTrack[][],
    options: { algorithms?: bigint[]; child?: typeof Cmaf; catalog?: ReturnType<typeof fakeCatalog> } = {},
  ) {
    const client = new FakeClient()
    client.serverSetup = new Setup(new SetupOptions().addSstsAlgorithms(options.algorithms ?? [0n, 0xff01n]).build())
    const { chain, failures } = testChain(
      () => [],
      () => client,
    )
    const sources: SstsSource[] = []
    let sink!: Sink
    chain.addSource(
      testSource({
        script: async (parent) => {
          for (const tracks of sets) {
            const request = { tracks, options: { child: options.child ?? Cmaf } }
            sources.push(
              await parent.ctx.load(Ssts, { ...request, ...(options.catalog && { catalog: options.catalog }) }),
            )
          }
          parent.emit('nomorepads')
        },
      }),
      {},
    )
    chain.addSink(collectingSink({ setup: (k) => ((sink = k), (k.accept = () => true)) }))
    await chain.init(new AbortController().signal)
    const renditions: string[] = []
    chain.host.player.on('renditionchange', (_, rendition) => renditions.push(rendition.id))
    await flush()
    return {
      client,
      sources,
      source: sources[0]!,
      sink,
      failures,
      renditions,
      /** The request a track's latest SUBSCRIBE or FETCH opened. */
      /** A track's latest subscription. */
      request: (name: string) =>
        client.requests.filter((r) => r.kind === 'subscribe' && requested(r.options) === name).at(-1)!,
      trace: () =>
        sink.items().map((item) => {
          if (item.type === 'packet') return `${item.object.location?.group}.${item.object.location?.object}`
          if (item.type === 'properties') return `props ${item.properties.track ?? ''}`.trim()
          return item.event.type === 'discontinuity' ? item.event.reason : item.event.type
        }),
    }
  }

  /** The assignment a SUBSCRIBE or REQUEST_UPDATE carries, in short. */
  function assigned(options: { parameters?: unknown[] }): string {
    const a = options.parameters?.find((p): p is SwitchingSetAssignment => p instanceof SwitchingSetAssignment)
    return a
      ? `set ${a.switchingSetId} alg ${a.algorithmId} kbps ${a.throughputThresholdKbps} activate ${a.activateSwitching}`
      : 'none'
  }
  const v = (name: string, bitrate = 1_000_000) => cmafTrack(name, { bitrate })

  test('every track subscribes into its set: negotiated algorithm, activate = set size, no FORWARD, IDs per client', async () => {
    const audio = cmafTrack('a1', { role: 'audio', bitrate: 128_000 })
    const c = await sstsChain([[v('v1', 800_000), v('v2', 2_000_000)], [audio]], { algorithms: [7n, 0xff01n] })
    const subscribes = c.client.sent<SubscribeOptions>('subscribe')
    expect(subscribes.map((s) => [requested(s), s.forward, assigned(s)])).toEqual([
      ['v1', undefined, 'set 0 alg 65281 kbps 800 activate 2'],
      ['v2', undefined, 'set 0 alg 65281 kbps 2000 activate 2'],
      ['a1', undefined, 'set 1 alg 65281 kbps 128 activate 1'],
    ])
    expect(c.sources.map((s) => [s.algorithms, s.algorithm])).toEqual([
      [[0xff01n], 0xff01n],
      [[0xff01n], 0xff01n],
    ])
    expect(c.sources[1]!.pad.capabilities.map((cap) => cap.value)).toEqual(['cmaf', 'audio'])
    // 0, the draft's default, when the peer accepts it.
    expect((await sstsChain([[v('v1')]])).source.algorithm).toBe(0n)
  })

  test('the arriving track writes its properties first and is the active one; one eos after every child', async () => {
    const c = await sstsChain([[v('v1'), v('v2')]])
    c.request('v1').push(mediaObject(1, 0))
    await flush()
    c.request('v2').push(mediaObject(2, 0, null, 'v2'))
    await flush()
    c.request('v1').end(PublishDoneStatusCode.TrackEnded)
    await flush()
    expect(c.trace()).toEqual(['props v1', '1.0', 'props v2', '2.0'])
    c.request('v2').end(PublishDoneStatusCode.TrackEnded)
    await flush()
    expect(c.trace().slice(-1)).toEqual(['eos'])
    expect(c.renditions).toEqual(['v2'])
    expect(c.source.getRenditions().map((r) => [r.id, r.active])).toEqual([
      ['v1', false],
      ['v2', true],
    ])
  })

  test('switch rejects; selectRenditions unsubscribes and re-adds; a retry re-sends activate', async () => {
    const c = await sstsChain([[v('v1'), v('v2')]])
    vi.useFakeTimers()
    await expect(c.source.switch()).rejects.toMatchObject({ code: 'UNSUPPORTED_SOURCE' })
    await c.source.selectRenditions(['v1'])
    await vi.advanceTimersByTimeAsync(0)
    expect(c.client.sent('unsubscribe')).toEqual([1n])
    c.request('v1').reset()
    await vi.advanceTimersByTimeAsync(6000)
    // The retry carries the current set size, then the count is sent again for the replacement.
    const retry = c.client.sent<SubscribeOptions>('subscribe').at(-1)!
    expect([requested(retry), assigned(retry)]).toEqual(['v1', 'set 0 alg 0 kbps 1000 activate 1'])
    const [update] = c.client.sent<UpdateOptions>('subscribeUpdate')
    expect(update).toEqual({ subscriptionRequestId: 2n, parameters: [expect.any(SwitchingSetAssignment)] })
    expect(assigned(update!)).toBe('set 0 alg 0 kbps 1000 activate 1')
    await c.source.selectRenditions(['v1', 'v2'])
    await vi.advanceTimersByTimeAsync(0)
    const added = c.client.sent<SubscribeOptions>('subscribe').at(-1)!
    expect([requested(added), assigned(added)]).toEqual(['v2', 'set 0 alg 0 kbps 1000 activate 2'])
    expect(c.failures).toEqual([])
  })

  test('pausing and resuming switching sends only the assignment', async () => {
    const c = await sstsChain([[v('v1'), v('v2')]])
    await c.source.pauseSwitching()
    await c.source.resumeSwitching()
    const updates = c.client.sent<UpdateOptions>('subscribeUpdate')
    expect(updates.map((u) => [Object.keys(u).sort(), assigned(u)])).toEqual([
      [['parameters', 'subscriptionRequestId'], 'set 0 alg 0 kbps 1000 activate 0'],
      [['parameters', 'subscriptionRequestId'], 'set 0 alg 0 kbps 1000 activate 2'],
    ])
  })

  test('setAlgorithm moves the whole set, later subscriptions follow, and a rejected update rolls back', async () => {
    const c = await sstsChain([[v('v1'), v('v2')]])
    await expect(c.source.setAlgorithm(7n)).rejects.toMatchObject({ code: 'UNSUPPORTED_SOURCE' })
    await c.source.setAlgorithm(0xff01n)
    const updates = () =>
      c.client.sent<UpdateOptions>('subscribeUpdate').map((u) => `${u.subscriptionRequestId} ${assigned(u)}`)
    expect(updates()).toEqual(['0 set 0 alg 65281 kbps 1000 activate 2', '1 set 0 alg 65281 kbps 1000 activate 2'])
    await c.source.selectRenditions(['v1'])
    await c.source.selectRenditions(['v1', 'v2'])
    await flush()
    expect(assigned(c.client.sent<SubscribeOptions>('subscribe').at(-1)!)).toContain('alg 65281')

    c.client.calls.length = 0
    c.client.update = (args) => {
      if (args.subscriptionRequestId === 2n) throw new Error('rejected')
    }
    await expect(c.source.setAlgorithm(0n)).rejects.toThrow('rejected')
    expect(c.source.algorithm).toBe(0xff01n)
    expect(updates()).toEqual([
      '0 set 0 alg 0 kbps 1000 activate 2',
      '2 set 0 alg 0 kbps 1000 activate 2',
      '0 set 0 alg 65281 kbps 1000 activate 2',
    ])

    c.client.update = () => {}
    const first = c.source.setAlgorithm(0n)
    const latest = c.source.setAlgorithm(0xff01n)
    await expect(first).rejects.toMatchObject({ code: 'ABORTED' })
    await latest
    expect(c.source.algorithm).toBe(0xff01n)
  })

  test('a seek is answered once: routed at the live edge, a separate child behind it', async () => {
    const c = await sstsChain([[v('v1'), v('v2')]], { catalog: fakeCatalog([], twoSecondTimeline()) })
    c.request('v1').push(mediaObject(5), mediaObject(6))
    await flush()
    c.sink.pads[0]!.sendEvent({ type: 'seek', mediaTime: 60 }) // group 31, past the edge
    await flush()
    c.request('v1').push(mediaObject(40))
    await flush()
    expect(c.trace().slice(-2)).toEqual(['seek', '40.0'])

    c.sink.pads[0]!.sendEvent({ type: 'seek', mediaTime: 5 }) // group 3, behind it
    await flush()
    const history = c.client.requests.at(-1)!
    expect(history.kind).toBe('fetch')
    expect(assigned(history.options)).toBe('none') // a fetch is outside the set
    c.request('v1').push(mediaObject(41)) // the set waits behind the history
    history.push(mediaObject(3), mediaObject(40))
    history.end()
    await flush()
    c.request('v1').push(mediaObject(42))
    await flush()
    expect(c.trace().slice(5)).toEqual(['props v1', 'seek', '3.0', '40.0', 'props v1', 'gap', '42.0'])
  })

  test('a seek child that fails to load answers its seek as failed', async () => {
    const child = testSource({
      media: true,
      script: (s, options) => {
        if (options?.['ranges']) throw new PlayerError('UNSUPPORTED_SOURCE', 'no history')
        const pad = s.addPad()
        s.emit('nomorepads')
        void pad.write({ type: 'packet', object: mediaObject(5) })
      },
    })
    const c = await sstsChain([[v('v1')]], {
      child: child as unknown as typeof Cmaf,
      catalog: fakeCatalog([], twoSecondTimeline()),
    })
    c.sink.pads[0]!.sendEvent({ type: 'seek', mediaTime: 5 }) // group 3, behind group 5
    await flush()
    expect(c.sink.items().at(-1)).toEqual({
      type: 'event',
      event: { type: 'discontinuity', reason: 'seek', failed: true },
    })
  })

  test('setTracks loads new tracks into the set and releases removed ones', async () => {
    const c = await sstsChain([[v('v1'), v('v2')]])
    c.source.setTracks([v('v1'), v('v3')])
    await flush()
    expect(c.client.sent('unsubscribe')).toEqual([1n])
    const added = c.client.sent<SubscribeOptions>('subscribe').at(-1)!
    expect([requested(added), assigned(added)]).toEqual(['v3', 'set 0 alg 0 kbps 1000 activate 2'])
  })

  test('any media source can be the child, with the set’s packaging; others are rejected', async () => {
    const loc = testSource({
      media: true,
      script: (s) => {
        const pad = s.addPad([{ direction: 'out', code: CapabilityCode.PACKAGING, value: 'loc' }])
        s.emit('nomorepads')
        void pad.write(packet(1))
      },
    })
    const c = await sstsChain([[cmafTrack('l1', { packaging: 'loc' })]], { child: loc as unknown as typeof Cmaf })
    expect(c.source.pad.capabilities.map((cap) => cap.value)).toEqual(['loc', 'video'])
    expect(c.sink.items().filter((i) => i.type === 'packet')).toHaveLength(1)
    await expect(sstsChain([[v('v1')]], { child: testSource() as unknown as typeof Cmaf })).rejects.toMatchObject({
      code: 'UNSUPPORTED_SOURCE',
    })
  })
}
