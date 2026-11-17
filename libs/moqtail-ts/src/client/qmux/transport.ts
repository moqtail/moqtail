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

import { logger } from '@/util/logger'
import { withTimeout } from '../util/timeout'
import { QmuxSession, type RecordSocket } from './session'

/**
 * @public
 * Which transport {@link MOQtailClient} opens: `webtransport` or `websocket` only that
 * one; `auto` WebTransport when the runtime has it and it connects in time, otherwise
 * WebSocket.
 */
export type TransportMode = 'auto' | 'webtransport' | 'websocket'

/** @public How long `auto` waits for WebTransport before trying WebSocket. */
export const DEFAULT_WEBTRANSPORT_TIMEOUT_MS = 3000

/**
 * @public
 * The members of `WebTransport` the client uses. A QMux session over WebSocket provides
 * the same ones, so the rest of the client runs on either.
 */
export type MoqtTransport = Pick<
  WebTransport,
  | 'ready'
  | 'closed'
  | 'close'
  | 'createBidirectionalStream'
  | 'createUnidirectionalStream'
  | 'incomingBidirectionalStreams'
  | 'incomingUnidirectionalStreams'
  | 'datagrams'
>

/** The QMux version this client speaks, as moq-dev puts it in front of the MoQ version. */
const QMUX_PREFIX = 'qmux-02.'

/** WebSocket subprotocols for the MoQ `versions`: moq-dev's prefixed form first, then bare. */
export function webSocketProtocols(versions: readonly string[]): string[] {
  return versions.flatMap((v) => [QMUX_PREFIX + v, v])
}

/** The `ws(s)://` twin of an `http(s)://` URL. */
export function toWebSocketUrl(url: string): string {
  const u = new URL(url)
  u.protocol = u.protocol === 'http:' ? 'ws:' : 'wss:'
  return u.toString()
}

/** Opens a WebSocket to `url` and runs QMux on it; resolves once the session is up. */
export async function connectWebSocket(url: string, versions: readonly string[]): Promise<QmuxSession> {
  const protocols = webSocketProtocols(versions)
  const ws = new WebSocket(toWebSocketUrl(url), protocols)
  ws.binaryType = 'arraybuffer'
  await new Promise<void>((resolve, reject) => {
    ws.onopen = () => resolve()
    ws.onerror = () => reject(new Error(`WebSocket connection to ${url} failed`))
    ws.onclose = (e) => reject(new Error(`WebSocket closed during the handshake: ${e.code} ${e.reason}`))
  })
  // A relay that answers without a subprotocol didn't agree on a MoQ version.
  if (!protocols.includes(ws.protocol)) {
    ws.close()
    throw new Error(`The relay chose no MoQ version (offered ${protocols.join(', ')})`)
  }
  const socket: RecordSocket = {
    // Records come from Writer.finish(), always backed by a plain ArrayBuffer.
    send: (record) => ws.send(record as Uint8Array<ArrayBuffer>),
    get bufferedAmount() {
      return ws.bufferedAmount
    },
    close: () => ws.close(1000),
  }
  const session = new QmuxSession(socket, true)
  ws.onmessage = (e: MessageEvent) => {
    if (e.data instanceof ArrayBuffer) session.onRecord(new Uint8Array(e.data))
    else session.onSocketClosed('the relay sent a text message')
  }
  ws.onclose = (e) => session.onSocketClosed(`WebSocket closed: ${e.code} ${e.reason}`)
  ws.onerror = null
  await session.ready
  return session
}

/**
 * Opens the session's transport. `options.protocols` are the MoQ versions to offer,
 * over either transport.
 */
export async function openTransport(
  url: string,
  options: WebTransportOptions & { protocols?: string[] },
  mode: TransportMode,
  webTransportTimeoutMs: number,
): Promise<{ transport: MoqtTransport; kind: 'webtransport' | 'websocket' }> {
  if (mode !== 'websocket') {
    if (typeof globalThis.WebTransport === 'undefined') {
      if (mode === 'webtransport') throw new Error('WebTransport is not available in this runtime')
      logger.log('MOQtailClient', 'No WebTransport in this runtime; using WebSocket')
    } else {
      const webTransport = new WebTransport(url, options)
      webTransport.closed.catch(() => {})
      if (mode === 'webtransport') {
        await webTransport.ready
        return { transport: webTransport, kind: 'webtransport' }
      }
      try {
        await withTimeout(webTransport.ready, webTransportTimeoutMs, 'WebTransport did not connect in time')
        return { transport: webTransport, kind: 'webtransport' }
      } catch (error) {
        logger.warn('MOQtailClient', 'WebTransport failed, falling back to WebSocket:', error)
        try {
          webTransport.close()
        } catch {
          // Never connected.
        }
      }
    }
  }
  const session = await connectWebSocket(url, options.protocols ?? [])
  return { transport: session as unknown as MoqtTransport, kind: 'websocket' }
}
