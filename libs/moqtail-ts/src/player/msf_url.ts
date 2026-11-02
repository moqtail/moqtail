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
 * MSF URLs (MSF 11.1): `moqt://authority/path[?query]#msf:<namespace>--<track>[&name=value...]`.
 * `MoqtUrl` parses the generic `moqt://` part; this file only reads the `msf` fragment.
 */

import { Location } from '../model/common/location'
import { MoqtUrl } from '../model/common/moqt_url'
import { Tuple, TupleField } from '../model/common/tuple'
import { PlayerError } from './error'

/** @public An inclusive range of MOQT Locations. */
export interface LocationRange {
  /** Start Object ID defaults to 0. */
  start: Location
  /** No end: open range. No end object: the whole end group. */
  end?: { group: bigint; object?: bigint }
}

/** @public An inclusive range in milliseconds. No end: open range. */
export interface TimeRange {
  start: number
  end?: number
}

/** @public A parsed MSF URL. */
export interface MsfSource {
  /** `moqt://authority/path[?query]`, lowercase scheme, no fragment. */
  url: string
  /** Namespace fields hold the decoded bytes. */
  namespace: Tuple
  /** The track name decoded as UTF-8, for display and probing. */
  track: string
  /** The track name bytes. Names are compared as bytes (MSF 11.2.2). */
  trackBytes: Uint8Array
  locationRanges: LocationRange[]
  mediaTimeRanges: TimeRange[]
  wallclockRanges: TimeRange[]
  c4m?: string
  connection?: 'q' | 'wt'
  /** Every fragment parameter, last value wins. Used for catalog variable substitution (MSF 5.4). */
  variables: Record<string, string>
}

// moqt-22 §2.4.1
const MAX_NAMESPACE_FIELDS = 32

/**
 * @public
 * Parses an MSF URL (MSF 11.1). Ranges stay in their own domain and aren't merged.
 * @throws PlayerError `UNSUPPORTED_SOURCE` for anything that isn't a valid `msf-uri`.
 */
export function parseMsfUrl(uri: string): MsfSource {
  let url: MoqtUrl
  try {
    url = MoqtUrl.parse(uri)
  } catch (cause) {
    throw new PlayerError('UNSUPPORTED_SOURCE', `Not a moqt:// URL: ${uri}`, { cause })
  }
  if (url.fragment?.kind !== 'msf') unsupported(`missing #msf: fragment in ${uri}`)

  const [trackId = '', ...params] = url.fragment.value.split('&')
  const source: MsfSource = {
    url: `moqt://${url.authority}${url.pathAndQuery()}`,
    ...decodeTrackIdentifier(trackId),
    locationRanges: [],
    mediaTimeRanges: [],
    wallclockRanges: [],
    variables: {},
  }

  for (const param of params) {
    const eq = param.indexOf('=')
    if (eq === -1) unsupported(`parameter without a value: ${param}`)
    const name = percentDecode(param.slice(0, eq))
    const value = percentDecode(param.slice(eq + 1))
    if (name === '') unsupported(`parameter without a name: ${param}`)
    source.variables[name] = value

    switch (name) {
      case 'location-range':
        source.locationRanges.push(parseLocationRange(value))
        break
      case 'mediatime-range':
        source.mediaTimeRanges.push(parseTimeRange(value))
        break
      case 'wallclock-range':
        source.wallclockRanges.push(parseTimeRange(value))
        break
      case 'c4m':
        source.c4m = value
        break
      case 'connection':
        if (value !== 'q' && value !== 'wt') unsupported(`connection must be q or wt, got ${value}`)
        source.connection = value
        break
    }
  }
  return source
}

/**
 * @public
 * Builds an MSF URL from a `moqt://authority/path[?query]` session URL and a full track name.
 */
export function formatMsfUrl(url: string, namespace: Tuple, track: Uint8Array | string): string {
  const trackBytes = typeof track === 'string' ? new TextEncoder().encode(track) : track
  const ns = namespace.fields.map((f) => encodeName(f.value)).join('-')
  return `${url}#msf:${ns}--${encodeName(trackBytes)}`
}

function unsupported(message: string): never {
  throw new PlayerError('UNSUPPORTED_SOURCE', `Invalid MSF URL: ${message}`)
}

function percentDecode(s: string): string {
  try {
    return decodeURIComponent(s)
  } catch (cause) {
    throw new PlayerError('UNSUPPORTED_SOURCE', `Invalid MSF URL: bad percent-encoding in ${s}`, { cause })
  }
}

/** Splits `<namespace>--<track>` at the first `--`, then the namespace at each `-`. */
function decodeTrackIdentifier(id: string): Pick<MsfSource, 'namespace' | 'track' | 'trackBytes'> {
  const sep = id.indexOf('--')
  if (sep === -1) unsupported(`no track name in ${id}`)
  const fields = id.slice(0, sep).split('-')
  if (fields.length > MAX_NAMESPACE_FIELDS) unsupported(`more than ${MAX_NAMESPACE_FIELDS} namespace fields`)
  if (fields.includes('')) unsupported(`empty namespace field in ${id}`)

  const trackBytes = decodeName(id.slice(sep + 2))
  return {
    namespace: new Tuple(fields.map((f) => new TupleField(decodeName(f)))),
    track: new TextDecoder().decode(trackBytes),
    trackBytes,
  }
}

