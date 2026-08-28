/**
 * Copyright 2025 The MOQtail Authors
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

import { FrozenByteBuffer } from '../common/byte_buffer'
import { ControlMessageType, FetchType } from './constant'
import { PublishBlocked } from './publish_blocked'
import { PublishNamespace } from './publish_namespace'
import { Namespace } from './namespace'
import { NamespaceDone } from './namespace_done'
import { Setup } from './setup'
import { Fetch } from './fetch'
import { FetchOk } from './fetch_ok'
import { GoAway } from './goaway'
import { Subscribe } from './subscribe'
import { PublishDone } from './publish_done'
import { Publish } from './publish'
import { SubscribeOk } from './subscribe_ok'
import { RequestUpdate } from './request_update'
import { TrackStatus } from './track_status'
import { SubscribeNamespace } from './subscribe_namespace'
import { SubscribeTracks } from './subscribe_tracks'
import { NotEnoughBytesError } from '../error/error'
import { Tuple } from '../common'
import { AuthorizationToken } from '../parameter/common/authorization_token'
import { RequestOk } from './request_ok'
import { RequestError } from './request_error'

export type ControlMessage =
  | Publish
  | PublishBlocked
  | PublishDone
  | PublishNamespace
  | Namespace
  | NamespaceDone
  | Setup
  | Fetch
  | FetchOk
  | GoAway
  | Subscribe
  | RequestError
  | SubscribeOk
  | RequestUpdate
  | TrackStatus
  | SubscribeNamespace
  | SubscribeTracks
  | RequestOk

export namespace ControlMessage {
  export function deserialize(buf: FrozenByteBuffer): ControlMessage {
    const messageTypeRaw = buf.getVI()
    const messageType = ControlMessageType.tryFrom(messageTypeRaw)
    const payloadLength = buf.getU16()
    if (buf.remaining < payloadLength)
      throw new NotEnoughBytesError('ControlMessage.deserialize(payload_bytes)', payloadLength, buf.remaining)
    const payloadBytes = buf.getBytes(payloadLength)
    const payload = new FrozenByteBuffer(payloadBytes)
    switch (messageType) {
      case ControlMessageType.Publish:
        return Publish.parsePayload(payload)
      // PUBLISH_OK (0x1E) is a REQUEST_OK alias with no body of its own: PUBLISH is
      // answered by REQUEST_OK. Parse its body as REQUEST_OK.
      case ControlMessageType.PublishOk:
        return RequestOk.parsePayload(payload)
      case ControlMessageType.PublishDone:
        return PublishDone.parsePayload(payload)
      case ControlMessageType.PublishNamespace:
        return PublishNamespace.parsePayload(payload)
      case ControlMessageType.Namespace:
        return Namespace.parsePayload(payload)
      case ControlMessageType.NamespaceDone:
        return NamespaceDone.parsePayload(payload)
      case ControlMessageType.RequestOk:
        return RequestOk.parsePayload(payload)
      case ControlMessageType.RequestError:
        return RequestError.parsePayload(payload)
      case ControlMessageType.Fetch:
        return Fetch.parsePayload(payload)
      case ControlMessageType.FetchOk:
        return FetchOk.parsePayload(payload)
      case ControlMessageType.GoAway:
        return GoAway.parsePayload(payload)
      case ControlMessageType.Subscribe:
        return Subscribe.parsePayload(payload)
      case ControlMessageType.SubscribeOk:
        return SubscribeOk.parsePayload(payload)
      case ControlMessageType.RequestUpdate:
        return RequestUpdate.parsePayload(payload)
      case ControlMessageType.TrackStatus:
        return TrackStatus.parsePayload(payload)
      case ControlMessageType.SubscribeNamespace:
        return SubscribeNamespace.parsePayload(payload)
      case ControlMessageType.SubscribeTracks:
        return SubscribeTracks.parsePayload(payload)
      case ControlMessageType.Setup:
        return Setup.parsePayload(payload)
      case ControlMessageType.PublishBlocked:
        return PublishBlocked.parsePayload(payload)
      default:
        throw new Error(`Unknown or unhandled ControlMessageType: ${messageType}`)
    }
  }

  export function serialize(msg: ControlMessage): FrozenByteBuffer {
    return msg.serialize()
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest

  describe('ControlMessage', () => {
    describe('PublishNamespace', () => {
      function buildTestPublishNamespace(): PublishNamespace {
        return new PublishNamespace(12345n, Tuple.fromUtf8Path('god/dayyum'), [
          AuthorizationToken.newUseValue(0n, new TextEncoder().encode('test-token')),
        ])
      }

      test('should roundtrip PublishNamespace correctly', () => {
        const announce = buildTestPublishNamespace()
        const serialized = ControlMessage.serialize(announce)
        const deserialized = ControlMessage.deserialize(serialized)
        expect(deserialized).toEqual(announce)
      })

      test('should roundtrip PublishNamespace with excess trailing bytes', () => {
        const announce = buildTestPublishNamespace()
        const serialized = ControlMessage.serialize(announce).toUint8Array()
        const excessBytes = new Uint8Array(serialized.length + 3)
        excessBytes.set(serialized)
        excessBytes.set([9, 1, 1], serialized.length)

        const buf = new FrozenByteBuffer(excessBytes)
        const deserialized = ControlMessage.deserialize(buf)
        expect(deserialized).toEqual(announce)
        expect(buf.remaining).toBe(3)
        expect(Array.from(buf.getBytes(3))).toEqual([9, 1, 1])
      })

      test('should throw on partial PublishNamespace message', () => {
        const announce = buildTestPublishNamespace()
        const serialized = ControlMessage.serialize(announce).toUint8Array()
        const partial = serialized.slice(0, Math.floor(serialized.length / 2))
        const buf = new FrozenByteBuffer(partial)
        expect(() => ControlMessage.deserialize(buf)).toThrow(NotEnoughBytesError)
      })
    })

    describe('RequestOk', () => {
      test('PUBLISH_OK (0x1E) parses as RequestOk', () => {
        const requestOk = new RequestOk()
        const bytes = ControlMessage.serialize(requestOk).toUint8Array()
        // Table 5 keeps 0x1E but points it at §10.5, REQUEST_OK. Both codepoints are
        // one-byte varints, so retyping the message is a single-byte edit.
        bytes[0] = ControlMessageType.PublishOk
        const deserialized = ControlMessage.deserialize(new FrozenByteBuffer(bytes))
        expect(deserialized).toBeInstanceOf(RequestOk)
        expect(deserialized).toEqual(requestOk)
      })
    })

    describe('Fetch', () => {
      function buildTestFetch(): Fetch {
        const requestId = 161803n
        const joiningRequestId = 119n
        const joiningStart = 73n
        const type = FetchType.Relative
        const parameters = [AuthorizationToken.newUseValue(0n, new TextEncoder().encode('test-token'))]
        return new Fetch(requestId, { type, props: { joiningRequestId, joiningStart } }, parameters)
      }

      test('should roundtrip Fetch correctly', () => {
        const fetchMsg = buildTestFetch()
        const serialized = ControlMessage.serialize(fetchMsg)
        const deserialized = ControlMessage.deserialize(serialized)
        expect(deserialized).toEqual(fetchMsg)
      })

      test('should roundtrip Fetch with excess trailing bytes', () => {
        const fetchMsg = buildTestFetch()
        const serialized = ControlMessage.serialize(fetchMsg).toUint8Array()
        const excessBytes = new Uint8Array(serialized.length + 3)
        excessBytes.set(serialized)
        excessBytes.set([8, 2, 2], serialized.length)

        const buf = new FrozenByteBuffer(excessBytes)
        const deserialized = ControlMessage.deserialize(buf)
        expect(deserialized).toEqual(fetchMsg)
        expect(buf.remaining).toBe(3)
        expect(Array.from(buf.getBytes(3))).toEqual([8, 2, 2])
      })

      test('should throw on partial Fetch message', () => {
        const fetchMsg = buildTestFetch()
        const serialized = ControlMessage.serialize(fetchMsg).toUint8Array()
        const partial = serialized.slice(0, Math.floor(serialized.length / 2))
        const buf = new FrozenByteBuffer(partial)
        expect(() => ControlMessage.deserialize(buf)).toThrow(NotEnoughBytesError)
      })
    })
  })
}
