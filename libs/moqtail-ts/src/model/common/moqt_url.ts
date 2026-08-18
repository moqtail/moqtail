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

import { ProtocolViolationError } from '../error/error'

/**
 * @public
 * A `moqt://` fragment, `#<type>:<value>`. Processed locally and never sent to the
 * server (§3.1).
 */
export interface MoqtFragment {
  kind: string
  value: string
}

/**
 * @public
 * A parsed `moqt://authority/path[?query][#fragment]` URI (draft-18 §3.1).
 *
 * `moqt-URI = "moqt" "://" authority path-abempty [ "?" query ]`, with an optional
 * local-only fragment. `authority` is `host[:port]`; `path` keeps its leading `/`
 * (empty when absent); `query` excludes the `?`.
 *
 * This is the TypeScript twin of `MoqtUrl` in `apps/client/src/connection.rs`; the two
 * must agree on what a given URL means.
 */
export class MoqtUrl {
  constructor(
    public readonly authority: string,
    public readonly path: string,
    public readonly query: string | undefined,
    public readonly fragment: MoqtFragment | undefined,
  ) {}

  static parse(input: string): MoqtUrl {
    if (!input.startsWith('moqt://'))
      throw new ProtocolViolationError('MoqtUrl.parse', `URL must use the moqt:// scheme: ${input}`)
    let rest = input.slice('moqt://'.length)

    // The fragment is split off first; it never reaches authority/path/query.
    let fragment: MoqtFragment | undefined
    const hashIdx = rest.indexOf('#')
    if (hashIdx !== -1) {
      fragment = parseFragment(rest.slice(hashIdx + 1))
      rest = rest.slice(0, hashIdx)
    }

    // Then the query, then the path, leaving the authority.
    let query: string | undefined
    const queryIdx = rest.indexOf('?')
    if (queryIdx !== -1) {
      query = rest.slice(queryIdx + 1)
      rest = rest.slice(0, queryIdx)
    }

    const slashIdx = rest.indexOf('/')
    const authority = slashIdx === -1 ? rest : rest.slice(0, slashIdx)
    const path = slashIdx === -1 ? '' : rest.slice(slashIdx)

    if (authority.length === 0)
      throw new ProtocolViolationError('MoqtUrl.parse', `moqt:// URL has an empty authority: ${input}`)

    return new MoqtUrl(authority, path, query, fragment)
  }

  /** The `path[?query]` string carried in the PATH setup option on native QUIC. */
  pathAndQuery(): string {
    return this.query !== undefined ? `${this.path}?${this.query}` : this.path
  }

  /** The equivalent `https://` URL for WebTransport, dropping the local fragment. */
  toHttps(): string {
    return `https://${this.authority}${this.pathAndQuery()}`
  }
}

/**
 * Parses a fragment `<type>:<value>`. The type identifier must be ASCII lowercase
 * letters, digits, and hyphens; the value is opaque here.
 */
function parseFragment(fragment: string): MoqtFragment {
  const colonIdx = fragment.indexOf(':')
  if (colonIdx === -1)
    throw new ProtocolViolationError('MoqtUrl.parse', `moqt:// fragment must be <type>:<value>, got #${fragment}`)
  const kind = fragment.slice(0, colonIdx)
  if (kind.length === 0 || !/^[a-z0-9-]+$/.test(kind))
    throw new ProtocolViolationError('MoqtUrl.parse', `moqt:// fragment type must be [a-z0-9-]+, got #${fragment}`)
  return { kind, value: fragment.slice(colonIdx + 1) }
}

/**
 * @public
 * Resolves a session URL to the URL WebTransport is actually opened with.
 *
 * `moqt://` is the scheme callers are meant to use (§3.1); it is mapped to the
 * equivalent `https://` here because the browser's WebTransport constructor only speaks
 * HTTP/3. An `https://` input is passed through unchanged so existing callers keep
 * working. Over WebTransport the authority and path travel in the HTTP CONNECT request,
 * so neither is repeated as a setup option -- AUTHORITY over WebTransport is a protocol
 * violation, and PATH would be redundant.
 */
export function resolveTransportUrl(url: string | URL): { transportUrl: string; moqtUrl: MoqtUrl | undefined } {
  const raw = typeof url === 'string' ? url : url.toString()
  if (!raw.startsWith('moqt://')) return { transportUrl: raw, moqtUrl: undefined }
  const moqtUrl = MoqtUrl.parse(raw)
  return { transportUrl: moqtUrl.toHttps(), moqtUrl }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest

  describe('MoqtUrl', () => {
    test('authority only', () => {
      const url = MoqtUrl.parse('moqt://host:4433')
      expect(url.authority).toBe('host:4433')
      expect(url.path).toBe('')
      expect(url.query).toBeUndefined()
      expect(url.toHttps()).toBe('https://host:4433')
    })

    test('path and query', () => {
      const url = MoqtUrl.parse('moqt://host:4433/moq-relay?a=1')
      expect(url.authority).toBe('host:4433')
      expect(url.path).toBe('/moq-relay')
      expect(url.query).toBe('a=1')
      expect(url.pathAndQuery()).toBe('/moq-relay?a=1')
      expect(url.toHttps()).toBe('https://host:4433/moq-relay?a=1')
    })

    test('fragment is parsed and kept out of the path', () => {
      const url = MoqtUrl.parse('moqt://host/app?q=1#warp:abc')
      expect(url.path).toBe('/app')
      expect(url.query).toBe('q=1')
      expect(url.fragment).toEqual({ kind: 'warp', value: 'abc' })
      // The fragment is local-only, so it must not survive into the transport URL.
      expect(url.toHttps()).toBe('https://host/app?q=1')
    })

    test('a fragment value may itself contain colons', () => {
      expect(MoqtUrl.parse('moqt://host#loc:a:b:c').fragment).toEqual({ kind: 'loc', value: 'a:b:c' })
    })

    test('rejects a non-moqt scheme, an empty authority, and a malformed fragment', () => {
      expect(() => MoqtUrl.parse('https://host:4433')).toThrow()
      expect(() => MoqtUrl.parse('moqt:///path')).toThrow()
      expect(() => MoqtUrl.parse('moqt://host#nocolon')).toThrow()
      expect(() => MoqtUrl.parse('moqt://host#UPPER:x')).toThrow()
    })
  })

  describe('resolveTransportUrl', () => {
    test('maps moqt:// to https:// and reports the parsed URL', () => {
      const { transportUrl, moqtUrl } = resolveTransportUrl('moqt://127.0.0.1:4433/relay#msf:ns')
      expect(transportUrl).toBe('https://127.0.0.1:4433/relay')
      expect(moqtUrl?.fragment).toEqual({ kind: 'msf', value: 'ns' })
    })

    test('passes an https:// URL through untouched', () => {
      const { transportUrl, moqtUrl } = resolveTransportUrl('https://relay.example.com/x')
      expect(transportUrl).toBe('https://relay.example.com/x')
      expect(moqtUrl).toBeUndefined()
    })
  })
}
