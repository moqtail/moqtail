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

/** moqlivemock's LOC catalog played with `player.sink(canvas)`, the decoders loaded by the chain. */

import { beforeAll, expect, onTestFinished, test, vi } from 'vitest'
import { userEvent } from 'vitest/browser'
import { TransformElement, type BaseElement } from '@/player/element/base'
import type { ElementContext } from '@/player/element/context'
import { CapabilityCode, defineElement, definitionOf } from '@/player/element/define'
import type { CatalogReader, CatalogTrack } from '@/player/element/interfaces'
import type { Pad, PadItem } from '@/player/element/pad'
import { roleOf } from '@/player/element/sink/media_sink'
import type { PlayerError } from '@/player/error'
import { MOQtailPlayer } from '@/player/player'
import { mlm } from './harness/sources'

/** Passes `raw` pads on, counting the frames of each role. */
class Tap extends TransformElement {
  readonly frames = new Map<string | undefined, number>()
  readonly #outputs = new Map<Pad, Pad>()

  constructor(context: ElementContext) {
    super(context)
    this.on('padadded', (pad) => {
      if (pad.direction === 'sink') this.#outputs.set(pad, this.createPad('src', pad.capabilities))
    })
  }

  negotiate(pad: Pad): boolean {
    return this.matchesCapabilities(pad)
  }

  async receive(pad: Pad, item: PadItem): Promise<void> {
    if (item.type === 'frame') this.frames.set(roleOf(pad), (this.frames.get(roleOf(pad)) ?? 0) + 1)
    await this.#outputs.get(pad)?.write(item)
  }
}

beforeAll(async () => {
  // The page's user activation lets the AudioContext run
  const button = document.createElement('button')
  button.textContent = 'activate'
  document.body.append(button)
  await userEvent.click(button)
  button.remove()
})

test('LOC plays on a canvas, in sync, through the webcodecs transforms the chain loads in front of a declared raw transform', async () => {
  let tap: Tap | undefined
  const raw = { direction: 'in' as const, code: CapabilityCode.PACKAGING, value: 'raw' }
  const tapDefinition = defineElement({
    type: 'transform',
    name: 'tap',
    capabilities: [[raw], [{ ...raw, direction: 'out' }]],
    init: async (context) => {
      tap = new Tap(context)
      return tap
    },
  })
  const canvas = document.createElement('canvas')
  document.body.append(canvas)
  const player = new MOQtailPlayer().source(mlm.uri()).transform(tapDefinition).sink(canvas)
  const errors: PlayerError[] = []
  player.on('error', (error) => errors.push(error))
  let catalog: CatalogReader | undefined
  player.on('catalog', (reader) => (catalog = reader))
  onTestFinished(async () => {
    await player.destroy()
    canvas.remove()
  })

  await player.start()
  await until(() => player.state === 'playing', 'playing')
  const sink = player.mediaSink!
  const drawn = spyDraws(() => sink.currentTime)
  const [start, video, audio] = [sink.currentTime, tap!.frames.get('video') ?? 0, tap!.frames.get('audio') ?? 0]
  await until(() => sink.currentTime >= start + 2, 'two seconds of playback', 15_000)
  expect(tap!.frames.get('video')).toBeGreaterThan(video)
  expect(tap!.frames.get('audio')).toBeGreaterThan(audio)

  const tracks = catalog!.tracks.filter((t) => t.role === 'video')
  expect([canvas.width, canvas.height]).toEqual([tracks[0]!.width, tracks[0]!.height])
  // Sink ← tap ← webcodecs ← loc, for both pads
  const upstream = (element: BaseElement) =>
    element.pads.filter((p) => p.direction === 'sink').map((p) => p.peers[0]!.element)
  expect(upstream(sink as unknown as BaseElement)).toEqual([tap, tap])
  const decoders = upstream(tap!)
  expect(decoders.map((d) => definitionOf(d)!.name)).toEqual(['webcodecs', 'webcodecs'])
  expect(decoders.flatMap(upstream).map((s) => definitionOf(s)!.name)).toEqual(['loc', 'loc'])

  // moqlivemock stamps both tracks in epoch µs without a Timescale: the shared clock case
  expect(drawn.length).toBeGreaterThan(0)
  const framerate = tracks[0]!.framerate!
  for (const { timestamp, currentTime } of drawn) {
    expect(Math.abs(timestamp - currentTime)).toBeLessThan(1 / framerate + 1 / 60 + 0.01)
  }

  // The renditions VideoDecoder can't decode, such as HEVC in headless Chromium, leave the set
  const decodable = await decodableOf(tracks)
  const renditions = player.mediaSources.flatMap((s) => s.getRenditions()).filter((r) => r.width !== undefined)
  expect(renditions.map((r) => r.id)).toEqual(decodable.map((t) => t.name))
  expect(renditions.find((r) => r.active)?.codec).toBe(decodable[0]!.codec)
  expect(decodable.length).toBeLessThan(tracks.length)
  expect(errors).toEqual([])
})

/** The tracks `VideoDecoder` decodes, in catalog order. */
async function decodableOf(tracks: CatalogTrack[]): Promise<CatalogTrack[]> {
  const support = await Promise.all(tracks.map((t) => VideoDecoder.isConfigSupported({ codec: t.codec! })))
  return tracks.filter((_, i) => support[i]!.supported)
}

/** Each frame drawn: its media time, and the clock then, both in seconds. */
function spyDraws(clock: () => number): { timestamp: number; currentTime: number }[] {
  const drawn: { timestamp: number; currentTime: number }[] = []
  const draw = CanvasRenderingContext2D.prototype.drawImage
  const spy = vi.spyOn(CanvasRenderingContext2D.prototype, 'drawImage')
  spy.mockImplementation(function (this: CanvasRenderingContext2D, ...args: unknown[]) {
    drawn.push({ timestamp: (args[0] as VideoFrame).timestamp / 1_000_000, currentTime: clock() })
    return draw.apply(this, args as Parameters<typeof draw>)
  })
  onTestFinished(() => spy.mockRestore())
  return drawn
}

async function until(condition: () => boolean, what: string, timeout = 10_000): Promise<void> {
  await vi.waitUntil(condition, { timeout, interval: 10 }).catch(() => {
    throw new Error(`Timed out waiting for ${what}`)
  })
}
