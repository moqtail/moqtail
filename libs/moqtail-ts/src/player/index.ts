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

export { parseMsfUrl, formatMsfUrl, type MsfSource, type LocationRange, type TimeRange } from './msf_url'
export { PlayerError, type PlayerErrorCode, type PlayerErrorCategory } from './error'
export { MOQtailPlayer, type MOQtailPlayerOptions, type PlayerEvents } from './player'
export type { PlayerState } from './state'
export type { Catalog, CatalogJson, InitData, MediaTimelineTemplate } from './elements/catalog/types'
export type { Catalog as CatalogElement } from './elements/catalog/element'
export type { TimelineEvent, TimelineRecord } from './element/event_timeline'
export { defineExtension, type ExtensionDefinition } from './extension'
export { MSE } from './elements/mse'
export { SSTS, SstsSource } from './extensions/ssts'
export type { MSEOptions } from './elements/mse/sink'
export {
  defineElement,
  hasInterface,
  CapabilityCode,
  DEFAULT_PRIORITY,
  type Capability,
  type Capabilities,
  type CapabilityBundles,
  type Provided,
  type Identifiers,
  type ElementDefinition,
  type SourceElementDefinition,
  type TransformElementDefinition,
  type SinkElementDefinition,
} from './element/define'
export { BaseElement, SourceElement, TransformElement, SinkElement, type ElementEvents } from './element/base'
export {
  Pad,
  CONNECTION_CAPACITY,
  type PadItem,
  type PadOutput,
  type PadProperties,
  type PadEvent,
} from './element/pad'
export type { ElementContext, ElementLoadRequest, PlayerHandle } from './element/context'
export {
  MEDIA_SINK,
  MEDIA_SOURCE,
  type ElementInterfaces,
  type InterfaceKey,
  type MediaSinkElement,
  type MediaSourceElement,
  type Rendition,
  type CatalogTrack,
  type CatalogReader,
  type TimelineReader,
  type RequestHooks,
  type MediaSourceOptions,
  type SwitchingSetOptions,
} from './element/interfaces'