// moqt-22 §8.8: these bytes are written literally, every other byte as `.xx`.
const isLiteral = (byte: number) => /[A-Za-z0-9_]/.test(String.fromCharCode(byte))

function encodeName(bytes: Uint8Array): string {
  let out = ''
  for (const b of bytes) out += isLiteral(b) ? String.fromCharCode(b) : '.' + b.toString(16).padStart(2, '0')
  return out
}

/** Decodes one name, rejecting anything that isn't its canonical encoding (moqt-22 §8.8). */
function decodeName(s: string): Uint8Array {
  const bytes: number[] = []
  for (let i = 0; i < s.length; i++) {
    const code = s.charCodeAt(i)
    if (isLiteral(code)) {
      bytes.push(code)
      continue
    }
    const hex = s.slice(i + 1, i + 3)
    if (s[i] !== '.' || !/^[0-9a-f]{2}$/.test(hex)) unsupported(`non-canonical name ${s}`)
    const byte = parseInt(hex, 16)
    if (isLiteral(byte)) unsupported(`escaped literal .${hex} in ${s}`)
    bytes.push(byte)
    i += 2
  }
  return new Uint8Array(bytes)
}

/** `G[.O][-G[.O]]` (MSF 11.1.1). */
function parseLocationRange(value: string): LocationRange {
  const m = /^(\d+)(?:\.(\d+))?(?:-(\d+)(?:\.(\d+))?)?$/.exec(value)
  if (!m) unsupported(`location-range ${value}`)
  const [, startGroup = '', startObject = '0', endGroup, endObject] = m
  const range: LocationRange = { start: new Location(BigInt(startGroup), BigInt(startObject)) }
  if (endGroup === undefined) return range

  const end = { group: BigInt(endGroup), ...(endObject !== undefined && { object: BigInt(endObject) }) }
  const { group, object } = range.start
  if (group > end.group || (group === end.group && end.object !== undefined && object > end.object))
    unsupported(`location-range starts after it ends: ${value}`)
  range.end = end
  return range
}

