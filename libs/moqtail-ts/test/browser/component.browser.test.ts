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
 * `<moqtail-video>`: loading, its own properties, the media-ui-extensions IDL, and HTMLMediaElement
 * forwarding. Playback itself is the player's, covered by the other browser tests.
 */

import { Attributes } from 'custom-media-element'
import { afterEach, beforeAll } from 'vitest'
import { abr, DefaultAbrController, ssts, SstsSource, type MOQtailPlayerOptions } from '@/player'
import { LiveBufferController } from '@/player/plugins/buffer/live'
import { MOQtailVideoElement, defineMOQtailVideo } from '@/player/component'
import { ch07, ch07_vod, describe, expect, test, warmUp } from './harness/test'

const TIMEOUT = 20_000
const elements: MOQtailVideoElement[] = []
// Created before the element is defined, with its config already set
const early = document.createElement('moqtail-video') as MOQtailVideoElement
const earlyConfig = { catalogTimeoutMs: 5000 }
early.config = earlyConfig

/** A muted `<moqtail-video>` in the page, with `attributes` set before it's connected. */
function create(attributes: Record<string, string> = {}, config?: MOQtailPlayerOptions): MOQtailVideoElement {
  const el = document.createElement('moqtail-video') as MOQtailVideoElement
  el.config = config
  el.muted = true
  for (const [name, value] of Object.entries(attributes)) el.setAttribute(name, value)
  document.body.append(el)
  elements.push(el)
  return el
}

function next(target: EventTarget, type: string): Promise<Event> {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error(`Timed out waiting for ${type}`)), TIMEOUT)
    target.addEventListener(type, (event) => (clearTimeout(timer), resolve(event)), { once: true })
  })
}

async function until(condition: () => boolean, what: string): Promise<void> {
  const deadline = performance.now() + TIMEOUT
  while (!condition()) {
    if (performance.now() > deadline) throw new Error(`Timed out waiting for ${what}`)
    await new Promise((resolve) => setTimeout(resolve, 50))
  }
}

afterEach(() => {
  for (const el of elements.splice(0)) {
    el.removeAttribute('src')
    el.remove()
  }
})

