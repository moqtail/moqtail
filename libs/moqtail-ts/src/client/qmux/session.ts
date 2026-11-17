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

import {
  DEFAULT_MAX_RECORD_SIZE,
  type Frame,
  QmuxErrorCode,
  QmuxProtocolError,
  STREAM_HEADER_MAX,
  type TransportParams,
  Writer,
  decodeRecord,
  encodeFrame,
  varintLength,
} from './codec'

/**
 * A QMux connection over anything that carries whole records: QUIC-style streams, flow
 * control and datagrams, exposed with the same shape as the browser's `WebTransport`
 * so the client uses either without knowing which. Mirrors
 * `libs/moqtail-rs/src/transport/qmux/session.rs`.
 */

/** What a session needs from the socket under it. */
export interface RecordSocket {
  send(record: Uint8Array): void
  /** Bytes handed to `send` and not yet written to the network. */
  readonly bufferedAmount: number
  close(): void
}

/** Receive window per stream, and for the whole connection. */
const STREAM_WINDOW = 1 << 20
const CONN_WINDOW = 16 << 20
/** Unsent bytes a stream may queue before `write` waits. */
const SEND_QUEUE_LIMIT = 256 * 1024
/** Bytes left in the socket's own buffer before we stop handing it records. Keeping it
 * shallow keeps unsent data in our queues, where resets and priorities still apply. */
const SOCKET_BUFFER_LIMIT = 256 * 1024
const DATAGRAM_QUEUE = 64
const PING_INTERVAL_MS = 5000
const IDLE_TIMEOUT_MS = 30000

export type QmuxConfig = { maxStreamsBidi: number; maxStreamsUni: number }
const DEFAULT_CONFIG: QmuxConfig = { maxStreamsBidi: 100, maxStreamsUni: 1000 }

/** A peer reset or stop, shaped like a `WebTransportError` so existing handling reads it. */
export class QmuxStreamError extends Error {
  readonly source = 'stream'
  constructor(readonly streamErrorCode: number) {
    super(`stream reset with code ${streamErrorCode}`)
    this.name = 'QmuxStreamError'
  }
}

/** The session ended under a read or write, shaped like a `WebTransportError`. */
export class QmuxSessionError extends Error {
  readonly source = 'session'
  constructor(message: string) {
    super(message)
    this.name = 'QmuxSessionError'
  }
}

export type QmuxCloseInfo = { closeCode: number; reason: string }

/** The numeric code a reset or stop reason carries; 0 when it carries none. */
function codeOf(reason: unknown): number {
  const code = (reason as { streamErrorCode?: unknown } | null | undefined)?.streamErrorCode
  return typeof code === 'number' ? code : 0
}

type SendHalf = {
  queue: Uint8Array[]
  queued: number
  /** Bytes handed to the socket so far. */
  offset: number
  /** Highest offset the peer lets us send. */
  max: number
  fin: 'open' | 'queued' | 'sent'
  reset: number | undefined
  stopped: number | undefined
  priority: number
  blockedAt: number | undefined
  /** `write` calls waiting for queue space. */
  waiters: Array<() => void>
  controller: WritableStreamDefaultController | undefined
}

type RecvHalf = {
  chunks: Uint8Array[]
  /** Next offset expected from the peer. */
  received: number
  /** Bytes handed to the application's stream (or discarded for it). */
  consumed: number
  /** Highest offset we let the peer send. */
  max: number
  window: number
  fin: number | undefined
  reset: number | undefined
  stopped: boolean
  closed: boolean
  /** A pending `pull` waiting for data. */
  wake: (() => void) | undefined
  controller: ReadableStreamDefaultController<Uint8Array> | undefined
}

type Stream = { send: SendHalf | undefined; recv: RecvHalf | undefined }

const sendHalf = (max: number, priority = 0): SendHalf => ({
  queue: [],
  queued: 0,
  offset: 0,
  max,
  fin: 'open',
  reset: undefined,
  stopped: undefined,
  priority,
  blockedAt: undefined,
  waiters: [],
  controller: undefined,
})

const recvHalf = (window: number): RecvHalf => ({
  chunks: [],
  received: 0,
  consumed: 0,
  max: window,
  window,
  fin: undefined,
  reset: undefined,
  stopped: false,
  closed: false,
  wake: undefined,
  controller: undefined,
})

const sendDone = (s: SendHalf) => s.reset !== undefined || s.fin === 'sent'
const recvDone = (r: RecvHalf) => r.reset !== undefined || r.stopped || r.closed

export type QmuxBidiStream = { readable: ReadableStream<Uint8Array>; writable: WritableStream<Uint8Array> }