/** `ms[-ms]` (MSF 11.1.1). */
function parseTimeRange(value: string): TimeRange {
  const m = /^(\d+)(?:-(\d+))?$/.exec(value)
  if (!m) unsupported(`time range ${value}`)
  const start = Number(m[1])
  const end = m[2] === undefined ? undefined : Number(m[2])
  if (!Number.isSafeInteger(start) || (end !== undefined && !Number.isSafeInteger(end)))
    unsupported(`time range out of bounds: ${value}`)
  if (end === undefined) return { start }
  if (start > end) unsupported(`time range starts after it ends: ${value}`)
  return { start, end }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest
  const utf8 = (t: Tuple) => t.fields.map((f) => f.toUtf8())
  const msf = (fragment: string) => parseMsfUrl(`moqt://example.com/relay#msf:${fragment}`)
  const unsupportedSource = { name: 'PlayerError', code: 'UNSUPPORTED_SOURCE' }

  describe('parseMsfUrl', () => {
    test('MSF 11.1.3 examples', () => {
      const catalog = parseMsfUrl('moqt://example.com/server/config?a=1&b=2#msf:customer-livestream-123--catalog')
      expect(catalog.url).toBe('moqt://example.com/server/config?a=1&b=2')
      expect(utf8(catalog.namespace)).toEqual(['customer', 'livestream', '123'])
      expect(catalog.track).toBe('catalog')
      expect(catalog.variables).toEqual({})

      const base = 'moqt://example.com/relay-app/relayID'
      expect(parseMsfUrl(`${base}#msf:customerID-broadcastID--catalog&connection=q`).connection).toBe('q')
      const video = parseMsfUrl(`${base}#msf:customerID-broadcastID--video&connection=wt`)
      expect([video.track, video.connection]).toEqual(['video', 'wt'])
      expect(parseMsfUrl(`${base}#msf:customerID-broadcastID--catalog&location-range=34-64`).locationRanges).toEqual([
        { start: new Location(34, 0), end: { group: 64n } },
      ])

      const c4m =
        'gqhkYWxnIGVzaGFyqGR0eXBNhdZ9hdWQAY3VybGZlbWlzcwZleWV2aW5uZWlhdGVwQWNyZW5lYnJmcmVqMTIzNDU2NzgwMHZpc3VlZF9hdD0xNzMwNDM'
      expect(parseMsfUrl(`${base}#msf:customerID-broadcastID--catalog&c4m=${c4m}`).c4m).toBe(c4m)
      const withToken = parseMsfUrl(
        `${base}?token=HTRCII74GHFT@JHBCVSW56HKKneH2Dbyq6NHBI2#msf:customerID-broadcastID--catalog&c4m=${c4m}`,
      )
      expect(withToken.url).toBe(`${base}?token=HTRCII74GHFT@JHBCVSW56HKKneH2Dbyq6NHBI2`)
      expect(withToken.variables).toEqual({ c4m })
    })

    test('scheme is normalised to lowercase', () => {
      expect(parseMsfUrl('MOQT://host#msf:a--b').url).toBe('moqt://host')
    })

    test('MSF 11.1.1 range forms', () => {
      const ranges = msf('a--b&location-range=34.0-2145.16&location-range=16.24&location-range=16-24')
      expect(ranges.locationRanges).toEqual([
        { start: new Location(34, 0), end: { group: 2145n, object: 16n } },
        { start: new Location(16, 24) },
        { start: new Location(16, 0), end: { group: 24n } },
      ])
      expect(msf('a--b&mediatime-range=0-13421&mediatime-range=982').mediaTimeRanges).toEqual([
        { start: 0, end: 13421 },
        { start: 982 },
      ])
      expect(
        msf('a--b&wallclock-range=1761759637565-1761759836189&wallclock-range=1761751753894').wallclockRanges,
      ).toEqual([{ start: 1761759637565, end: 1761759836189 }, { start: 1761751753894 }])
      expect(msf('a--b&location-range=5.3-5').locationRanges).toEqual([
        { start: new Location(5, 3), end: { group: 5n } },
      ])
    })

    test('parameters are percent-decoded and the last value wins', () => {
      expect(msf('a--b&x=1&x=2&k%20ey=v%2Fal=ue').variables).toEqual({ x: '2', 'k ey': 'v/al=ue' })
    })

    test('rejects invalid URLs', () => {
      for (const fragment of [
        'a--b&location-range=5.2-5.1',
        'a--b&location-range=6-5',
        'a--b&location-range=-1',
        'a--b&location-range=1.',
        'a--b&mediatime-range=10-5',
        'a--b&mediatime-range=+5',
        'a--b&wallclock-range=99999999999999999999',
        'a--b&connection=h3',
        'a--b&=x',
        'a--b&noequals',
        'a--b&x=%zz',
        'a-b', // no track name
      ])
        expect(() => msf(fragment), fragment).toThrow(expect.objectContaining(unsupportedSource))
      // No msf: type (MSF 5.2.44, 11.5.2) and not a moqt:// URL at all.
      expect(() => parseMsfUrl('moqt://host#namespace--name&token=x')).toThrow(
        expect.objectContaining(unsupportedSource),
      )
      expect(() => parseMsfUrl('moqt://host')).toThrow(expect.objectContaining(unsupportedSource))
      expect(() => parseMsfUrl('https://host#msf:a--b')).toThrow(expect.objectContaining(unsupportedSource))
    })
  })

  describe('namespace-name encoding', () => {
    test('MSF 11.1.2 example', () => {
      const s = msf('customer-livestream-123--caf.c3.a9')
      expect(s.track).toBe('café')
      expect(s.trackBytes).toEqual(new Uint8Array([0x63, 0x61, 0x66, 0xc3, 0xa9]))
      expect(formatMsfUrl('moqt://h', Tuple.fromUtf8Path('customer/livestream/123'), 'café')).toBe(
        'moqt://h#msf:customer-livestream-123--caf.c3.a9',
      )
    })

    test('round trips arbitrary bytes, including delimiters and invalid UTF-8', () => {
      const ns = new Tuple([new TupleField(new Uint8Array([0x2d, 0x2e, 0xff, 0x41])), TupleField.fromUtf8('x.')])
      const track = new Uint8Array([0x00, 0x2d, 0x2d, 0x80])
      const url = formatMsfUrl('moqt://h/p', ns, track)
      expect(url).toBe('moqt://h/p#msf:.2d.2e.ffA-x.2e--.00.2d.2d.80')
      const parsed = parseMsfUrl(url)
      expect(parsed.namespace.equals(ns)).toBe(true)
      expect(parsed.trackBytes).toEqual(track)
    })

    test('a trailing escape is decoded, an empty track name is allowed', () => {
      expect(utf8(msf('ab.2e--c.2e').namespace)).toEqual(['ab.'])
      expect(msf('ab.2e--c.2e').track).toBe('c.')
      expect(msf('a--').trackBytes).toEqual(new Uint8Array())
    })

    test('rejects non-canonical names', () => {
      for (const fragment of [
        'a--.61', // escaped literal
        'a--.2D', // uppercase hex
        'a--.2', // short escape
        'a--x.', // dangling period
        'a--%41', // percent-encoding in the track identifier
        'a--b-c', // bare hyphen in the track name
        'a---b', // same, right after the separator
        '-a--b', // empty namespace field
        'a--b--c',
        '--b', // no namespace
        Array(33).fill('a').join('-') + '--b',
      ])
        expect(() => msf(fragment), fragment).toThrow(expect.objectContaining(unsupportedSource))
      expect(utf8(msf(Array(32).fill('a').join('-') + '--b').namespace)).toHaveLength(32)
    })
  })
}
