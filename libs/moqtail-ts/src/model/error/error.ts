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

/**
 * @public
 * Base class for all MOQtail errors.
 */
export class MOQtailError extends Error {
  constructor(cause?: string) {
    super(cause)
    this.name = this.constructor.name
  }
}

/**
 * @public
 * Not enough bytes in buffer to satisfy a read.
 */
export class NotEnoughBytesError extends MOQtailError {
  constructor(
    public context: string,
    public needed: number,
    public available: number,
  ) {
    super(`[${context}] not enough bytes: needed ${needed}, available ${available}`)
  }
}

/**
 * @public
 * Cannot cast between two incompatible types.
 */
export class CastingError extends MOQtailError {
  constructor(
    public context: string,
    public fromType: string,
    public toType: string,
    public details: string,
  ) {
    super(`[${context}] cannot cast from ${fromType} to ${toType}, [${details}]`)
  }
}

/**
 * @public
 * Value too large to encode as varint.
 */
export class VarIntOverflowError extends MOQtailError {
  constructor(
    public context: string,
    public value: number,
  ) {
    super(`[${context}] value ${value} too large to encode as varint`)
  }
}

/**
 * @public
 * Length exceeds maximum allowed by protocol.
 */
export class LengthExceedsMaxError extends MOQtailError {
  constructor(
    public context: string,
    public max: number,
    public len: number,
  ) {
    super(`[${context}] length ${len} exceeds maximum of ${max}, protocol violation`)
  }
}

/**
 * @public
 * Key/value formatting error.
 */
export class KeyValueFormattingError extends MOQtailError {
  constructor(public context: string) {
    super(`[${context}] key value formatting error`)
  }
}

/**
 * @public
 * Invalid discriminant or type tag.
 */
export class InvalidTypeError extends MOQtailError {
  constructor(
    public context: string,
    public details: string,
  ) {
    super(`Invalid type: [${context}], [${details}]`)
  }
}

/**
 * @public
 * Invalid enum discriminant / value encountered when mapping to a protocol enum.
 */
export class InvalidEnumValue extends MOQtailError {
  constructor(
    public context: string,
    public value: bigint | number,
  ) {
    super(`Invalid enum value: [${context}], [${value}]`)
  }
}

/**
 * @public
 * Invalid UTF-8 encountered in a string or bytes field.
 */
export class InvalidUTF8Error extends MOQtailError {
  constructor(
    public context: string,
    public details: string,
  ) {
    super(`Invalid UTF8: [${context}], [${details}]`)
  }
}

/**
 * @public
 * Generic protocol violation.
 */
export class ProtocolViolationError extends MOQtailError {
  constructor(
    public context: string,
    public details: string,
  ) {
    super(`Protocol violation: [${context}], [${details}]`)
  }
}

/**
 * @public
 * Track naming error.
 */
export class TrackNameError extends MOQtailError {
  constructor(
    public context: string,
    public details: string,
  ) {
    super(`Track naming error: [${context}], [${details}]`)
  }
}

/**
 * @public
 * Request ID error.
 */
export class RequestIdError extends MOQtailError {
  constructor(
    public context: string,
    public details: string,
  ) {
    super(`Request ID error: [${context}], [${details}]`)
  }
}

/**
 * @public
 * Operation timed out.
 */
export class TimeoutError extends MOQtailError {
  constructor(public context: string) {
    super(`Timeout: [${context}]`)
  }
}

/**
 * @public
 * Internal error.
 */
export class InternalError extends MOQtailError {
  constructor(
    public context: string,
    public details: string,
  ) {
    super(`Internal error: [${context}], [${details}]`)
  }
}

/**
 * @public
 * Connection terminated.
 */
export class TerminationError extends MOQtailError {
  constructor(
    public context: string,
    public terminationCode: number,
  ) {
    super(`Connection terminated with code ${terminationCode}: ${context}`)
  }
}
