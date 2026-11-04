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
 * The optional interfaces an element can declare in `provides`, their keys, and the
 * catalog contract sources are written against (implemented in milestone 4).
 */

import type { FetchOptions, FetchResult, SubscribeOptions, SubscribeResult } from '@/client/types'
import type { Location } from '@/model/common/location'
import type { Tuple } from '@/model/common/tuple'
import type { MsfSource } from '@/player/msf_url'
import type { CatalogTrack, MediaTimelineTemplate } from '@/player/elements/catalog/types'
import type { SourceElementDefinition } from './define'

// Symbol.for, so the keys still match when the ESM and CJS builds are both loaded.
/** @public Key of {@link MediaSinkElement}. */
export const MEDIA_SINK = Symbol.for('moqtail.mediaSink')
/** @public Key of {@link MediaSourceElement}. */
export const MEDIA_SOURCE = Symbol.for('moqtail.mediaSource')

export type { CatalogTrack, MediaTimelineTemplate }

/** @public Maps each interface key to its interface. New interfaces add a line here. */
export interface ElementInterfaces {
  [MEDIA_SINK]: MediaSinkElement
  [MEDIA_SOURCE]: MediaSourceElement
}

/** @public */
export type InterfaceKey = keyof ElementInterfaces

/**
 * @public
 * Implemented by sinks that drive a display backend. Times are in seconds.
 * Dispatches `playing`, `pause`, `stalled`, `waiting`, and `timeupdate`.
 */
export interface MediaSinkElement extends EventTarget {
  readonly currentTime: number
  readonly buffered: TimeRanges
  readonly paused: boolean
  readonly seeking: boolean
  readonly ended: boolean
  readonly playbackRate: number
  setPlaybackRate(rate: number): void
  play(): Promise<void>
  pause(): Promise<void>
  seek(mediaTime: number): Promise<void>
}

/** @public Implemented by sources that deliver media and handle a switching set. */
export interface MediaSourceElement {
  getRenditions(): Rendition[]
  switch(renditionId: string): Promise<void>
  /** Sets which renditions ABR may choose from. */
  selectRenditions(renditionIds: string[]): Promise<void>
  /** Called by the catalog element when an update changes this source's switching set. */
  setTracks(tracks: readonly CatalogTrack[]): void
}

/** @public One track of a media source's switching set. */
export interface Rendition {
  /** Track name, with the namespace when it differs from the catalog's. */
  id: string
  src: string
  bitrate: number
  codec: string
  width?: number
  height?: number
  frameRate?: number
  /** Currently playing. */
  active: boolean
  /** Considered for ABR. */
  selected: boolean
}

/** @public What the catalog element offers the elements it loads. Updates arrive with the player's `catalog` event. */
export interface CatalogReader {
  /** The catalog's session URL, `moqt://authority/path[?query]` without a fragment. */
  readonly url: string
  /** The catalog track's namespace. */
  readonly namespace: Tuple
  /** The last good catalog. */
  readonly tracks: readonly CatalogTrack[]
  readonly isComplete: boolean
  /** Loads (shared) the media timeline element for the track, or wraps its template. Undefined if neither exists. */
  timeline(track: CatalogTrack): Promise<TimelineReader | undefined>
}

/** @public Translates between media time, wallclock time, and locations. Times in milliseconds. */
export interface TimelineReader {
  /** The group containing that time. */
  timeToLocation(mediaTimeMs: number): Location | undefined
  locationToTime(location: Location): number | undefined
  wallclockToLocation(wallclockMs: number): Location | undefined
  /** Media time, after `accessibleWindowTime`. */
  readonly range: { start: number; end: number } | undefined
}

/**
 * @public
 * Load options every media source takes. The catalog element, and wrapping sources such as
 * the SSTS one, pass them to whichever source the probe picks.
 */
export interface MediaSourceOptions {
  /** From the catalog source options: switch with `SWITCH_FROM`. */
  switchFrom?: boolean
  /** From the catalog URL. */
  ranges?: Pick<MsfSource, 'locationRanges' | 'mediaTimeRanges' | 'wallclockRanges'>
  /** How a parent adds to every SUBSCRIBE and FETCH the source sends. */
  requests?: RequestHooks
}

/**
 * @public
 * What an extension's switching set source is loaded with: the media source options, and the
 * definition the probe picked for the set's tracks.
 */
export interface SwitchingSetOptions extends MediaSourceOptions {
  child: SourceElementDefinition
}

/**
 * @public
 * Called for every SUBSCRIBE and FETCH a media source sends: start, retry, reconnect,
 * seek, switch, ranges. How a parent such as an SSTS source adds to the requests. The
 * hooks run last, after the source built its own options, and `subscribed` and `fetched`
 * get the options that were actually sent. They aren't called for rejected requests.
 */
export interface RequestHooks {
  subscribe?(track: CatalogTrack, options: SubscribeOptions): SubscribeOptions
  subscribed?(track: CatalogTrack, result: SubscribeResult, options: SubscribeOptions): void
  fetch?(track: CatalogTrack, options: FetchOptions): FetchOptions
  fetched?(track: CatalogTrack, result: FetchResult, options: FetchOptions): void
}
