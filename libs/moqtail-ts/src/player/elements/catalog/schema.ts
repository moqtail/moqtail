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
 * Valibot schemas of the MSF catalog (MSF 5.1, 5.2, 5.3) with the CMSF track fields
 * (CMSF 3.5.2). Unknown fields are kept. Encryption and content protection fields
 * (MSF 5.2.39 to 5.2.42, CMSF 5.1) stay untyped extra fields.
 */

import * as v from 'valibot'
import type { Tuple } from '@/model/common/tuple'
import { splitNamespace, toBytes } from './strings'

/** The `version` values this player parses, with the editor's draft rules. */
export const SUPPORTED_MSF_VERSIONS = ['draft-01'] as const

const int = v.pipe(v.number(), v.integer())
const location = v.tuple([int, int])

/** MSF 5.2.5: `key=value;key=value`, the first occurrence of a key wins. */
function keyValues(s: string): Record<string, string> {
  const record: Record<string, string> = {}
  for (const pair of s.split(';')) {
    const at = pair.indexOf('=')
    const key = pair.slice(0, at)
    if (at > 0 && !Object.hasOwn(record, key)) record[key] = pair.slice(at + 1)
  }
  return record
}

// Keys each packaging defines for packagingMetadata (MSF 5.2.5); others are ignored
const PACKAGING_METADATA_KEYS: Record<string, readonly string[]> = { loc: ['streamMapping'] }

const definedKeys = (packaging: string, record: Record<string, string>) =>
  Object.fromEntries(Object.entries(record).filter(([key]) => PACKAGING_METADATA_KEYS[packaging]?.includes(key)))

/** The media timeline template of MSF 7.4.1, with named fields. Times in milliseconds. */
const TemplateSchema = v.pipe(
  v.tuple([v.number(), v.number(), location, location, v.number(), v.number()]),
  v.transform(([startMediaTime, deltaMediaTime, startLocation, deltaLocation, startWallclock, deltaWallclock]) => ({
    startMediaTime,
    deltaMediaTime,
    startLocation,
    deltaLocation,
    startWallclock,
    deltaWallclock,
  })),
)

// MSF 5.2.4.1: fields required beyond namespace, name, packaging, and isLive
const REQUIRED_FIELDS: Record<string, readonly string[]> = {
  'loc/audio': ['codec', 'samplerate', 'channelConfig'],
  'loc/video': ['codec', 'framerate'],
  'loc/audiodescription': ['codec', 'samplerate', 'channelConfig', 'lang'],
  'loc/signlanguage': ['codec', 'framerate'],
  'eventtimeline/eventtimeline': ['eventType'],
}

const missingFields = (track: Record<string, unknown>) =>
  (REQUIRED_FIELDS[`${track['packaging']}/${track['role']}`] ?? []).filter((field) => track[field] === undefined)

/** The fields of a track object (MSF 5.2, CMSF 3.5.2), unknown ones kept. */
const TrackFields = v.looseObject({
  namespace: v.optional(v.pipe(v.string(), v.transform(splitNamespace))),
  name: v.string(),
  packaging: v.string(),
  packagingMetadata: v.optional(v.pipe(v.string(), v.transform(keyValues))),
  eventType: v.optional(v.string()),
  role: v.optional(v.string()),
  isLive: v.boolean(),
  targetLatency: v.optional(v.number()),
  buffers: v.optional(
    v.looseObject({ target: v.optional(int), min: v.optional(v.number()), max: v.optional(v.number()) }),
  ),
  label: v.optional(v.string()),
  renderGroup: v.optional(int),
  altGroup: v.optional(int),
  initRef: v.optional(v.string()),
  depends: v.optional(v.array(v.union([v.string(), int]))),
  template: v.optional(TemplateSchema),
  temporalId: v.optional(int),
  spatialId: v.optional(int),
  codec: v.optional(v.string()),
  mimeType: v.optional(v.string()),
  framerate: v.optional(v.number()),
  timescale: v.optional(v.number()),
  bitrate: v.optional(v.number()),
  avgBitrate: v.optional(v.number()),
  maxGopDuration: v.optional(v.number()),
  maxGroupDuration: v.optional(v.number()),
  width: v.optional(v.number()),
  height: v.optional(v.number()),
  samplerate: v.optional(v.number()),
  channelConfig: v.optional(v.string()),
  displayWidth: v.optional(v.number()),
  displayHeight: v.optional(v.number()),
  lang: v.optional(v.string()),
  parentName: v.optional(v.string()),
  parentNamespace: v.optional(v.string()),
  trackDuration: v.optional(int),
  connectionUri: v.optional(v.string()),
  token: v.optional(v.string()),
  // TODO(authorization): present tokens for these schemes when subscribing (MSF 11.5.3)
  authInfo: v.optional(v.record(v.string(), v.unknown())),
  accessibility: v.optional(v.array(v.looseObject({ scheme: v.string(), value: v.string() }))),
  accessibleWindowTime: v.optional(v.number()),
  keyframeSpacing: v.optional(v.pipe(int, v.minValue(1))),
  maxGrpSapStartingType: v.optional(int),
  maxObjSapStartingType: v.optional(int),
})

