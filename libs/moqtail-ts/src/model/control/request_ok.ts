/**
 * Copyright 2026 The MOQtail Authors
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 * http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

import { BaseByteBuffer, ByteBuffer, FrozenByteBuffer } from '../common/byte_buffer'

import { ControlMessageType, FilterType, GroupOrder } from './constant'
import { CastingError, LengthExceedsMaxError, ProtocolViolationError } from '../error/error'
import {
  MessageParameter,
  deserializeMessageParameterKvps,
  serializeMessageParameterKvps,
} from '../parameter/message_parameter'
import { TrackProperty, ObjectDeliveryTimeoutProperty, MaxCacheDurationProperty } from '../property/track_property'

import { Location } from '../common/location'
import { Forward } from '../parameter/message/forward'
import { SubscriberPriority } from '../parameter/message/subscriber_priority'
import { GroupOrderParam } from '../parameter/message/group_order_param'
import { SubscriptionFilter } from '../parameter/message/subscription_filter'
import { ObjectDeliveryTimeout } from '../parameter/message/object_delivery_timeout'

/**
 * @public
 * REQUEST_OK (0x7) answers PUBLISH, REQUEST_UPDATE, TRACK_STATUS, SUBSCRIBE_NAMESPACE,
 * SUBSCRIBE_TRACKS and PUBLISH_NAMESPACE. There is one wire type; the per-request-type
 * names (PUBLISH_OK, TRACK_STATUS_OK, ...) are shorthands for logging, not distinct
 * messages. SUBSCRIBE_OK (0x4) and FETCH_OK (0x18) do keep bodies of their own.
 *
 * It carries no Request ID: the request stream it arrives on identifies the request
 * (§10.1).
 */
export class RequestOk {
  public readonly parameters: MessageParameter[]
  /**
   * Draft-18 Track Properties, which the trailing bytes of the payload carry with no
   * count of their own. Populated only in TRACK_STATUS_OK; empty for every other request
   * type — see {@link RequestOk.validateTrackProperties}.
   */
  public readonly trackProperties: TrackProperty[]

  constructor(parameters: MessageParameter[] = [], trackProperties: TrackProperty[] = []) {
    this.parameters = parameters
    this.trackProperties = trackProperties
  }

  getType(): ControlMessageType {
    return ControlMessageType.RequestOk
  }

  /**
   * Track Properties may only be non-empty when this REQUEST_OK answers a TRACK_STATUS
   * request (§10.5). A receiver that sees them in any other REQUEST_OK — PUBLISH_OK,
   * REQUEST_UPDATE_OK, SUBSCRIBE_NAMESPACE_OK or PUBLISH_NAMESPACE_OK — MUST close the
   * session with a PROTOCOL_VIOLATION.
   *
   * @throws :{@link ProtocolViolationError} If Track Properties are present and this
   * REQUEST_OK does not answer a TRACK_STATUS.
   */
  validateTrackProperties(answersTrackStatus: boolean): void {
    if (!answersTrackStatus && this.trackProperties.length > 0) {
      throw new ProtocolViolationError(
        'RequestOk.validateTrackProperties',
        'Track Properties present in a non-TRACK_STATUS REQUEST_OK',
      )
    }
  }

  serialize(): FrozenByteBuffer {
    const buf = new ByteBuffer()
    buf.putVI(ControlMessageType.RequestOk)

    const payload = new ByteBuffer()
    payload.putVI(BigInt(this.parameters.length))

    payload.putBytes(serializeMessageParameterKvps(this.parameters.map((p) => p.toKeyValuePair())).toUint8Array())

    // Track Properties span the remaining message length (no explicit count).
    TrackProperty.serializeInto(this.trackProperties, payload)

    const payloadBytes = payload.toUint8Array()
    if (payloadBytes.length > 0xffff) {
      throw new LengthExceedsMaxError('RequestOk::serialize(payloadBytes)', 0xffff, payloadBytes.length)
    }

    buf.putU16(payloadBytes.length)
    buf.putBytes(payloadBytes)

    return buf.freeze()
  }

  /**
   * Parses a REQUEST_OK body. `buf` must hold exactly the payload: whatever follows the
   * parameters is read as Track Properties, so trailing bytes from a longer buffer would
   * be parsed as properties.
   */
  static parsePayload(buf: BaseByteBuffer): RequestOk {
    const numParamsBig = buf.getVI()
    const numParams = Number(numParamsBig)
    if (BigInt(numParams) !== numParamsBig) {
      throw new CastingError('RequestOk.parsePayload numParams', 'bigint', 'number', `${numParamsBig}`)
    }

    const parameters: MessageParameter[] = []
    for (const kvp of deserializeMessageParameterKvps(buf, numParams)) {
      const param = MessageParameter.fromKeyValuePair(kvp)
      if (param !== undefined) parameters.push(param)
    }

    const trackProperties = TrackProperty.deserializeAll(buf)

    return new RequestOk(parameters, trackProperties)
  }
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest

