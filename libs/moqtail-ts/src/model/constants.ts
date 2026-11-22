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
 * Protocol string array exchanged in wt-available-protocols header
 */
export const SUPPORTED_VERSIONS = ['moqt-22']

/**
 * @public
 * Which fields of a LOCATION_FILTER follow its type, and how they are read.
 */
export enum LocationFilterType {
  /** No filter: every Object passes. */
  None = 0x0,
  /** Starts StartGroup groups before the Next Group. */
  RelativeStart = 0x1,
  AbsoluteStart = 0x2,
  /** Ends with the last Object of the End Group. */
  AbsoluteStartGroupEnd = 0x3,
  /** Ends with End Object of the End Group. */
  AbsoluteRange = 0x4,
  /** Starts at the Object after the Largest Object. */
  NextObject = 0x5,
}

/**
 * @public
 * Switch modes for subscription switching requests.
 */
export enum SwitchMode {
  Hard = 0x0,
  Soft = 0x1,
}

/**
 * @public
 * Group ordering options for object delivery.
 */
export enum GroupOrder {
  Original = 0x0,
  Ascending = 0x1,
  Descending = 0x2,
}

/**
 * Status codes for track status responses.
 */
export enum TrackStatusCode {
  InProgress = 0x00,
  DoesNotExist = 0x01,
  NotYetBegun = 0x02,
  Finished = 0x03,
  RelayUnavailable = 0x04,
}

/**
 * @public
 * Status codes for PublishDone control messages.
 */
export enum PublishDoneStatusCode {
  InternalError = 0x0,
  Unauthorized = 0x1,
  TrackEnded = 0x2,
  GoingAway = 0x4,
  TooFarBehind = 0x5,
  Expired = 0x6,
  UpdateFailed = 0x8,
  ExcessiveLoad = 0x9,
  MalformedTrack = 0x12,
}

/**
 * @public
 * Unified error codes for REQUEST_ERROR control messages.
 *
 * A separate registry from `StreamResetCode`, which disagrees with it on
 * the same names: `GOING_AWAY` is `0x6` here but `0x4` as a stream reset code.
 */
export enum RequestErrorCode {
  InternalError = 0x0,
  Unauthorized = 0x1,
  Timeout = 0x2,
  NotSupported = 0x3,
  MalformedAuthToken = 0x4,
  ExpiredAuthToken = 0x5,
  GoingAway = 0x6,
  ExcessiveLoad = 0x9,
  DoesNotExist = 0x10,
  InvalidRange = 0x11,
  MalformedTrack = 0x12,
  Uninterested = 0x20,
  PrefixOverlap = 0x30,
  NamespaceTooLarge = 0x31,
  InvalidSwitch = 0x32,
  UnsupportedExtension = 0x33,
  Redirect = 0x34,
  ConflictingFilters = 0x35,
  InvalidFilter = 0x36,
}

/** @public */
export enum TokenAliasType {
  Delete = 0x0,
  Register = 0x1,
  UseAlias = 0x2,
  UseValue = 0x3,
}