type TrackFields = v.InferOutput<typeof TrackFields>

/** The track fields MSF 5.2 and CMSF 3.5.2 define; any other field is an extra field. */
export const TRACK_FIELD_NAMES: readonly string[] = Object.keys(TrackFields.entries)

/**
 * One track object (MSF 5.2). A track without a namespace inherits `catalogNamespace`.
 * `toBytes` and `splitNamespace` throw a `PlayerError` for a bad name or namespace (MSF
 * 11.2, 5.2.2), which Valibot passes through.
 */
const trackSchema = (catalogNamespace: Tuple) =>
  v.pipe(
    TrackFields,
    v.check(
      (track) => missingFields(track).length === 0,
      (issue) => `missing ${missingFields(issue.input as Record<string, unknown>).join(', ')} (MSF 5.2.4.1)`,
    ),
    v.check(
      (track) => (track.packaging === 'eventtimeline') === (track.eventType !== undefined),
      'eventType goes with, and only with, eventtimeline packaging (MSF 5.2.6)',
    ),
    v.check(
      (track) => track.targetLatency === undefined || track.buffers === undefined,
      'targetLatency and buffers are mutually exclusive (MSF 5.2.9)',
    ),
    v.check(
      (track) => !track.isLive || track.trackDuration === undefined,
      'a live track has no trackDuration (MSF 5.2.36)',
    ),
    // Annotated, so the output keeps the index signature of unknown fields
    v.transform((track): TrackFields & { nameBytes: Uint8Array; namespace: Tuple; init?: InitData } => ({
      ...track,
      nameBytes: toBytes(track.name),
      namespace: track.namespace ?? catalogNamespace,
      ...(track.packagingMetadata && { packagingMetadata: definedKeys(track.packaging, track.packagingMetadata) }),
    })),
  )

/** @public Where a track's init data comes from (MSF 5.1.7, 12.2). */
export type InitData =
  { type: 'inline'; data: Uint8Array } | { type: 'track-property' | 'object-property'; property: number }

const Base64Schema = v.pipe(v.string(), v.base64())
const HEX = /^(0x)?[0-9a-f]+$/i

/** An `initDataList` entry (MSF 5.1.7), resolved to the bytes or the property type. */
const InitDataSchema = v.pipe(
  v.object({ id: v.string(), type: v.picklist(['inline', 'track-property', 'object-property']), data: v.string() }),
  v.check(
    ({ type, data }) => (type === 'inline' ? v.is(Base64Schema, data) : HEX.test(data)),
    'initDataList data must be base64, or a property type in hex (MSF 5.1.7)',
  ),
  v.transform(({ id, type, data }): { id: string; init: InitData } => ({
    id,
    init:
      type === 'inline'
        ? { type, data: Uint8Array.from(atob(data), (c) => c.charCodeAt(0)) }
        : { type, property: parseInt(data, 16) },
  })),
)

/** An independent catalog (MSF 5.1) whose tracks without a namespace inherit `catalogNamespace`. */
export const catalogSchema = (catalogNamespace: Tuple) =>
  v.pipe(
    v.looseObject({
      version: v.picklist(SUPPORTED_MSF_VERSIONS, (issue) => `unsupported version ${JSON.stringify(issue.input)}`),
      generatedAt: v.optional(v.number()),
      isComplete: v.optional(v.literal(true)),
      tracks: v.array(trackSchema(catalogNamespace)),
      publishTracks: v.optional(v.array(trackSchema(catalogNamespace))),
      initDataList: v.optional(v.array(InitDataSchema)),
    }),
    v.check(
      ({ tracks, initDataList }) =>
        tracks.every((track) => track.initRef === undefined || initDataList?.some((e) => e.id === track.initRef)),
      'an initRef names no initDataList entry (MSF 5.2.14)',
    ),
  )

const nameRef = { name: v.string(), namespace: v.optional(v.string()) }
const parentRef = v.looseObject({ parentName: v.string(), parentNamespace: v.optional(v.string()) })

