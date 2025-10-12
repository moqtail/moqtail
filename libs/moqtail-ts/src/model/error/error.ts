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
 * Base class for all MOQtail errors.
 */
export class MOQtailError extends Error {
  constructor(cause?: string) {
    super(cause)
    this.name = this.constructor.name
  }
}

/**
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
 * Key/value formatting error.
 */
export class KeyValueFormattingError extends MOQtailError {
  constructor(public context: string) {
    super(`[${context}] key value formatting error`)
  }
}

/**
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
 * Operation timed out.
 */
export class TimeoutError extends MOQtailError {
  constructor(public context: string) {
    super(`Timeout: [${context}]`)
  }
}

/**
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