describe('component', () => {
  warmUp(ch07)
  beforeAll(() => defineMOQtailVideo())

  test('defineMOQtailVideo registers once, and upgrades an element whose config was set before', () => {
    // `customElements.define` throws for a taken name: a second call must not
    expect(() => defineMOQtailVideo()).not.toThrow()
    customElements.upgrade(early)
    expect(customElements.get('moqtail-video')).toBe(MOQtailVideoElement)
    expect(early).toBeInstanceOf(MOQtailVideoElement)
    expect(early.config).toBe(earlyConfig)
  })

  test('live: autoplay, streamType, duration, seekable, liveEdgeStart, targetLiveWindow, tracks, and renditions', async () => {
    const config = { catalogTimeoutMs: 15_000 }
    const el = create({ autoplay: '' }, config)
    const streamType = next(el, 'streamtypechange')
    el.src = ch07.uri()
    await next(el, 'playing')
    await streamType
    expect([el.player?.options.catalogTimeoutMs, el.streamType, el.duration, el.targetLiveWindow]).toEqual([
      15_000,
      'live',
      Infinity,
      Infinity,
    ])
    expect(el.player?.bufferController).toBeInstanceOf(LiveBufferController) // the defaults, attached
    expect(el.player?.abrController).toBeInstanceOf(DefaultAbrController)

    await until(() => !Number.isNaN(el.liveEdgeStart), 'liveEdgeStart')
    expect(el.seekable.length).toBe(1)
    expect(el.liveEdgeStart).toBeLessThanOrEqual(el.seekable.end(0))

    expect([el.videoTracks.length, el.audioTracks.length, el.textTracks.length]).toEqual([1, 1, 0])
    const renditions = [...el.videoRenditions]
    const expected = el.player!.mediaSources.flatMap((s) => s.getRenditions()).filter((r) => r.width !== undefined)
    expect(
      renditions.map((r) => [r.id, r.src, r.width, r.height, r.codec, r.frameRate, r.bitrate, r.selected]),
    ).toEqual(expected.map((r) => [r.id, r.src, r.width, r.height, r.codec, r.frameRate, r.bitrate, false]))
    expect(renditions.length).toBeGreaterThan(1)
    for (const r of renditions) expect(r.src).toMatch(/#msf:[^&]+$/) // no fragment parameters
    expect([...el.audioRenditions].length).toBe(1)

    // None selected is automatic; a single selection is a manual switch, which detaches the ABR controller
    expect(el.videoRenditions.selectedIndex).toBe(-1)
    const target = renditions.findIndex((r) => r.height !== el.videoHeight)
    const changed = next(el.videoRenditions, 'change')
    el.videoRenditions.selectedIndex = target
    await changed
    expect(el.videoRenditions.selectedIndex).toBe(target)
    expect(renditions.map((r) => r.selected)).toEqual(renditions.map((_, i) => i === target))
    expect(el.player?.abrController).toBeUndefined()
    await until(() => el.videoHeight === renditions[target]!.height, `${renditions[target]!.id} on screen`)
    el.videoRenditions.selectedIndex = -1
    const sources = el.player!.mediaSources
    await until(() => sources.every((s) => s.getRenditions().every((r) => r.selected)), 'every rendition selected')
    expect(el.videoRenditions.selectedIndex).toBe(-1)
    expect(el.player?.abrController).toBeInstanceOf(DefaultAbrController)
    // A selection made through the player API shows in the list too
    const video = sources.find((s) => s.getRenditions().some((r) => r.width !== undefined))!
    await video.selectRenditions([renditions[0]!.id!])
    expect([el.videoRenditions.selectedIndex, el.player?.abrController]).toEqual([0, undefined])
  })

  test('HTMLMediaElement: every member exists, and forwarded properties read as the inner video', async () => {
    const el = create({ preload: 'auto' })
    el.src = ch07.uri()
    await el.play()
    expect(el.paused).toBe(false)
    el.pause()
    expect(el.paused).toBe(true)
    const time = el.buffered.end(0) - 1
    const seeked = next(el, 'seeked')
    el.currentTime = time
    await seeked
    // The sink may jump a gap after the seek
    expect(Math.abs(el.nativeEl.currentTime - time)).toBeLessThan(1)

    const members = new Set<string>()
    for (const proto of [HTMLMediaElement.prototype, HTMLVideoElement.prototype]) {
      for (const name of Object.getOwnPropertyNames(proto)) if (name !== 'constructor') members.add(name)
    }
    // Attribute-backed members read from the element's attributes, which it forwards
    const own = new Set<string>(['src', 'error', ...Attributes])
    for (const name of members) {
      expect(name in el, name).toBe(true)
      const value = Reflect.get(el.nativeEl, name)
      if (own.has(name.toLowerCase()) || typeof value === 'function' || typeof value === 'object') continue
      expect(Reflect.get(el, name), name).toBe(value)
    }
    expect(el.readyState).toBeGreaterThanOrEqual(HTMLMediaElement.HAVE_CURRENT_DATA)
    expect(el.buffered.length).toBeGreaterThan(0)
  })

  test('src, load(), preload, and switch-from: each load is a new player, started as asked', async () => {
    const el = create({ preload: 'none', 'switch-from': '' })
    el.setAttribute('src', ch07.uri())
    const idle = el.player!
    await new Promise((resolve) => setTimeout(resolve))
    expect(idle.state).toBe('idle')
    await el.play()
    expect(el.player).toBe(idle)
    expect(idle.mediaSources.every((s) => (s as { options?: { switchFrom?: boolean } }).options?.switchFrom)).toBe(true)

    el.preload = 'metadata'
    el.load()
    const reloaded = el.player!
    expect(reloaded).not.toBe(idle)
    await until(() => reloaded.state === 'ready', 'ready without play()')
    expect(el.paused).toBe(true)

    el.src = ch07.uri('x=1')
    expect(el.player).not.toBe(reloaded)
    el.removeAttribute('src')
    expect(el.player).toBeUndefined()
    await until(() => reloaded.state === 'destroyed', 'the old player destroyed')
  })

  test('SSTS through config.plugins: selectedIndex changes the switching set, without switch(), and -1 is automatic', async () => {
    const el = create({}, { plugins: [ssts(), abr()] })
    el.src = ch07.uri()
    await el.play()
    const source = el.player!.mediaSources.find((s) => s instanceof SstsSource)!
    expect(source).toBeDefined()
    expect(el.player!.abrController).toBeUndefined() // SSTS switches itself
    await until(() => el.videoRenditions.length > 1, 'video renditions')
    const warnings: string[] = []
    el.player!.on('warning', (w) => warnings.push(w.code))
    expect(el.videoRenditions.selectedIndex).toBe(-1)
    el.videoRenditions.selectedIndex = 0
    await next(el.videoRenditions, 'change')
    await until(() => source.getRenditions().filter((r) => r.selected).length === 1, 'one selected rendition')
    expect(source.getRenditions().find((r) => r.selected)!.id).toBe(el.videoRenditions[0]!.id)
    el.videoRenditions.selectedIndex = -1
    await until(() => source.getRenditions().every((r) => r.selected), 'every rendition selected')
    expect(el.videoRenditions.selectedIndex).toBe(-1)
    expect([el.error, warnings]).toEqual([null, []])
  })

  test('on-demand: streamType, finite duration, NaN liveEdgeStart and targetLiveWindow, and ended', async () => {
    const el = create()
    el.src = ch07_vod.uri()
    await el.play()
    await until(() => el.streamType === 'on-demand', 'on-demand')
    expect(Number.isFinite(el.duration)).toBe(true)
    expect([el.liveEdgeStart, el.targetLiveWindow]).toEqual([NaN, NaN])
    // Nothing switches automatically on demand: the active rendition shows selected, and -1 is undone
    expect(el.player!.abrController).toBeUndefined()
    const video = el.player!.mediaSources.flatMap((s) => s.getRenditions()).filter((r) => r.width !== undefined)
    const active = video.findIndex((r) => r.active)
    expect(el.videoRenditions.selectedIndex).toBe(active)
    el.videoRenditions.selectedIndex = -1
    await until(() => el.videoRenditions.selectedIndex === active, 'the active rendition shown again')
    const ended = next(el, 'ended')
    el.currentTime = el.duration - 1
    await ended
    expect(el.ended).toBe(true)
  })

  test('error: a URI no source accepts is a MEDIA_ERR_SRC_NOT_SUPPORTED error event', async () => {
    const el = create()
    const error = next(el, 'error')
    el.src = 'moqt://relay.moqtail.dev/app#not-msf'
    await error
    expect(el.error?.code).toBe(MediaError.MEDIA_ERR_SRC_NOT_SUPPORTED)
    expect(el.player).toBeUndefined()
  })
})