export class QmuxSession {
  readonly ready: Promise<void>
  readonly closed: Promise<QmuxCloseInfo>
  readonly incomingBidirectionalStreams: ReadableStream<QmuxBidiStream>
  readonly incomingUnidirectionalStreams: ReadableStream<ReadableStream<Uint8Array>>
  readonly datagrams: {
    readable: ReadableStream<Uint8Array>
    writable: WritableStream<Uint8Array>
    createWritable(): WritableStream<Uint8Array>
  }
  /** Last measured round-trip time, from QX_PING. */
  rttMs: number | undefined

  readonly #socket: RecordSocket
  readonly #isClient: boolean
  readonly #local: TransportParams
  #peer: TransportParams | undefined
  #paramsSent = false

  #nextBidi = 0
  #nextUni = 0
  #peerMaxBidi = 0
  #peerMaxUni = 0
  #peerOpenedBidi = 0
  #peerOpenedUni = 0
  #localMaxBidi: number
  #localMaxUni: number
  #maxStreamsDirty = { bidi: false, uni: false }
  #openWaiters: Array<() => void> = []

  readonly #streams = new Map<number, Stream>()
  #sendMaxData = 0
  #sentData = 0
  #dataBlockedAt: number | undefined
  #recvMaxData = CONN_WINDOW
  #recvData = 0
  #consumedData = 0
  #maxDataDirty = false
  readonly #creditDirty = new Set<number>()

  readonly #control: Frame[] = []
  readonly #datagramsOut: Uint8Array[] = []
  #close: QmuxCloseInfo | undefined
  #rrCursor = -1
  #pumpScheduled = false
  #retryTimer: ReturnType<typeof setTimeout> | undefined
  #keepalive: ReturnType<typeof setInterval> | undefined
  #pingSeq = 0
  #pingSent: { seq: number; at: number } | undefined
  #lastRx = Date.now()

  #incomingBidi!: ReadableStreamDefaultController<QmuxBidiStream>
  #incomingUni!: ReadableStreamDefaultController<ReadableStream<Uint8Array>>
  #datagramsIn!: ReadableStreamDefaultController<Uint8Array>
  #resolveReady!: () => void
  #rejectReady!: (e: Error) => void
  #resolveClosed!: (info: QmuxCloseInfo) => void

