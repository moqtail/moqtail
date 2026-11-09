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
 * proposal 0011). `selected` is the set ABR may choose from, so every rendition starts
 * selected, and `selectedIndex` is the single selected one, or -1 for all of them.
 */

import type {
  AudioRendition,
  AudioRenditionList,
  AudioTrack,
  VideoRendition,
  VideoRenditionList,
  VideoTrack,
} from 'media-tracks'
import type { SourceElement } from '@/player/element/base'
import { CapabilityCode } from '@/player/element/define'
import type { MediaSourceElement, Rendition } from '@/player/element/interfaces'
import type { MOQtailPlayer } from '@/player/player'
import { logger } from '@/util/logger'

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
      singleSelectedIndex(list)
      list.addEventListener('change', () => this.#apply(), { signal: this.#abort.signal })
    }
    player.on('catalog', this.sync)
    player.on('renditionchange', this.sync)
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
      this.#update(listed, source.getRenditions())
    }
  }

  dispose(): void {
    this.#abort.abort()
    this.#player.off('catalog', this.sync)
    this.#player.off('renditionchange', this.sync)
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

  #update(listed: Listed, renditions: Rendition[]): void {
    const ids = new Set(renditions.map((r) => r.id))
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
      Object.assign(rendition, { id: r.id, src: r.src, codec: r.codec, bitrate: r.bitrate, active: r.active })
      if (listed.role === 'video')
        Object.assign(rendition, { width: r.width, height: r.height, frameRate: r.frameRate })
      rendition.selected = r.selected
    }
  }

  /** Hands each source the renditions now selected, when they differ from its own. */
  #apply(): void {
    for (const [source, listed] of this.#listed) {
      const ids = [...listed.renditions].filter(([, r]) => r.selected).map(([id]) => id)
      const current = source.getRenditions().filter((r) => r.selected)
      if (ids.length === current.length && current.every((r) => ids.includes(r.id))) continue
      // A plain source switches to the closest selected bitrate, so one rendition is a manual switch
      source.selectRenditions(ids).catch((error) => logger.warn('MOQtailVideo', 'selectRenditions failed', error))
    }
  }
}

/** Replaces the list's `selectedIndex`: the single selected rendition, or -1; setting -1 selects all. */
function singleSelectedIndex(list: VideoRenditionList | AudioRenditionList): void {
  if (Object.hasOwn(list, 'selectedIndex')) return
  Object.defineProperty(list, 'selectedIndex', {
    get(): number {
      const all = [...list]
      const selected = all.filter((r) => r.selected)
      return selected.length === 1 ? all.indexOf(selected[0]!) : -1
    },
    set(index: number) {
      for (const [i, rendition] of [...list].entries()) rendition.selected = index === -1 || i === index
    },
  })
}

function roleOf(source: Source): string | undefined {
  const pad = source.pads.find((p) => p.direction === 'src')
  return pad?.capabilities.find((c) => c.code === CapabilityCode.ROLE)?.value
}
