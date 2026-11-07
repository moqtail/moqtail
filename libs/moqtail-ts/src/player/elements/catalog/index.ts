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

import { defineElement } from '@/player/element/define'
import { PlayerError } from '@/player/error'
import { CatalogElement, type CatalogOptions } from './element'

/**
 * The built-in source for an MSF catalog track. The player registers it, and it's reached
 * through the probe, so it isn't exported.
 */
export const CatalogSource = defineElement({
  type: 'source',
  name: 'catalog',
  capabilities: [],
  probe: (ids) => (ids.track === 'catalog' || ids.packaging === 'catalog' ? 1 : 0),
  init: async (context, options?: CatalogOptions) => {
    const source = options?.source
    if (!source)
      throw new PlayerError('UNSUPPORTED_SOURCE', 'The catalog source needs the catalog URL', { element: 'catalog' })
    const catalog = new CatalogElement(context, { ...options, source })
    await catalog.start()
    return catalog
  },
})

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest
  const { FakeClient, gzip, refuse, testObject } = await import('@/player/testing/client')
  const { testChain, TestSource, flush, settle } = await import('@/player/testing/elements')
  const interfaces = await import('@/player/element/interfaces')
  // Annotated, since copying a unique symbol into a new const widens it to symbol
  const MEDIA_SOURCE: typeof interfaces.MEDIA_SOURCE = interfaces.MEDIA_SOURCE
  const { defineExtension } = await import('@/player/extension')
  const { parseMsfUrl } = await import('@/player/msf_url')
  const { Location } = await import('@/model/common/location')
  const { MsfCompressionObjectProperty } = await import('@/player/util/compression')
  const { requested } = await import('@/player/testing/cmaf')
  type Track = import('@/player/element/interfaces').CatalogTrack
  type Options = import('@/player/player').MOQtailPlayerOptions
  type Definition = import('@/player/element/define').SourceElementDefinition
  type Respond = import('@/player/testing/client').FakeClient['respond']
  type MoqtObject = import('@/model/data/object').MoqtObject
  type MediaSourceOptions = import('@/player/element/interfaces').MediaSourceOptions

  const bytes = (json: unknown) => new TextEncoder().encode(typeof json === 'string' ? json : JSON.stringify(json))
  const track = (name: string, role: string, extra: Record<string, unknown> = {}) => ({
    name,
    packaging: 'cmaf',
    role,
    isLive: true,
    ...(role === 'video' && { altGroup: 1 }),
    ...extra,
  })
  const catalog = (tracks: unknown[], extra: Record<string, unknown> = {}) => ({
    version: 'draft-01',
    tracks,
    ...extra,
  })
  const delta = (op: string, ...tracks: unknown[]) => ({ generatedAt: 1, deltaUpdate: [{ op, tracks }] })
  const object = (group: number, id: number, payload: Uint8Array, compressed = false) =>
    testObject(
      'catalog',
      group,
      id,
      payload,
      compressed ? [new MsfCompressionObjectProperty(1).toKeyValuePair()] : null,
    )
  const names = (tracks: readonly Track[]) => tracks.map((t) => t.name).join(',')

  /** A catalog source over a fake client, with a media source that logs loads and `setTracks`. */
  async function setup(
    options: Options | ((stub: (name: string) => Definition) => Options) = {},
    respond?: Respond,
    params = '',
  ) {
    const client = new FakeClient()
    if (respond) client.respond = respond
    const log: string[] = []
    const warnings: string[] = []
    /** Loaded as `name`, it logs its load and every `setTracks`. */
    const stub = (name: string) =>
      defineElement({
        type: 'source',
        name,
        capabilities: [],
        provides: [MEDIA_SOURCE],
        probe: (t) => (t.packaging === 'cmaf' && (t.role === 'video' || t.role === 'audio') ? 1 : 0),
        init: async (context, loadOptions?: Record<string, unknown>) => {
          const first = context.tracks[0]!.name
          const child = loadOptions?.['child'] ? ` around ${(loadOptions['child'] as Definition).name}` : ''
          const ranges = (loadOptions as MediaSourceOptions | undefined)?.ranges
          const where = ranges
            ? ` ${[...ranges.locationRanges.map((r) => `L${r.start.group}-${r.end?.group ?? ''}`), ...ranges.mediaTimeRanges.map((r) => `T${r.start}-${r.end ?? ''}`)]}`
            : ''
          log.push(`${name === 'media' ? 'load' : name} ${names(context.tracks)}${child}${where}`)
          return Object.assign(new TestSource(context), {
            getRenditions: () => [],
            switch: async () => {},
            selectRenditions: async () => {},
            setTracks: (tracks: readonly Track[]) => void log.push(`${first} ← ${names(tracks)}`),
          })
        },
      })
    const media = stub('media')
    const { chain } = testChain(
      () => [media, CatalogSource],
      () => client,
    )
    Object.assign(chain.host.player.options, typeof options === 'function' ? options(stub) : options)
    let element: CatalogElement | undefined
    chain.host.player.on('catalog', (c) => (element = c as CatalogElement))
    chain.host.player.on('warning', (w) => warnings.push(w.code))
    chain.addSource(CatalogSource, { ...parseMsfUrl(`moqt://relay.test/app#msf:ns--catalog${params}`) })
    let settled: string | undefined
    const init = chain.init(new AbortController().signal).then(
      () => (settled = 'resolved'),
      (e: PlayerError) => (settled = e.code),
    )
    await flush()
    const push = async (...objects: MoqtObject[]) => {
      client.last.push(...objects)
      await settle()
    }
    return { client, media, log, warnings, init, push, settled: () => settled, catalog: () => element! }
  }

  describe('CatalogSource', () => {
    test('an empty first catalog waits for updates, then one video and one audio set load', async () => {
      const c = await setup({ eventTimelines: 'all' })
      expect(c.client.sent<{ priority: number }>('subscribe')[0]!.priority).toBe(0)
      await c.push(object(1, 0, bytes(catalog([]))))
      expect(c.settled()).toBeUndefined()
      const timeline = { name: 'sap', packaging: 'eventtimeline', eventType: 'x', isLive: true }
      const tracks = [
        track('v1', 'video'),
        track('v2', 'video'),
        { ...track('a1', 'audio'), role: undefined, samplerate: 1 },
      ]
      await c.push(object(1, 1, bytes(delta('add', ...tracks, timeline))))
      await c.init
      expect([c.settled(), c.log, c.warnings]).toEqual(['resolved', ['load v1,v2', 'load a1'], ['TRACK_SKIPPED']])
      expect([c.catalog().streamType, c.catalog().duration, c.catalog().byRole('audio').length]).toEqual([
        'live',
        Infinity,
        1,
      ])
    })

    test('isComplete without tracks loads nothing; tracks with none playable, or a bad catalog, are fatal', async () => {
      const complete = await setup()
      await complete.push(object(1, 0, bytes(catalog([], { isComplete: true }))))
      await complete.init
      expect([complete.settled(), complete.log, complete.catalog().isComplete]).toEqual(['resolved', [], true])

      const unplayable = await setup()
      await unplayable.push(object(1, 0, bytes(catalog([{ ...track('x', 'video'), packaging: 'other' }]))))
      await unplayable.init
      expect(unplayable.settled()).toBe('UNSUPPORTED_SOURCE')

      const bad = await setup()
      await bad.push(object(1, 0, bytes('{')))
      await bad.init
      expect(bad.settled()).toBe('UNSUPPORTED_SOURCE')
    })

    test('rejected subscribes are retried, and no catalog in time is CATALOG_TIMEOUT', async () => {
      const refused = await setup({ catalogTimeoutMs: 400 }, () => refuse())
      await refused.init
      expect(refused.settled()).toBe('CATALOG_TIMEOUT')
      expect(refused.client.sent('subscribe').length).toBeGreaterThan(1)

      const silent = await setup({ catalogTimeoutMs: 20 })
      await silent.init
      expect([silent.settled(), silent.client.sent('unsubscribe')]).toEqual(['CATALOG_TIMEOUT', [0n]])
    })

    test('fill, compression, deltas out of order, and a stale group end at the expected catalog', async () => {
      const c = await setup()
      const v = (name: string, extra = {}) => track(name, 'video', extra)
      await c.push(
        object(5, 0, await gzip(bytes(catalog([v('v1')]))), true),
        object(5, 2, bytes(delta('add', v('v3')))),
        object(4, 1, bytes(delta('add', v('old')))),
        object(5, 1, bytes(delta('add', v('v2')))),
      )
      await c.init
      expect([c.log, names(c.catalog().tracks)]).toEqual([['load v1', 'v1 ← v1,v2', 'v1 ← v1,v2,v3'], 'v1,v2,v3'])
      // An unknown track: rejected, the catalog goes stale and later deltas of the group are ignored
      await c.push(object(5, 3, bytes(delta('remove', { name: 'nope' }))), object(5, 4, bytes(delta('add', v('v9')))))
      expect([c.warnings, c.catalog().stale, names(c.catalog().tracks)]).toEqual([
        ['INVALID_CATALOG_UPDATE'],
        true,
        'v1,v2,v3',
      ])
      await c.push(
        object(6, 0, bytes(catalog([v('v1'), v('v2')]))),
        object(6, 1, bytes(delta('update', { parentName: 'v2', bitrate: 5 }))),
      )
      expect([c.catalog().stale, names(c.catalog().tracks), c.catalog().find('v2')?.bitrate]).toEqual([
        false,
        'v1,v2',
        5,
      ])
      expect(c.log.slice(3)).toEqual(['v1 ← v1,v2', 'v1 ← v1,v2'])
    })

    test('updates: in place, remove and add, added renditions, a new audio source, VOD, and isComplete', async () => {
      const c = await setup()
      await c.push(object(1, 0, bytes(catalog([track('v1', 'video'), track('v2', 'video'), track('a1', 'audio')]))))
      await c.init
      await c.push(
        object(1, 1, bytes(delta('update', { parentName: 'v1', bitrate: 2 }))), // in place
        object(1, 2, bytes(delta('update', { parentName: 'v2', codec: 'hvc1' }))), // removed and added
        object(1, 3, bytes(delta('add', track('v3', 'video'), track('v4', 'video', { altGroup: 2 })))),
        object(1, 4, bytes(delta('remove', { name: 'a1' }))),
        object(1, 5, bytes(delta('add', track('a2', 'audio')))), // fits no set, but audio has none left
      )
      const vod = { isLive: false, trackDuration: 4000 }
      const tracks = [{}, { codec: 'hvc1' }, {}, { altGroup: 2 }].map((extra, i) =>
        track(`v${i + 1}`, 'video', { ...vod, ...extra }),
      )
      await c.push(object(2, 0, bytes(catalog([...tracks, track('a2', 'audio', vod)]))))
      expect([c.catalog().streamType, c.catalog().duration]).toEqual(['on-demand', 4])
      await c.push(object(3, 0, bytes(catalog([], { isComplete: true }))))
      expect(c.log).toEqual([
        ...['load v1,v2', 'load a1', 'v1 ← v1,v2', 'v1 ← v1,v2', 'v1 ← v1,v2,v3', 'a1 ← '],
        ...['load a2', 'v1 ← v1,v2,v3', 'a2 ← a2'],
      ])
      expect([c.catalog().isComplete, c.catalog().tracks.length, c.catalog().streamType]).toEqual([
        true,
        0,
        'on-demand',
      ])
    })

    test('an isComplete catalog keeps a track it changes beyond in place, once', async () => {
      const c = await setup()
      await c.push(object(1, 0, bytes(catalog([track('v1', 'video'), track('v2', 'video')]))))
      await c.init
      const tracks = [track('v1', 'video'), track('v2', 'video', { codec: 'hvc1' })]
      await c.push(object(2, 0, bytes(catalog(tracks, { isComplete: true }))))
      expect(c.log).toEqual(['load v1,v2', 'v1 ← v1,v2'])
    })

    test('timeline(): the template first, else one shared media timeline that follows updates', async () => {
      const c = await setup()
      const history = { name: 'history', packaging: 'mediatimeline', isLive: true, depends: [1] }
      const template = [0, 2000, [1, 0], [1, 0], 0, 0]
      const tracks = [track('v1', 'video', { renderGroup: 1 }), track('a1', 'audio', { renderGroup: 1, template })]
      await c.push(object(1, 0, bytes(catalog([...tracks, history]))))
      await c.init
      const [v1, a1] = c.catalog().tracks as [Track, Track]
      expect((await c.catalog().timeline(a1))?.timeToLocation(4000)).toEqual(new Location(3n, 0n))
      const first = c.catalog().timeline(v1)
      const second = c.catalog().timeline(v1)
      await flush()
      const entries = (json: unknown[], id = 0) => testObject('history', 1, id, bytes(json))
      c.client.last.push(entries([[0, [1, 0], 0]]))
      const [reader] = await Promise.all([first, second])
      expect(c.client.sent('subscribe')).toHaveLength(2) // the catalog, and one media timeline
      await c.push(entries([[2000, [2, 0], 0]], 1))
      expect([reader?.timeToLocation(2500), reader?.range]).toEqual([new Location(2n, 0n), { start: 0, end: 2000 }])
    })

    test('location ranges: as written for the video set, as media time for audio when both have a timeline', async () => {
      const params = '&location-range=2-4&mediatime-range=50000'
      const c = await setup({}, undefined, params)
      const template = (ms: number) => ({ template: [0, ms, [0, 0], [1, 0], 0, 0] })
      await c.push(
        object(1, 0, bytes(catalog([track('v1', 'video', template(2000)), track('a1', 'audio', template(700))]))),
      )
      await c.init
      // Video groups 2-4 are 4 s up to 10 s
      expect(c.log).toEqual(['load v1 L2-4,T50000-', 'load a1 T50000-,T4000-9999'])

      const plain = await setup({}, undefined, params)
      await plain.push(object(1, 0, bytes(catalog([track('v1', 'video'), track('a1', 'audio')]))))
      expect(plain.log).toEqual(['load v1 L2-4,T50000-', 'load a1 L2-4,T50000-'])
    })

    test('timeline(): a media timeline that fails is a warning once, then undefined without a new subscribe', async () => {
      const c = await setup({}, (_, options) => (requested(options) === 'history' ? refuse() : {}))
      const history = { name: 'history', packaging: 'mediatimeline', isLive: true, depends: ['v1'] }
      await c.push(object(1, 0, bytes(catalog([track('v1', 'video'), history]))))
      await c.init
      const v1 = c.catalog().tracks[0]!
      expect(await c.catalog().timeline(v1)).toBeUndefined()
      expect(await c.catalog().timeline(v1)).toBeUndefined()
      expect([c.client.sent('subscribe').length, c.warnings]).toEqual([2, ['TRACK_SKIPPED']])
    })

    test('a switching set source wraps media sets only when its extension is listed and negotiated', async () => {
      for (const accepted of [true, false]) {
        const c = await setup((stub) => ({
          extensions: [defineExtension({ name: 'sets', switchingSetSource: stub('wrap'), negotiated: () => accepted })],
        }))
        await c.push(object(1, 0, bytes(catalog([track('v1', 'video')]))))
        await c.init
        expect([c.settled(), c.log]).toEqual(['resolved', [accepted ? 'wrap v1 around media' : 'load v1']])
      }
    })
  })
}
