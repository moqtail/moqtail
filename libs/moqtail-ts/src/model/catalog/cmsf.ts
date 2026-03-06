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

// Type definitions
export interface CMSFTrack {
  name: string
  packaging: 'cmaf' | 'chunk-per-object' | 'timeline'
  codec?: string
  role: 'video' | 'audio' | 'timeline'
  depends?: string[]
  mimeType?: string
  width?: number
  height?: number
  timescale?: number
  bitrate?: number
  initData?: string
}

export interface CMSF {
  version: number
  tracks: CMSFTrack[]
}

// Simple validation primitives
function isString(v: unknown): v is string {
  return typeof v === 'string'
}

function isNumber(v: unknown): v is number {
  return typeof v === 'number'
}

function isStringArray(v: unknown): v is string[] {
  return Array.isArray(v) && v.every((item) => isString(item))
}

function isValidPackaging(v: unknown): v is 'cmaf' | 'chunk-per-object' | 'timeline' {
  return v === 'cmaf' || v === 'chunk-per-object' || v === 'timeline'
}

function isValidRole(v: unknown): v is 'video' | 'audio' | 'timeline' {
  return v === 'video' || v === 'audio' || v === 'timeline'
}

function isValidBase64(str: string): boolean {
  try {
    return btoa(atob(str)) === str
  } catch {
    return false
  }
}

function validateTrack(track: unknown): CMSFTrack {
  if (typeof track !== 'object' || track === null) {
    throw new Error('Track must be an object')
  }

  const t = track as Record<string, unknown>

  // Validate required fields
  if (!isString(t['name'])) {
    throw new Error('Track.name must be a string')
  }
  if (!isValidPackaging(t['packaging'])) {
    throw new Error("Track.packaging must be 'cmaf', 'chunk-per-object', or 'timeline'")
  }
  if (!isValidRole(t['role'])) {
    throw new Error("Track.role must be 'video', 'audio', or 'timeline'")
  }

  const name = t['name'] as string
  const packaging = t['packaging'] as 'cmaf' | 'chunk-per-object' | 'timeline'
  const role = t['role'] as 'video' | 'audio' | 'timeline'

  // Validate optional fields
  let codec: string | undefined
  if (t['codec'] !== undefined) {
    if (!isString(t['codec'])) {
      throw new Error('Track.codec must be a string')
    }
    codec = t['codec']
  }

  let depends: string[] | undefined
  if (t['depends'] !== undefined) {
    if (!isStringArray(t['depends'])) {
      throw new Error('Track.depends must be an array of strings')
    }
    depends = t['depends']
  }

  let mimeType: string | undefined
  if (t['mimeType'] !== undefined) {
    if (!isString(t['mimeType'])) {
      throw new Error('Track.mimeType must be a string')
    }
    mimeType = t['mimeType']
  }

  let width: number | undefined
  if (t['width'] !== undefined) {
    if (!isNumber(t['width'])) {
      throw new Error('Track.width must be a number')
    }
    width = t['width']
  }

  let height: number | undefined
  if (t['height'] !== undefined) {
    if (!isNumber(t['height'])) {
      throw new Error('Track.height must be a number')
    }
    height = t['height']
  }

  let timescale: number | undefined
  if (t['timescale'] !== undefined) {
    if (!isNumber(t['timescale'])) {
      throw new Error('Track.timescale must be a number')
    }
    timescale = t['timescale']
  }

  let bitrate: number | undefined
  if (t['bitrate'] !== undefined) {
    if (!isNumber(t['bitrate'])) {
      throw new Error('Track.bitrate must be a number')
    }
    bitrate = t['bitrate']
  }

  let initData: string | undefined
  if (t['initData'] !== undefined) {
    if (!isString(t['initData'])) {
      throw new Error('Track.initData must be a string')
    }
    if (!isValidBase64(t['initData'])) {
      throw new Error('Track.initData must be a valid base64 string')
    }
    initData = t['initData']
  }

  // Custom validation rules
  if (packaging === 'cmaf' && (!codec || codec.length === 0)) {
    throw new Error("codec is required when packaging is 'cmaf'")
  }

  if (packaging === 'timeline' && mimeType !== 'text/csv') {
    throw new Error("mimeType must be 'text/csv' when packaging is 'timeline'")
  }

  if (packaging === 'timeline' && (!depends || depends.length === 0)) {
    throw new Error("At least one dependee must be present when packaging is 'timeline'")
  }

  const result: CMSFTrack = {
    name,
    packaging,
    role,
  }

  if (codec !== undefined) result.codec = codec
  if (depends !== undefined) result.depends = depends
  if (mimeType !== undefined) result.mimeType = mimeType
  if (width !== undefined) result.width = width
  if (height !== undefined) result.height = height
  if (timescale !== undefined) result.timescale = timescale
  if (bitrate !== undefined) result.bitrate = bitrate
  if (initData !== undefined) result.initData = initData

  return result
}

