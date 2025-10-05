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
import { ControlMessageType, controlMessageTypeFromBigInt, FetchType, GroupOrder } from './constant'
import { PublishNamespace } from './publish_namespace'
import { PublishNamespaceCancel } from './publish_namespace_cancel'
import { PublishNamespaceError } from './publish_namespace_error'
import { PublishNamespaceOk } from './publish_namespace_ok'
import { ClientSetup } from './client_setup'
import { Fetch } from './fetch'
import { FetchCancel } from './fetch_cancel'
import { FetchError } from './fetch_error'
import { FetchOk } from './fetch_ok'
import { GoAway } from './goaway'
import { MaxRequestId } from './max_request_id'
import { ServerSetup } from './server_setup'
import { Subscribe } from './subscribe'
import { SubscribeDone } from './subscribe_done'
import { SubscribeError } from './subscribe_error'
import { SubscribeOk } from './subscribe_ok'
import { SubscribeUpdate } from './subscribe_update'
import { RequestsBlocked } from './requests_blocked'
import { TrackStatus } from './track_status'
import { PublishNamespaceDone } from './publish_namespace_done'
import { Unsubscribe } from './unsubscribe'
import { SubscribeNamespace } from './subscribe_namespace'
import { SubscribeNamespaceOk } from './subscribe_namespace_ok'
import { SubscribeNamespaceError } from './subscribe_namespace_error'
import { UnsubscribeNamespace } from './unsubscribe_namespace'
import { NotEnoughBytesError } from '../error/error'
import { Tuple, KeyValuePair } from '../common'
import { TrackStatusOk } from './track_status_ok'
import { TrackStatusError } from './track_status_error'

export type ControlMessage =
  | PublishNamespace
  | PublishNamespaceCancel
  | PublishNamespaceError
  | PublishNamespaceOk
  | ClientSetup
  | Fetch
  | FetchCancel
  | FetchError
  | FetchOk
  | GoAway
  | MaxRequestId
  | ServerSetup
  | Subscribe
  | SubscribeDone
  | SubscribeError
  | SubscribeOk
  | SubscribeUpdate
  | RequestsBlocked
  | TrackStatus
  | TrackStatusOk
  | TrackStatusError
  | PublishNamespaceDone
  | Unsubscribe
  | SubscribeNamespace
  | SubscribeNamespaceOk
  | SubscribeNamespaceError
  | UnsubscribeNamespace

export namespace ControlMessage {
  export function deserialize(buf: FrozenByteBuffer): ControlMessage {
    const messageTypeRaw = buf.getVI()
    const messageType = controlMessageTypeFromBigInt(messageTypeRaw)
    const payloadLength = buf.getU16()
    if (buf.remaining < payloadLength)
      throw new NotEnoughBytesError('ControlMessage.deserialize(payload_bytes)', payloadLength, buf.remaining)
    const payloadBytes = buf.getBytes(payloadLength)
    const payload = new FrozenByteBuffer(payloadBytes)
    switch (messageType) {
      case ControlMessageType.PublishNamespace:
        return PublishNamespace.parsePayload(payload)
      case ControlMessageType.PublishNamespaceCancel:
        return PublishNamespaceCancel.parsePayload(payload)
      case ControlMessageType.PublishNamespaceError:
        return PublishNamespaceError.parsePayload(payload)
      case ControlMessageType.PublishNamespaceOk:
        return PublishNamespaceOk.parsePayload(payload)
      case ControlMessageType.ClientSetup:
        return ClientSetup.parsePayload(payload)
      case ControlMessageType.Fetch:
        return Fetch.parsePayload(payload)
      case ControlMessageType.FetchCancel:
        return FetchCancel.parsePayload(payload)
      case ControlMessageType.FetchError:
        return FetchError.parsePayload(payload)
      case ControlMessageType.FetchOk:
        return FetchOk.parsePayload(payload)
      case ControlMessageType.GoAway:
        return GoAway.parsePayload(payload)
      case ControlMessageType.MaxRequestId:
        return MaxRequestId.parsePayload(payload)
      case ControlMessageType.ServerSetup:
        return ServerSetup.parsePayload(payload)
      case ControlMessageType.Subscribe:
        return Subscribe.parsePayload(payload)
      case ControlMessageType.SubscribeDone:
        return SubscribeDone.parsePayload(payload)
      case ControlMessageType.SubscribeError:
        return SubscribeError.parsePayload(payload)
      case ControlMessageType.SubscribeOk:
        return SubscribeOk.parsePayload(payload)
      case ControlMessageType.SubscribeUpdate:
        return SubscribeUpdate.parsePayload(payload)
      case ControlMessageType.RequestsBlocked:
        return RequestsBlocked.parsePayload(payload)
      case ControlMessageType.TrackStatus:
        return TrackStatus.parsePayload(payload)
      case ControlMessageType.TrackStatus:
        return TrackStatus.parsePayload(payload)
      case ControlMessageType.PublishNamespaceDone:
        return PublishNamespaceDone.parsePayload(payload)
      case ControlMessageType.Unsubscribe:
        return Unsubscribe.parsePayload(payload)
      case ControlMessageType.SubscribeNamespace:
        return SubscribeNamespace.parsePayload(payload)
      case ControlMessageType.SubscribeNamespaceOk:
        return SubscribeNamespaceOk.parsePayload(payload)
      case ControlMessageType.SubscribeNamespaceError:
        return SubscribeNamespaceError.parsePayload(payload)
      case ControlMessageType.UnsubscribeNamespace:
        return UnsubscribeNamespace.parsePayload(payload)
      default:
        // This case should ideally be unreachable if controlMessageTypeFromBigInt is exhaustive
        // or throws on unknown types. If it can return a type not in the switch,
        // an error here is appropriate.
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
          KeyValuePair.tryNewVarInt(0, 10),
          KeyValuePair.tryNewBytes(1, new TextEncoder().encode('wololoo')),
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
        expect(buf.remaining).toBe(3) // Check that excess bytes are still there
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

    describe('Fetch', () => {
      function buildTestFetch(): Fetch {
        const requestId = 161803n
        const subscriberPriority = 15
        const groupOrder = GroupOrder.Descending
        const joiningRequestId = 119n
        const joiningStart = 73n
        const type = FetchType.Relative
        const parameters = [
          KeyValuePair.tryNewVarInt(4444, 12321n),
          KeyValuePair.tryNewBytes(1, new TextEncoder().encode('fetch me ok')),
        ]
        return new Fetch(
          requestId,
          subscriberPriority,
          groupOrder,
          { type, props: { joiningRequestId, joiningStart } },
          parameters,
        )
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