  /** Strips the type and length header and hands back exactly the payload. */
  function payloadOf(msg: RequestOk): FrozenByteBuffer {
    const frozen = msg.serialize()
    expect(frozen.getVI()).toBe(BigInt(ControlMessageType.RequestOk))
    const msgLength = frozen.getU16()
    expect(msgLength).toBe(frozen.remaining)
    return new FrozenByteBuffer(frozen.getBytes(msgLength))
  }

  describe('RequestOk', () => {
    // The request stream identifies the request (§10.1), so an empty REQUEST_OK is a
    // one-byte payload: the parameter count, and nothing before it.
    test('roundtrip', () => {
      const msg = new RequestOk()
      const payload = payloadOf(msg)
      expect(payload.remaining).toBe(1)
      const deserialized = RequestOk.parsePayload(payload)
      expect(deserialized.parameters.length).toBe(0)
      expect(deserialized.trackProperties.length).toBe(0)
      expect(payload.remaining).toBe(0)
    })

    // PUBLISH is answered by REQUEST_OK, so the parameters a PUBLISH_OK used to carry
    // travel here now.
    test('roundtrip with parameters', () => {
      const parameters: MessageParameter[] = [
        new Forward(true),
        new SubscriberPriority(100),
        new GroupOrderParam(GroupOrder.Ascending),
        new SubscriptionFilter(FilterType.LatestObject, undefined, undefined),
      ]
      const msg = new RequestOk(parameters)
      const payload = payloadOf(msg)
      const deserialized = RequestOk.parsePayload(payload)
      expect(deserialized.parameters.length).toBe(parameters.length)
      expect(payload.remaining).toBe(0)
    })

    test('roundtrip with AbsoluteRangeFill subscription filter', () => {
      const parameters: MessageParameter[] = [
        new SubscriptionFilter(FilterType.AbsoluteRangeFill, new Location(5n, 10n), 20n),
        new ObjectDeliveryTimeout(5000n),
      ]
      const msg = new RequestOk(parameters)
      const payload = payloadOf(msg)
      const deserialized = RequestOk.parsePayload(payload)
      expect(deserialized.parameters.length).toBe(2)
      expect(payload.remaining).toBe(0)
    })

    test('roundtrip with track properties (TRACK_STATUS_OK)', () => {
      const msg = new RequestOk(
        [new ObjectDeliveryTimeout(100n)],
        [new ObjectDeliveryTimeoutProperty(5000n), new MaxCacheDurationProperty(60000n)],
      )
      const payload = payloadOf(msg)
      const deserialized = RequestOk.parsePayload(payload)
      expect(deserialized.parameters.length).toBe(1)
      expect(deserialized.trackProperties.length).toBe(2)
      expect(deserialized.trackProperties[0]).toBeInstanceOf(ObjectDeliveryTimeoutProperty)
      expect(deserialized.trackProperties[1]).toBeInstanceOf(MaxCacheDurationProperty)
      expect(payload.remaining).toBe(0)
    })

    test('track properties are only valid in a TRACK_STATUS_OK', () => {
      const withProperties = new RequestOk([], [new MaxCacheDurationProperty(1n)])
      // Answering a TRACK_STATUS: allowed.
      expect(() => withProperties.validateTrackProperties(true)).not.toThrow()
      // Answering PUBLISH, REQUEST_UPDATE, SUBSCRIBE_NAMESPACE or PUBLISH_NAMESPACE: a
      // protocol violation.
      expect(() => withProperties.validateTrackProperties(false)).toThrow(ProtocolViolationError)

      // No properties is always fine.
      const empty = new RequestOk()
      expect(() => empty.validateTrackProperties(true)).not.toThrow()
      expect(() => empty.validateTrackProperties(false)).not.toThrow()
    })

    test('excess roundtrip', () => {
      const msg = new RequestOk([new ObjectDeliveryTimeout(5000n)])
      const serialized = msg.serialize().toUint8Array()
      const excess = new Uint8Array([9, 1, 1])
      const buf = new ByteBuffer()
      buf.putBytes(serialized)
      buf.putBytes(excess)
      const frozen = buf.freeze()
      const msgType = frozen.getVI()
      expect(msgType).toBe(BigInt(ControlMessageType.RequestOk))
      const msgLength = frozen.getU16()
      expect(msgLength).toBe(frozen.remaining - 3)
      const payload = new FrozenByteBuffer(frozen.getBytes(msgLength))
      const deserialized = RequestOk.parsePayload(payload)
      expect(deserialized.parameters.length).toBe(1)
      expect(payload.remaining).toBe(0)
      expect(frozen.remaining).toBe(3)
      expect(Array.from(frozen.getBytes(3))).toEqual([9, 1, 1])
    })

    test('partial message', () => {
      const msg = new RequestOk([new ObjectDeliveryTimeout(5000n)])
      const serialized = msg.serialize().toUint8Array()
      const upper = Math.floor(serialized.length / 2)
      const partial = serialized.slice(0, upper)
      const frozen = new FrozenByteBuffer(partial)
      expect(() => {
        frozen.getVI()
        frozen.getU16()
        RequestOk.parsePayload(frozen)
      }).toThrow()
    })
  })
}