function validateCMSF(obj: unknown): CMSF {
  if (typeof obj !== 'object' || obj === null) {
    throw new Error('CMSF must be an object')
  }

  const o = obj as Record<string, unknown>

  if (!isNumber(o['version'])) {
    throw new Error('CMSF.version must be a number')
  }

  if (!Array.isArray(o['tracks'])) {
    throw new Error('CMSF.tracks must be an array')
  }

  const version = o['version'] as number
  const tracksArray = o['tracks'] as unknown[]
  const tracks = tracksArray.map(validateTrack)

  return {
    version,
    tracks,
  }
}

class CMSFCatalog {
  #catalog: CMSF

  private constructor(catalog: CMSF) {
    this.#catalog = catalog
  }

  static from(payload: ArrayBufferLike) {
    const decoder = new TextDecoder()
    const json = decoder.decode(payload)
    const catalog = validateCMSF(JSON.parse(json))
    return new CMSFCatalog(catalog)
  }

  getByTrackName(trackName: string) {
    return this.#catalog.tracks.find((track) => track.name === this.extractTrackName(trackName))
  }

  getTracks(role?: CMSF['tracks'][number]['role']) {
    if (role) return this.#catalog.tracks.filter((track) => track.role === role)
    return this.#catalog.tracks
  }

  getVideo(index = 0) {
    const videos = this.#catalog.tracks.filter((track) => track.role === 'video')
    if (index < 0 || index >= videos.length) return undefined
    return videos[index]
  }

  getAudio(index = 0) {
    const audios = this.#catalog.tracks.filter((track) => track.role === 'audio')
    if (index < 0 || index >= audios.length) return undefined
    return audios[index]
  }

  getTimeline(index = 0) {
    const timelines = this.#catalog.tracks.filter((track) => track.role === 'timeline')
    if (index < 0 || index >= timelines.length) return undefined
    return timelines[index]
  }

  getCodecString(trackName: string) {
    const track = this.#catalog.tracks.find((track) => track.name === this.extractTrackName(trackName))
    if (!track) return undefined
    if (!track.codec) throw new Error(`Track ${trackName} does not have a codec`)
    return track.codec
  }

  getRole(trackName: string) {
    const track = this.#catalog.tracks.find((track) => track.name === this.extractTrackName(trackName))
    if (!track) return undefined
    return track.role
  }

  getPackaging(trackName: string) {
    const track = this.#catalog.tracks.find((track) => track.name === this.extractTrackName(trackName))
    if (!track) return undefined
    return track.packaging
  }

  isCMAF(trackName: string) {
    const packaging = this.getPackaging(trackName)
    return packaging === 'cmaf' || packaging === 'chunk-per-object'
  }

  getTimescale(trackName: string) {
    const track = this.#catalog.tracks.find((track) => track.name === this.extractTrackName(trackName))
    return track?.timescale
  }

  getInitData(trackName: string) {
    const track = this.#catalog.tracks.find((track) => track.name === this.extractTrackName(trackName))
    if (!track) return undefined
    if (!track.initData) throw new Error(`Track ${trackName} does not have initData`)
    return new Uint8Array(
      atob(track.initData)
        .split('')
        .map((c) => c.charCodeAt(0)),
    )
  }

  private extractTrackName(input: string) {
    const parts = input.split('/')
    return parts[parts.length - 1]
  }
}

export { CMSFCatalog }
