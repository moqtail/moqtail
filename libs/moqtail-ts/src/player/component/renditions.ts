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
 * The player's media sources as media-tracks tracks and renditions (media-ui-extensions
 * proposal 0011). As media-tracks and Video.js read it, `selected` is a manual pick and none
 * selected is automatic switching, when `player.isAbrAvailable()`. Otherwise the active
 * rendition shows selected. Only the public player API is used.
 */

import type {
  AudioRendition,
  AudioRenditionList,
  AudioTrack,
  VideoRendition,
  VideoRenditionList,
  VideoTrack,
} from 'media-tracks'
import {
  CapabilityCode,
  type MediaSourceElement,
  type MOQtailPlayer,
  type Rendition,
  type SourceElement,
} from '@/player'
import { logger } from '@/util'

/** The element's media-tracks API. */
export interface TrackHost {
  readonly videoRenditions: VideoRenditionList
  readonly audioRenditions: AudioRenditionList
  addVideoTrack(kind: string, label?: string, language?: string): VideoTrack
  addAudioTrack(kind: string, label?: string, language?: string): AudioTrack
  removeVideoTrack(track: VideoTrack): void
  removeAudioTrack(track: AudioTrack): void
}

type Source = SourceElement & MediaSourceElement
type ListedRendition = (VideoRendition | AudioRendition) & { active?: boolean }

/** One media source's track and its renditions by id. */
type Listed = ({ role: 'video'; track: VideoTrack } | { role: 'audio'; track: AudioTrack }) & {
  renditions: Map<string, ListedRendition>
}

/** Keeps a host's tracks and renditions in step with one player. `dispose()` removes them. */
export class Renditions {
  readonly #host: TrackHost
  readonly #player: MOQtailPlayer
  readonly #listed = new Map<Source, Listed>()
  readonly #abort = new AbortController()

  constructor(host: TrackHost, player: MOQtailPlayer) {
    this.#host = host
    this.#player = player
    for (const list of [host.videoRenditions, host.audioRenditions]) {
      list.addEventListener('change', () => this.#apply(), { signal: this.#abort.signal })
    }
    // Registered after the player's own listeners, so its ABR controller is up to date here
    player.on('catalog', this.sync)
    player.on('renditionchange', this.sync)
    player.on('streamtypechange', this.sync)
    player.on('selectionchange', this.sync)
  }

  /** Adds, updates, and removes tracks and renditions from the player's media sources. */
  readonly sync = (): void => {
    if (this.#abort.signal.aborted) return
    const sources = this.#player.mediaSources
    for (const [source, listed] of this.#listed) if (!sources.includes(source)) this.#remove(source, listed)
    for (const source of sources) {
      const role = roleOf(source)
      if (role !== 'video' && role !== 'audio') continue
      const listed = this.#listed.get(source) ?? this.#add(source, role)
      this.#update(source, listed)
    }
  }

  dispose(): void {
    this.#abort.abort()
    this.#player.off('catalog', this.sync)
    this.#player.off('renditionchange', this.sync)
    this.#player.off('streamtypechange', this.sync)
    this.#player.off('selectionchange', this.sync)
    for (const [source, listed] of this.#listed) this.#remove(source, listed)
  }

  #add(source: Source, role: 'video' | 'audio'): Listed {
    let listed: Listed
    if (role === 'video') {
      const track = this.#host.addVideoTrack('main')
      // A video track list has one selected track, and only its renditions are listed. The list
      // may also hold the inner video's native tracks, so only our own count.
      track.selected = ![...this.#listed.values()].some((l) => l.role === 'video')
      listed = { role, track, renditions: new Map() }
    } else {
      const track = this.#host.addAudioTrack('main')
      track.enabled = true
      listed = { role, track, renditions: new Map() }
    }
    this.#listed.set(source, listed)
    return listed
  }

  #remove(source: Source, listed: Listed): void {
    this.#listed.delete(source)
    if (listed.role === 'video') this.#host.removeVideoTrack(listed.track)
    else this.#host.removeAudioTrack(listed.track)
  }

  #update(source: Source, listed: Listed): void {
    const renditions = source.getRenditions()
    const ids = new Set(renditions.map((r) => r.id))
    const shown = shownId(this.#player.isAbrAvailable(source), renditions)
    let activeChanged = false
    for (const [id, rendition] of listed.renditions) {
      if (ids.has(id)) continue
      listed.renditions.delete(id)
      listed.track.removeRendition(rendition as VideoRendition & AudioRendition)
    }
    for (const r of renditions) {
      let rendition = listed.renditions.get(r.id)
      if (!rendition) {
        rendition = listed.track.addRendition(r.src)
        listed.renditions.set(r.id, rendition)
      }
      if (rendition.active !== r.active) activeChanged = true
      Object.assign(rendition, { id: r.id, src: r.src, codec: r.codec, bitrate: r.bitrate, active: r.active })
      if (listed.role === 'video')
        Object.assign(rendition, { width: r.width, height: r.height, frameRate: r.frameRate })
      rendition.selected = r.id === shown
    }
    // Video.js refreshes its "Auto (…)" label on it; media-tracks has no event for `active`
    if (!activeChanged) return
    const list = listed.role === 'video' ? this.#host.videoRenditions : this.#host.audioRenditions
    list.dispatchEvent(new Event('activechange'))
  }

  /** Hands each source the renditions now selected (all of them for none), when they differ from its own. */
  #apply(): void {
    for (const [source, listed] of this.#listed) {
      let ids = [...listed.renditions].filter(([, r]) => r.selected).map(([id]) => id)
      const renditions = source.getRenditions()
      const abr = this.#player.isAbrAvailable(source)
      if (ids.length === 0 && !abr) {
        // Nothing switches automatically: show the active rendition again
        this.#update(source, listed)
        continue
      }
      const current = renditions.filter((r) => r.selected)
      const active = renditions.find((r) => r.active)
      if (ids.length === 0) ids = [...listed.renditions.keys()]
      // Without ABR the active rendition shows selected; handing it back would pin the source
      else if (!abr && ids.length === 1 && active && ids[0] === active.id && current.includes(active)) continue
      if (ids.length === current.length && current.every((r) => ids.includes(r.id))) continue
      // A plain source switches to the closest selected bitrate, so one rendition is a manual switch
      source.selectRenditions(ids).catch((error) => logger.warn('MOQtailVideo', 'selectRenditions failed', error))
    }
  }
}

/** The rendition to show selected: a single pick, else none under ABR (automatic), else the active one. */
function shownId(abr: boolean, renditions: Rendition[]): string | undefined {
  const selected = renditions.filter((r) => r.selected)
  if (selected.length === 1) return selected[0]!.id
  return abr ? undefined : renditions.find((r) => r.active)?.id
}

function roleOf(source: Source): string | undefined {
  const pad = source.pads.find((p) => p.direction === 'src')
  return pad?.capabilities.find((c) => c.code === CapabilityCode.ROLE)?.value
}