/** A delta update (MSF 5.1.6, 5.3): operations and `generatedAt`, no other root field. */
export const DeltaSchema = v.looseObject({
  generatedAt: v.number(),
  deltaUpdate: v.pipe(
    v.array(
      v.variant('op', [
        v.looseObject({ op: v.literal('add'), tracks: v.array(v.looseObject(nameRef)) }),
        v.looseObject({ op: v.literal('remove'), tracks: v.array(v.strictObject(nameRef)) }),
        v.looseObject({ op: v.literal('clone'), tracks: v.array(parentRef) }),
        v.looseObject({ op: v.literal('update'), tracks: v.array(parentRef) }),
      ]),
    ),
    v.minLength(1),
  ),
  version: v.optional(v.never()),
  isComplete: v.optional(v.never()),
  tracks: v.optional(v.never()),
  publishTracks: v.optional(v.never()),
  initDataList: v.optional(v.never()),
})

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest
  const { Tuple } = await import('@/model/common/tuple')
  const { PlayerError } = await import('@/player/error')
  const namespace = Tuple.fromUtf8Path('live')
  const TrackSchema = trackSchema(namespace)
  const video = { name: 'v', packaging: 'loc', role: 'video', isLive: true, codec: 'av01', framerate: 30 }
  const issue = (input: unknown) => {
    const result = v.safeParse(TrackSchema, input)
    return result.success ? undefined : v.summarize(result.issues)
  }

  describe('trackSchema', () => {
    test('keeps unknown fields, inherits the namespace, and transforms its fields', () => {
      const output = v.parse(TrackSchema, {
        ...video,
        custom: { a: 1 },
        packagingMetadata: 'streamMapping=object;streamMapping=group;bad;x=1',
        template: [0, 2002, [0, 0], [1, 0], 1759924158381, 2002],
      })
      expect(output['custom']).toEqual({ a: 1 })
      expect(output.namespace).toBe(namespace)
      expect(output.nameBytes).toEqual(new TextEncoder().encode('v'))
      expect(output.packagingMetadata).toEqual({ streamMapping: 'object' })
      expect(output.template).toEqual({
        startMediaTime: 0,
        deltaMediaTime: 2002,
        startLocation: [0, 0],
        deltaLocation: [1, 0],
        startWallclock: 1759924158381,
        deltaWallclock: 2002,
      })
      const other = v.parse(TrackSchema, { ...video, packaging: 'cmaf', namespace: 'a/b', packagingMetadata: 'x=1' })
      expect(other.namespace.fields.map((f) => f.toUtf8())).toEqual(['a', 'b'])
      expect(other.packagingMetadata).toEqual({})
    })

    test('requires the MSF 5.2.4.1 fields per (packaging, role)', () => {
      expect(issue(video)).toBeUndefined()
      expect(issue({ ...video, framerate: undefined })).toMatch(/missing framerate/)
      expect(issue({ ...video, role: 'audio' })).toMatch(/missing samplerate, channelConfig/)
      expect(issue({ name: 'e', packaging: 'eventtimeline', role: 'eventtimeline', isLive: true })).toMatch(/eventType/)
      expect(issue({ ...video, packaging: 'cmaf', codec: undefined })).toBeUndefined()
      expect(issue({ ...video, isLive: undefined })).toMatch(/isLive/)
    })

    test('checks the cross-field rules', () => {
      expect(issue({ name: 'e', packaging: 'eventtimeline', isLive: true })).toMatch(/MSF 5.2.6/)
      expect(issue({ ...video, eventType: 'x' })).toMatch(/MSF 5.2.6/)
      expect(issue({ ...video, targetLatency: 1, buffers: {} })).toMatch(/MSF 5.2.9/)
      expect(issue({ ...video, trackDuration: 1 })).toMatch(/MSF 5.2.36/)
      expect(issue({ ...video, isLive: false, trackDuration: 1 })).toBeUndefined()
    })

    test('a bad name or namespace throws the catalog-wide error', () => {
      expect(() => issue({ ...video, name: 'v\uDEAD' })).toThrow(PlayerError)
      expect(() => issue({ ...video, namespace: 'a//b' })).toThrow(/MSF 5.2.2/)
    })
  })

  describe('catalogSchema', () => {
    const schema = catalogSchema(namespace)
    const parse = (root: Record<string, unknown>) => v.safeParse(schema, { version: 'draft-01', tracks: [], ...root })
    const summary = (root: Record<string, unknown>) => {
      const result = parse(root)
      return result.success ? '' : v.summarize(result.issues)
    }

    test('decodes initDataList entries', () => {
      const result = parse({
        initDataList: [
          { id: 'a', type: 'inline', data: btoa('init') },
          { id: 'p', type: 'track-property', data: '0x0D' },
        ],
      })
      expect(result.success && result.output.initDataList).toEqual([
        { id: 'a', init: { type: 'inline', data: new TextEncoder().encode('init') } },
        { id: 'p', init: { type: 'track-property', property: 0x0d } },
      ])
    })

    test('rejects a broken initRef, bad init data, isComplete: false, and an unsupported version', () => {
      expect(summary({ tracks: [{ ...video, initRef: 'x' }] })).toMatch(/MSF 5.2.14/)
      expect(summary({ initDataList: [{ id: 'a', type: 'inline', data: 'AAAA...' }] })).toMatch(/MSF 5.1.7/)
      expect(summary({ initDataList: [{ id: 'a', type: 'other', data: '' }] })).not.toBe('')
      expect(summary({ isComplete: false })).toMatch(/isComplete/)
      expect(summary({ version: '1' })).toMatch(/unsupported version "1"/)
    })
  })
}