  constructor(socket: RecordSocket, isClient: boolean, config: QmuxConfig = DEFAULT_CONFIG) {
    this.#socket = socket
    this.#isClient = isClient
    this.#local = {
      maxIdleTimeoutMs: IDLE_TIMEOUT_MS,
      initialMaxData: CONN_WINDOW,
      initialMaxStreamDataBidiLocal: STREAM_WINDOW,
      initialMaxStreamDataBidiRemote: STREAM_WINDOW,
      initialMaxStreamDataUni: STREAM_WINDOW,
      initialMaxStreamsBidi: config.maxStreamsBidi,
      initialMaxStreamsUni: config.maxStreamsUni,
      maxDatagramFrameSize: DEFAULT_MAX_RECORD_SIZE,
      maxRecordSize: DEFAULT_MAX_RECORD_SIZE,
    }
    this.#localMaxBidi = config.maxStreamsBidi
    this.#localMaxUni = config.maxStreamsUni
    this.ready = new Promise((resolve, reject) => {
      this.#resolveReady = resolve
      this.#rejectReady = reject
    })
    this.ready.catch(() => {})
    this.closed = new Promise((resolve) => (this.#resolveClosed = resolve))
    this.incomingBidirectionalStreams = new ReadableStream({ start: (c) => void (this.#incomingBidi = c) })
    this.incomingUnidirectionalStreams = new ReadableStream({ start: (c) => void (this.#incomingUni = c) })
    const datagramWritable = () => new WritableStream<Uint8Array>({ write: (chunk) => this.#sendDatagram(chunk) })
    this.datagrams = {
      readable: new ReadableStream({ start: (c) => void (this.#datagramsIn = c) }),
      writable: datagramWritable(),
      createWritable: datagramWritable,
    }
    this.#keepalive = setInterval(() => this.#ping(), PING_INTERVAL_MS)
    ;(this.#keepalive as { unref?: () => void }).unref?.()
    this.#schedule()
  }

  // ---- WebTransport-shaped API ------------------------------------------------------

  async createBidirectionalStream(): Promise<QmuxBidiStream> {
    const id = await this.#open(true)
    return { readable: this.#readable(id), writable: this.#writable(id) }
  }

  async createUnidirectionalStream(options?: { sendOrder?: number | null }): Promise<WritableStream<Uint8Array>> {
    const id = await this.#open(false)
    const send = this.#streams.get(id)?.send
    if (send) send.priority = options?.sendOrder ?? 0
    return this.#writable(id)
  }

  close(info?: { closeCode?: number; reason?: string }): void {
    const closeCode = info?.closeCode ?? 0
    const reason = info?.reason ?? ''
    this.#shut(
      { closeCode, reason },
      {
        type: 'connection_close',
        app: true,
        code: closeCode,
        frameType: 0,
        reason: new TextEncoder().encode(reason),
      },
    )
  }

  // ---- socket input ---------------------------------------------------------------

  /** Applies one record from the socket. */
  onRecord(record: Uint8Array): void {
    this.#lastRx = Date.now()
    if (this.#close) return
    try {
      if (record.length > this.#local.maxRecordSize)
        throw new QmuxProtocolError(QmuxErrorCode.FRAME_ENCODING_ERROR, 'record larger than max_record_size')
      for (const frame of decodeRecord(record)) {
        this.#handle(frame)
        if (this.#close) return
      }
    } catch (e) {
      if (!(e instanceof QmuxProtocolError)) throw e
      this.#fail(e)
    }
  }

  /** The socket closed under the session. */
  onSocketClosed(reason: string): void {
    this.#shut({ closeCode: 0, reason }, undefined)
  }

  // ---- connection lifecycle ---------------------------------------------------------

  #shut(info: QmuxCloseInfo, frame: Frame | undefined): void {
    if (this.#close) return
    this.#close = info
    clearInterval(this.#keepalive)
    clearTimeout(this.#retryTimer)
    if (frame) {
      // Our CONNECTION_CLOSE goes out at once, ahead of anything still queued.
      const w = new Writer()
      if (!this.#paramsSent) encodeFrame({ type: 'transport_parameters', params: this.#local }, w)
      encodeFrame(frame, w)
      try {
        this.#socket.send(w.finish())
      } catch {
        // The socket is already gone.
      }
    }
    try {
      this.#socket.close()
    } catch {
      // Already closed.
    }
    const error = new QmuxSessionError(`session closed: ${info.reason || info.closeCode}`)
    for (const stream of this.#streams.values()) {
      if (stream.send) {
        stream.send.queue = []
        stream.send.queued = 0
        stream.send.controller?.error(error)
        for (const wake of stream.send.waiters.splice(0)) wake()
      }
      if (stream.recv && !stream.recv.closed) {
        stream.recv.controller?.error(error)
        stream.recv.wake?.()
      }
    }
    for (const c of [this.#incomingBidi, this.#incomingUni, this.#datagramsIn]) {
      try {
        c.close()
      } catch {
        // Already closed or errored.
      }
    }
    for (const wake of this.#openWaiters.splice(0)) wake()
    this.#rejectReady(error)
    this.#resolveClosed(info)
  }

  #fail(error: QmuxProtocolError): void {
    this.#shut(
      { closeCode: error.code, reason: error.message },
      {
        type: 'connection_close',
        app: false,
        code: error.code,
        frameType: 0,
        reason: new TextEncoder().encode(error.message),
      },
    )
  }

  #ping(): void {
    if (this.#close) return
    if (Date.now() - this.#lastRx > IDLE_TIMEOUT_MS) {
      this.#shut({ closeCode: 0, reason: 'idle timeout' }, undefined)
      return
    }
    this.#pingSeq += 1
    this.#pingSent = { seq: this.#pingSeq, at: Date.now() }
    this.#queueControl({ type: 'ping_request', seq: this.#pingSeq })
  }

  #closedError(): QmuxSessionError {
    return new QmuxSessionError(`session closed: ${this.#close?.reason || this.#close?.closeCode}`)
  }

  // ---- incoming frames ---------------------------------------------------------------

  #handle(frame: Frame): void {
    if (!this.#peer) {
      if (frame.type !== 'transport_parameters')
        throw new QmuxProtocolError(QmuxErrorCode.PROTOCOL_VIOLATION, 'first frame must be the transport parameters')
      this.#peer = frame.params
      this.#sendMaxData = frame.params.initialMaxData
      this.#peerMaxBidi = frame.params.initialMaxStreamsBidi
      this.#peerMaxUni = frame.params.initialMaxStreamsUni
      for (const wake of this.#openWaiters.splice(0)) wake()
      this.#resolveReady()
      this.#schedule()
      return
    }
    switch (frame.type) {
      case 'padding':
      case 'data_blocked':
      case 'stream_data_blocked':
      case 'streams_blocked':
        return
      case 'ping_response':
        if (this.#pingSent?.seq === frame.seq) {
          this.rttMs = Date.now() - this.#pingSent.at
          this.#pingSent = undefined
        }
        return
      case 'ping_request':
        return this.#queueControl({ type: 'ping_response', seq: frame.seq })
      case 'transport_parameters':
        throw new QmuxProtocolError(QmuxErrorCode.PROTOCOL_VIOLATION, 'transport parameters sent twice')
      case 'stream':
        return this.#onStream(frame.id, frame.offset, frame.data, frame.fin)
      case 'reset_stream':
        return this.#onReset(frame.id, frame.code, frame.finalSize)
      case 'stop_sending':
        return this.#onStopSending(frame.id, frame.code)
      case 'max_data':
        if (frame.max > this.#sendMaxData) {
          this.#sendMaxData = frame.max
          this.#schedule()
        }
        return
      case 'max_stream_data': {
        this.#checkStream(frame.id, true, false)
        const send = this.#streams.get(frame.id)?.send
        if (send && frame.max > send.max) {
          send.max = frame.max
          this.#schedule()
        }
        return
      }
      case 'max_streams':
        if (frame.max > 2 ** 60)
          throw new QmuxProtocolError(QmuxErrorCode.FRAME_ENCODING_ERROR, 'stream limit above 2^60')
        if (frame.bidi) this.#peerMaxBidi = Math.max(this.#peerMaxBidi, frame.max)
        else this.#peerMaxUni = Math.max(this.#peerMaxUni, frame.max)
        for (const wake of this.#openWaiters.splice(0)) wake()
        return
      case 'connection_close':
        return this.#shut({ closeCode: frame.code, reason: new TextDecoder().decode(frame.reason) }, undefined)
      case 'datagram': {
        // Drop rather than buffer without bound when the application isn't reading.
        if ((this.#datagramsIn.desiredSize ?? 0) > -DATAGRAM_QUEUE) this.#datagramsIn.enqueue(frame.data.slice())
        return
      }
    }
  }

  #isLocal(id: number): boolean {
    return (id % 2 === 0) === this.#isClient
  }

  /** Checks a frame's stream id against who may send what, and opens any peer streams
   * it implies. The stream may since have been closed and forgotten. */
  #checkStream(id: number, needsSend: boolean, needsRecv: boolean): void {
    const bidi = (id & 2) === 0
    const index = Math.floor(id / 4)
    const stateError = (reason: string) => new QmuxProtocolError(QmuxErrorCode.STREAM_STATE_ERROR, reason)
    if (this.#isLocal(id)) {
      if (index >= (bidi ? this.#nextBidi : this.#nextUni))
        throw stateError('frame for a stream this endpoint never opened')
      if (!bidi && needsRecv) throw stateError('data on our own unidirectional stream')
      return
    }
    if (!bidi && needsSend) throw stateError("send-side frame for the peer's unidirectional stream")
    const opened = bidi ? this.#peerOpenedBidi : this.#peerOpenedUni
    const limit = bidi ? this.#localMaxBidi : this.#localMaxUni
    if (index >= limit)
      throw new QmuxProtocolError(QmuxErrorCode.STREAM_LIMIT_ERROR, 'peer opened more streams than allowed')
    // Opening a stream opens every lower-numbered one of its type too.
    const peer = this.#peer!
    for (let i = opened; i <= index; i++) {
      const newId = i * 4 + (id & 3)
      this.#streams.set(newId, {
        send: bidi ? sendHalf(peer.initialMaxStreamDataBidiLocal) : undefined,
        recv: recvHalf(bidi ? this.#local.initialMaxStreamDataBidiRemote : this.#local.initialMaxStreamDataUni),
      })
      if (bidi) this.#incomingBidi.enqueue({ readable: this.#readable(newId), writable: this.#writable(newId) })
      else this.#incomingUni.enqueue(this.#readable(newId))
    }
    if (index >= opened) {
      if (bidi) this.#peerOpenedBidi = index + 1
      else this.#peerOpenedUni = index + 1
    }
  }

  /** Counts bytes against the connection window and hands the credit straight back. */
  #discard(len: number): void {
    if (this.#recvData + len > this.#recvMaxData)
      throw new QmuxProtocolError(QmuxErrorCode.FLOW_CONTROL_ERROR, 'peer exceeded the connection window')
    this.#recvData += len
    this.#consumeConn(len)
  }

  #consumeConn(len: number): void {
    this.#consumedData += len
    if (this.#recvMaxData - this.#consumedData < CONN_WINDOW / 2) {
      this.#recvMaxData = this.#consumedData + CONN_WINDOW
      this.#maxDataDirty = true
      this.#schedule()
    }
  }

  #onStream(id: number, offset: number, data: Uint8Array, fin: boolean): void {
    this.#checkStream(id, false, true)
    const len = data.length
    const end = offset + len
    const recv = this.#streams.get(id)?.recv
    if (!recv || recv.reset !== undefined) return this.#discard(len)
    if (offset !== recv.received)
      throw new QmuxProtocolError(QmuxErrorCode.PROTOCOL_VIOLATION, 'stream data out of order')
    if (recv.fin !== undefined && (end > recv.fin || (fin && end !== recv.fin)))
      throw new QmuxProtocolError(QmuxErrorCode.FINAL_SIZE_ERROR, 'stream data past its final size')
    if (end > recv.max) throw new QmuxProtocolError(QmuxErrorCode.FLOW_CONTROL_ERROR, 'peer exceeded the stream window')
    if (this.#recvData + len > this.#recvMaxData)
      throw new QmuxProtocolError(QmuxErrorCode.FLOW_CONTROL_ERROR, 'peer exceeded the connection window')
    recv.received = end
    if (fin) recv.fin = end
    this.#recvData += len
    if (recv.stopped) {
      recv.consumed = end
      this.#consumeConn(len)
    } else if (len > 0) {
      // The record's buffer is reused by nothing, but copying keeps each chunk small.
      recv.chunks.push(data.slice())
    }
    recv.wake?.()
    this.#maybeRemove(id)
  }

  #onReset(id: number, code: number, finalSize: number): void {
    this.#checkStream(id, false, true)
    const recv = this.#streams.get(id)?.recv
    if (!recv || recv.reset !== undefined) return
    if (finalSize < recv.received || (recv.fin !== undefined && recv.fin !== finalSize))
      throw new QmuxProtocolError(QmuxErrorCode.FINAL_SIZE_ERROR, 'reset with a different final size')
    if (finalSize > recv.max)
      throw new QmuxProtocolError(QmuxErrorCode.FLOW_CONTROL_ERROR, 'reset past the stream window')
    const newBytes = finalSize - recv.received
    if (this.#recvData + newBytes > this.#recvMaxData)
      throw new QmuxProtocolError(QmuxErrorCode.FLOW_CONTROL_ERROR, 'reset past the connection window')
    const unread = finalSize - recv.consumed
    recv.received = finalSize
    recv.consumed = finalSize
    recv.chunks = []
    recv.reset = code
    this.#recvData += newBytes
    this.#consumeConn(unread)
    if (!recv.closed) recv.controller?.error(new QmuxStreamError(code))
    recv.wake?.()
    this.#maybeRemove(id)
  }

  #onStopSending(id: number, code: number): void {
    this.#checkStream(id, true, false)
    const send = this.#streams.get(id)?.send
    if (!send) return
    send.stopped = code
    // The peer won't read any more, so stop sending with its code, as QUIC does.
    if (!sendDone(send)) this.#resetSend(id, code)
    send.controller?.error(new QmuxStreamError(code))
  }

  #maybeRemove(id: number): void {
    const stream = this.#streams.get(id)
    if (!stream) return
    if (stream.send && !sendDone(stream.send)) return
    if (stream.recv && !recvDone(stream.recv)) return
    this.#streams.delete(id)
    this.#creditDirty.delete(id)
    if (!this.#isLocal(id)) {
      if ((id & 2) === 0) {
        this.#localMaxBidi += 1
        this.#maxStreamsDirty.bidi = true
      } else {
        this.#localMaxUni += 1
        this.#maxStreamsDirty.uni = true
      }
      this.#schedule()
    }
  }

  // ---- local streams ----------------------------------------------------------------

  async #open(bidi: boolean): Promise<number> {
    for (;;) {
      if (this.#close) throw this.#closedError()
      if (this.#peer) {
        const next = bidi ? this.#nextBidi : this.#nextUni
        if (next < (bidi ? this.#peerMaxBidi : this.#peerMaxUni)) {
          const id = next * 4 + (this.#isClient ? 0 : 1) + (bidi ? 0 : 2)
          this.#streams.set(id, {
            send: sendHalf(bidi ? this.#peer.initialMaxStreamDataBidiRemote : this.#peer.initialMaxStreamDataUni),
            recv: bidi ? recvHalf(this.#local.initialMaxStreamDataBidiLocal) : undefined,
          })
          if (bidi) this.#nextBidi += 1
          else this.#nextUni += 1
          return id
        }
      }
      await new Promise<void>((resolve) => this.#openWaiters.push(resolve))
    }
  }

  #writable(id: number): WritableStream<Uint8Array> {
    const writable = new WritableStream<Uint8Array>({
      start: (controller) => {
        const send = this.#streams.get(id)?.send
        if (send) send.controller = controller
      },
      write: (chunk) => this.#push(id, chunk),
      close: () => this.#finish(id),
      abort: (reason) => this.#reset(id, codeOf(reason)),
    })
    // WebTransport send streams expose their priority this way.
    Object.defineProperty(writable, 'sendOrder', {
      get: () => this.#streams.get(id)?.send?.priority ?? 0,
      set: (value: number | null) => {
        const send = this.#streams.get(id)?.send
        if (send) send.priority = value ?? 0
      },
    })
    return writable
  }

  #sendHalf(id: number): SendHalf {
    if (this.#close) throw this.#closedError()
    const send = this.#streams.get(id)?.send
    if (!send) throw new QmuxSessionError('stream closed')
    if (send.stopped !== undefined) throw new QmuxStreamError(send.stopped)
    if (send.reset !== undefined || send.fin !== 'open') throw new QmuxSessionError('stream closed')
    return send
  }

  async #push(id: number, chunk: Uint8Array): Promise<void> {
    const send = this.#sendHalf(id)
    if (chunk.length > 0) {
      // The caller may reuse its buffer once write resolves; the bytes may not be sent yet.
      send.queue.push(chunk.slice())
      send.queued += chunk.length
      this.#schedule()
    }
    while (send.queued >= SEND_QUEUE_LIMIT) {
      await new Promise<void>((resolve) => send.waiters.push(resolve))
      this.#sendHalf(id)
    }
  }

  #finish(id: number): void {
    const send = this.#sendHalf(id)
    send.fin = 'queued'
    this.#schedule()
  }

  #reset(id: number, code: number): void {
    const send = this.#streams.get(id)?.send
    if (!send || sendDone(send)) return
    this.#resetSend(id, code)
  }

  #resetSend(id: number, code: number): void {
    const send = this.#streams.get(id)?.send
    if (!send) return
    send.queue = []
    send.queued = 0
    send.reset = code
    for (const wake of send.waiters.splice(0)) wake()
    this.#queueControl({ type: 'reset_stream', id, code, finalSize: send.offset })
    this.#maybeRemove(id)
  }

  #readable(id: number): ReadableStream<Uint8Array> {
    return new ReadableStream<Uint8Array>({
      start: (controller) => {
        const recv = this.#streams.get(id)?.recv
        if (recv) recv.controller = controller
      },
      pull: () => this.#pull(id),
      cancel: (reason) => this.#stop(id, codeOf(reason)),
    })
  }

  /** Hands the application the next chunk, waiting for one if needed. Credit goes back
   * as chunks are handed over, so the peer can't run more than a window ahead. */
  async #pull(id: number): Promise<void> {
    for (;;) {
      const recv = this.#streams.get(id)?.recv
      if (!recv || recv.closed || recv.reset !== undefined) return
      const chunk = recv.chunks.shift()
      if (chunk) {
        recv.consumed += chunk.length
        if (recv.fin === undefined && recv.max - recv.consumed < recv.window / 2) {
          recv.max = recv.consumed + recv.window
          this.#creditDirty.add(id)
          this.#schedule()
        }
        this.#consumeConn(chunk.length)
        recv.controller?.enqueue(chunk)
        return
      }
      if (recv.fin === recv.consumed) {
        recv.closed = true
        recv.controller?.close()
        this.#maybeRemove(id)
        return
      }
      if (this.#close) return
      await new Promise<void>((resolve) => (recv.wake = resolve))
      recv.wake = undefined
    }
  }

  #stop(id: number, code: number): void {
    const recv = this.#streams.get(id)?.recv
    if (!recv || recvDone(recv)) return
    recv.stopped = true
    const unread = recv.received - recv.consumed
    recv.consumed = recv.received
    recv.chunks = []
    this.#consumeConn(unread)
    this.#queueControl({ type: 'stop_sending', id, code })
    this.#maybeRemove(id)
  }

  // ---- datagrams ---------------------------------------------------------------------

  #sendDatagram(payload: Uint8Array): void {
    if (this.#close) throw this.#closedError()
    const peer = this.#peer
    if (!peer || peer.maxDatagramFrameSize === 0) throw new QmuxSessionError('peer does not accept datagrams')
    const frameLength = 1 + varintLength(payload.length) + payload.length
    if (frameLength > Math.min(peer.maxDatagramFrameSize, peer.maxRecordSize))
      throw new QmuxSessionError('datagram too large')
    this.#datagramsOut.push(payload.slice())
    if (this.#datagramsOut.length > DATAGRAM_QUEUE) this.#datagramsOut.shift()
    this.#schedule()
  }

  // ---- outgoing ----------------------------------------------------------------------

  #queueControl(frame: Frame): void {
    this.#control.push(frame)
    this.#schedule()
  }

  #schedule(): void {
    if (this.#pumpScheduled || this.#close) return
    this.#pumpScheduled = true
    queueMicrotask(() => {
      this.#pumpScheduled = false
      this.#pump()
    })
  }

  /** Hands records to the socket while its own buffer stays shallow. */
  #pump(): void {
    if (this.#close) return
    while (this.#socket.bufferedAmount < SOCKET_BUFFER_LIMIT) {
      const record = this.buildRecord()
      if (!record) return
      this.#socket.send(record)
    }
    // The socket is full; come back once it has drained a little.
    clearTimeout(this.#retryTimer)
    this.#retryTimer = setTimeout(() => this.#pump(), 5)
  }

  /** The next record to send, or `undefined` when there is nothing to send. */
  buildRecord(): Uint8Array | undefined {
    const max = Math.min(this.#peer?.maxRecordSize ?? DEFAULT_MAX_RECORD_SIZE, 1 << 20)
    const w = new Writer()
    if (!this.#paramsSent) {
      encodeFrame({ type: 'transport_parameters', params: this.#local }, w)
      this.#paramsSent = true
    }
    if (this.#peer) {
      this.#queueCredit()
      while (this.#control.length > 0) {
        const probe = new Writer()
        encodeFrame(this.#control[0]!, probe)
        if (w.length > 0 && w.length + probe.length > max) break
        this.#control.shift()
        w.bytes(probe.finish())
      }
      while (this.#datagramsOut.length > 0 && w.length + STREAM_HEADER_MAX + this.#datagramsOut[0]!.length <= max)
        encodeFrame({ type: 'datagram', data: this.#datagramsOut.shift()! }, w)
      while (w.length + STREAM_HEADER_MAX < max) {
        const id = this.#nextSender()
        if (id === undefined) break
        this.#writeStreamFrame(id, max - w.length - STREAM_HEADER_MAX, w)
      }
    }
    return w.length > 0 ? w.finish() : undefined
  }

  #queueCredit(): void {
    if (this.#maxDataDirty) {
      this.#maxDataDirty = false
      this.#control.push({ type: 'max_data', max: this.#recvMaxData })
    }
    for (const id of this.#creditDirty) {
      const recv = this.#streams.get(id)?.recv
      if (recv && !recvDone(recv)) this.#control.push({ type: 'max_stream_data', id, max: recv.max })
    }
    this.#creditDirty.clear()
    if (this.#maxStreamsDirty.bidi) this.#control.push({ type: 'max_streams', bidi: true, max: this.#localMaxBidi })
    if (this.#maxStreamsDirty.uni) this.#control.push({ type: 'max_streams', bidi: false, max: this.#localMaxUni })
    this.#maxStreamsDirty = { bidi: false, uni: false }
  }

  /** The stream whose data goes next: highest priority first, then round-robin by id.
   * Also signals the peer once per limit when credit runs out. */
  #nextSender(): number | undefined {
    const connCredit = this.#sendMaxData - this.#sentData
    let best: { id: number; priority: number; after: boolean } | undefined
    let connBlocked = false
    // shortcut: scans every stream per frame; index ready streams by priority if
    // sessions with thousands of streams show up in profiles.
    for (const [id, { send }] of this.#streams) {
      if (!send || send.reset !== undefined || (send.queued === 0 && send.fin !== 'queued')) continue
      const finOnly = send.queued === 0
      const streamCredit = send.max > send.offset
      if (!finOnly && !(streamCredit && connCredit > 0)) {
        if (!streamCredit && send.blockedAt !== send.max) {
          send.blockedAt = send.max
          this.#control.push({ type: 'stream_data_blocked', id, limit: send.max })
        }
        connBlocked ||= connCredit === 0
        continue
      }
      // Ids after the cursor come first, so equal priorities take turns.
      const after = id > this.#rrCursor
      if (
        !best ||
        send.priority > best.priority ||
        (send.priority === best.priority && after && !best.after) ||
        (send.priority === best.priority && after === best.after && id < best.id)
      )
        best = { id, priority: send.priority, after }
    }
    if (connBlocked && this.#dataBlockedAt !== this.#sendMaxData) {
      this.#dataBlockedAt = this.#sendMaxData
      this.#control.push({ type: 'data_blocked', limit: this.#sendMaxData })
    }
    return best?.id
  }

  /** Writes one STREAM frame for `id`, copying straight from its queue. */
  #writeStreamFrame(id: number, room: number, w: Writer): void {
    const send = this.#streams.get(id)?.send
    if (!send) return
    const n = Math.min(send.queued, send.max - send.offset, this.#sendMaxData - this.#sentData, room)
    const fin = send.fin === 'queued' && n === send.queued
    w.byte(0x08 | 0x04 | 0x02 | (fin ? 1 : 0))
    w.varint(id)
    w.varint(send.offset)
    w.varint(n)
    let left = n
    while (left > 0) {
      const front = send.queue[0]!
      if (front.length <= left) {
        w.bytes(front)
        send.queue.shift()
        left -= front.length
      } else {
        w.bytes(front.subarray(0, left))
        send.queue[0] = front.subarray(left)
        left = 0
      }
    }
    send.offset += n
    send.queued -= n
    this.#sentData += n
    this.#rrCursor = id
    if (fin) send.fin = 'sent'
    if (send.queued < SEND_QUEUE_LIMIT) for (const wake of send.waiters.splice(0)) wake()
    if (fin) this.#maybeRemove(id)
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest

  /** Two sessions back to back; records cross in a microtask, as over a socket. */
  function pair(): [QmuxSession, QmuxSession] {
    const sessions: QmuxSession[] = []
    const socket = (peer: number): RecordSocket => ({
      send: (record) => queueMicrotask(() => sessions[peer]!.onRecord(record)),
      bufferedAmount: 0,
      close: () => queueMicrotask(() => sessions[peer]!.onSocketClosed('closed')),
    })
    sessions.push(new QmuxSession(socket(1), true), new QmuxSession(socket(0), false))
    return [sessions[0]!, sessions[1]!]
  }

  async function readAll(readable: ReadableStream<Uint8Array>): Promise<Uint8Array> {
    const parts: Uint8Array[] = []
    for await (const chunk of readable) parts.push(chunk)
    const out = new Uint8Array(parts.reduce((n, p) => n + p.length, 0))
    let at = 0
    for (const p of parts) {
      out.set(p, at)
      at += p.length
    }
    return out
  }

  describe('QmuxSession', () => {
    test('echoes data larger than every window over several streams', async () => {
      const [client, server] = pair()
      await Promise.all([client.ready, server.ready])
      const echo = (async () => {
        const reader = server.incomingBidirectionalStreams.getReader()
        for (let i = 0; i < 3; i++) {
          const { value } = await reader.read()
          void value!.readable.pipeTo(value!.writable)
        }
      })()
      const size = 3 * STREAM_WINDOW
      await Promise.all(
        [1, 2, 3].map(async (fill) => {
          const { readable, writable } = await client.createBidirectionalStream()
          const writer = writable.getWriter()
          const sending = (async () => {
            for (let at = 0; at < size; at += 64 * 1024) await writer.write(new Uint8Array(64 * 1024).fill(fill))
            await writer.close()
          })()
          const got = await readAll(readable)
          await sending
          expect(got.length).toBe(size)
          expect(got.every((b) => b === fill)).toBe(true)
        }),
      )
      await echo
    })

    test('a reset reaches the reader with its code', async () => {
      const [client, server] = pair()
      const writable = await client.createUnidirectionalStream()
      const writer = writable.getWriter()
      await writer.write(new Uint8Array([1, 2, 3]))
      const { value: readable } = await server.incomingUnidirectionalStreams.getReader().read()
      await writer.abort({ streamErrorCode: 0x2 })
      const error = await readAll(readable!).catch((e: unknown) => e)
      expect(error).toBeInstanceOf(QmuxStreamError)
      expect((error as QmuxStreamError).streamErrorCode).toBe(0x2)
    })

    test("stopping a stream errors the sender's writes with the same code", async () => {
      const [client, server] = pair()
      const writable = await client.createUnidirectionalStream()
      const writer = writable.getWriter()
      await writer.write(new Uint8Array([1]))
      const { value: readable } = await server.incomingUnidirectionalStreams.getReader().read()
      await readable!.cancel({ streamErrorCode: 0x10 })
      await expect(writer.closed).rejects.toMatchObject({ source: 'stream', streamErrorCode: 0x10 })
    })

    test('higher sendOrder goes first', async () => {
      const [client, server] = pair()
      await client.ready
      const low = await client.createUnidirectionalStream({ sendOrder: 1 })
      const high = await client.createUnidirectionalStream({ sendOrder: 10 })
      void low.getWriter().write(new Uint8Array(20000).fill(1))
      void high.getWriter().write(new Uint8Array(20000).fill(2))
      const record = client.buildRecord()!
      const streams = decodeRecord(record).filter((f) => f.type === 'stream')
      expect(streams.map((f) => (f as { id: number }).id)).toEqual([6])
      void server
    })

    test('the peer sending past a stream window closes the session with FLOW_CONTROL_ERROR', async () => {
      const [client, server] = pair()
      await server.ready
      await client.createUnidirectionalStream()
      // Forge what an honest sender never builds: in-order records that run past the
      // 1 MiB window while nobody reads, so no credit comes back.
      const chunk = 16000
      for (let offset = 0; offset <= STREAM_WINDOW; offset += chunk) {
        const w = new Writer()
        encodeFrame({ type: 'stream', id: 2, offset, data: new Uint8Array(chunk), fin: false }, w)
        server.onRecord(w.finish())
      }
      expect((await server.closed).closeCode).toBe(QmuxErrorCode.FLOW_CONTROL_ERROR)
    })

    test('close reaches the peer with its code and reason, and fails pending work', async () => {
      const [client, server] = pair()
      await client.ready
      const pending = server.incomingUnidirectionalStreams.getReader().read()
      client.close({ closeCode: 0x10, reason: 'bye' })
      expect(await server.closed).toEqual({ closeCode: 0x10, reason: 'bye' })
      expect((await pending).done).toBe(true)
      await expect(client.createUnidirectionalStream()).rejects.toBeInstanceOf(QmuxSessionError)
    })

    test('closed peer streams return stream credit', async () => {
      const [client, server] = pair()
      await client.ready
      const writable = await client.createUnidirectionalStream()
      await writable.getWriter().close()
      const { value: readable } = await server.incomingUnidirectionalStreams.getReader().read()
      expect((await readAll(readable!)).length).toBe(0)
      // The server allows 1000 uni streams at once; opening 1000 more only succeeds once
      // the finished one's credit came back (otherwise this hangs and the test times out).
      for (let i = 0; i < 1000; i++) await client.createUnidirectionalStream()
    })
  })
}
