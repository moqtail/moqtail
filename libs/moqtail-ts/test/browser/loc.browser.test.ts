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

/** LOC tracks played with `player.sink(canvas)`, the decoders loaded by the chain. */

import { beforeAll, expect, onTestFinished, test, vi } from 'vitest'
import { userEvent } from 'vitest/browser'
import { TransformElement, type BaseElement } from '@/player/element/base'
import type { ElementContext } from '@/player/element/context'
import { CapabilityCode, defineElement, definitionOf } from '@/player/element/define'
import type { CatalogReader } from '@/player/element/interfaces'
import type { Pad, PadItem } from '@/player/element/pad'
import { roleOf } from '@/player/element/sink/media_sink'
import type { PlayerError } from '@/player/error'
import { MediaSink } from '@/player/element/sink/media_sink'
import { MOQtailPlayer } from '@/player/player'
import { RENDER_QUEUE_LIMIT } from '@/player/elements/playout/engine'
import { liveBuffer } from '@/player/plugins/buffer'
import { frameFilter, type Draw, type FrameFilter } from './harness/engines'
import { testsrc } from './harness/sources'
import { playerOptions, preferWorker } from './harness/thread'
import { warmUp } from './harness/warm'

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

warmUp(testsrc)

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
  // A main-thread transform: in a worker run, decoding happens there and the frames are bridged
  const player = new MOQtailPlayer(playerOptions()).source(testsrc.uri()).transform(tapDefinition).sink(canvas)
  const { errors, catalog } = watch(player, canvas)

  await player.start()
  await until(() => player.state === 'playing', 'playing')
  const sink = player.mediaSink!
  const drawn = spyDraws(() => sink.currentTime)
  const [start, video, audio] = [sink.currentTime, tap!.frames.get('video') ?? 0, tap!.frames.get('audio') ?? 0]
  await until(() => sink.currentTime >= start + 2, 'two seconds of playback', 15_000)
  expect(tap!.frames.get('video')).toBeGreaterThan(video)
  expect(tap!.frames.get('audio')).toBeGreaterThan(audio)

  const tracks = catalog()!.tracks.filter((t) => t.role === 'video')
  expect([canvas.width, canvas.height]).toEqual([tracks[0]!.width, tracks[0]!.height])
  // Sink ← tap ← webcodecs ← loc, for both pads
  const upstream = (element: BaseElement) =>
    element.pads.filter((p) => p.direction === 'sink').map((p) => p.peers[0]!.element)
  expect(upstream(sink as unknown as BaseElement)).toEqual([tap, tap])
  const decoders = upstream(tap!)
  expect(decoders.map((d) => definitionOf(d)!.name)).toEqual(['webcodecs', 'webcodecs'])
  expect(decoders.flatMap(upstream).map((s) => definitionOf(s)!.name)).toEqual(['loc', 'loc'])

  // Both tracks are stamped in epoch µs without a Timescale: the shared clock case
  expect(drawn.length).toBeGreaterThan(0)
  const framerate = tracks[0]!.framerate!
  for (const { timestamp, currentTime } of drawn) {
    expect(Math.abs(timestamp - currentTime)).toBeLessThan(1 / framerate + 1 / 60 + 0.01)
  }

  expect(errors).toEqual([])
})

test('liveBuffer settles at rate 1 with the forward buffer webcodecs holds encoded, past what the render queue holds', async () => {
  const canvas = document.createElement('canvas')
  document.body.append(canvas)
  const player = new MOQtailPlayer({ ...playerOptions(), plugins: [liveBuffer()] }).source(testsrc.uri()).sink(canvas)
  const { errors, catalog } = watch(player, canvas)
  await player.start()
  await until(() => player.state === 'playing', 'playing')
  const sink = player.mediaSink!
  const forward = () => (sink.buffered.length ? sink.buffered.end(0) - sink.currentTime : 0)
  // At rate 1 the forward buffer is within liveBuffer's tolerance of a goal no lower than 0.5 s
  await until(() => sink.playbackRate === 1 && forward() > 0.4, 'rate 1 at the goal', 15_000)
  const framerate = catalog()!.tracks.find((t) => t.role === 'video')!.framerate!
  expect(RENDER_QUEUE_LIMIT / framerate).toBeLessThan(0.4)
  expect(errors).toEqual([])
})

test.skipIf(!preferWorker)(
  'LOC with nothing in between links webcodecs to playout in the worker, and plays',
  async () => {
    const canvas = document.createElement('canvas')
    document.body.append(canvas)
    const received = vi.spyOn(MediaSink.prototype, 'receive')
    onTestFinished(() => received.mockRestore())
    const player = new MOQtailPlayer(playerOptions()).source(testsrc.uri()).sink(canvas)
    const { errors } = watch(player, canvas)
    await player.start()
    await until(() => player.state === 'playing', 'playing')
    const sink = player.mediaSink!
    const start = sink.currentTime
    await until(() => sink.currentTime >= start + 2, 'two seconds of playback', 15_000)
    // The canvas went to the worker, and no frame came through the main thread
    expect(() => canvas.getContext('2d')).toThrow(expect.objectContaining({ name: 'InvalidStateError' }))
    expect(received.mock.calls.filter(([, item]) => item.type === 'frame')).toEqual([])
    const decoders = (sink as unknown as BaseElement).pads.map((p) => p.peers[0]!)
    expect(decoders.map((pad) => definitionOf(pad.element)!.name)).toEqual(['webcodecs', 'webcodecs'])
    expect(decoders.every((pad) => pad.capabilities.some((c) => c.code === CapabilityCode.THREAD))).toBe(true)
    expect(errors).toEqual([])
  },
)

test('a custom engine between webcodecs and playout links both its pads in the worker, or runs locally, and plays in sync', async () => {
  let filter: FrameFilter | undefined
  const canvas = document.createElement('canvas')
  document.body.append(canvas)
  const definition = {
    ...frameFilter,
    init: (context: ElementContext) => frameFilter.init(context).then((f) => (filter = f)),
  }
  const player = new MOQtailPlayer(playerOptions()).source(testsrc.uri()).transform(definition).sink(canvas)
  const { errors, catalog } = watch(player, canvas)
  await player.start()
  await until(() => player.state === 'playing', 'playing')
  const sink = player.mediaSink!
  const drawLog = preferWorker ? filter!.drawLog() : undefined
  const samples = sampleClock(() => sink.currentTime)
  const drawn = preferWorker ? undefined : spyDraws(() => sink.currentTime)
  const [start, frames] = [sink.currentTime, filter!.frames]
  await until(() => sink.currentTime >= start + 2, 'two seconds of playback', 15_000)
  expect(filter!.frames).toBeGreaterThan(frames)
  // Linked, nothing reaches the element on the main thread
  expect(filter!.received > 0).toBe(!preferWorker)

  const framerate = catalog()!.tracks.find((t) => t.role === 'video')!.framerate!
  if (drawn) {
    expect(drawn.length).toBeGreaterThan(0)
    for (const { timestamp, currentTime } of drawn) {
      expect(Math.abs(timestamp - currentTime)).toBeLessThan(1 / framerate + 1 / 60 + 0.01)
    }
  } else {
    const draws = await drawLog!.draws()
    expect(draws.length).toBeGreaterThan(0)
    // The clock reaches the main thread once per animation frame, so it reads up to one late
    const offset = median(samples.map((s) => s.currentTime - s.at))
    for (const { timestamp, at } of draws.filter((d: Draw) => d.at >= samples[0]!.at)) {
      expect(Math.abs(timestamp - (at + offset))).toBeLessThan(1 / framerate + 2 / 60 + 0.01)
    }
  }
  expect(errors).toEqual([])
})

test('advertised video VideoDecoder cannot decode, neither avc1 nor avc3, fails instead of playing audio alone', async () => {
  // testsrc's LOC video decodes as avc1 and avc3, so both are made to fail
  const isConfigSupported = VideoDecoder.isConfigSupported.bind(VideoDecoder)
  const spy = vi
    .spyOn(VideoDecoder, 'isConfigSupported')
    .mockImplementation(async (config) =>
      config.codec.startsWith('avc') ? { supported: false, config } : isConfigSupported(config),
    )
  onTestFinished(() => spy.mockRestore())
  const canvas = document.createElement('canvas')
  document.body.append(canvas)
  const player = new MOQtailPlayer(playerOptions()).source(testsrc.uri()).sink(canvas)
  const { errors } = watch(player, canvas)

  await expect(player.start()).rejects.toMatchObject({ code: 'UNSUPPORTED_SOURCE', element: 'catalog' })
  expect(errors).toMatchObject([{ code: 'UNSUPPORTED_SOURCE', message: expect.stringContaining('No video track') }])
})

test('a second player on one canvas plays on it, or fails at once if the first drew from the worker', async () => {
  const canvas = document.createElement('canvas')
  document.body.append(canvas)
  const first = new MOQtailPlayer(playerOptions()).source(testsrc.uri()).sink(canvas)
  await first.start()
  await until(() => first.state === 'playing', 'the first playing')
  await first.destroy()

  const second = new MOQtailPlayer(playerOptions()).source(testsrc.uri()).sink(canvas)
  const { errors } = watch(second, canvas)
  void second.start().catch(() => {})
  if (preferWorker) {
    await until(() => errors.length > 0, 'an error')
    expect(errors).toMatchObject([{ code: 'UNSUPPORTED_SOURCE', element: 'playout' }])
  } else {
    await until(() => second.state === 'playing', 'the second playing')
    expect(errors).toEqual([])
  }
})

function watch(player: MOQtailPlayer, canvas: HTMLCanvasElement) {
  const errors: PlayerError[] = []
  player.on('error', (error) => errors.push(error))
  let catalog: CatalogReader | undefined
  player.on('catalog', (reader) => (catalog = reader))
  onTestFinished(async () => {
    await player.destroy()
    canvas.remove()
  })
  return { errors, catalog: () => catalog }
}

/** The clock and the epoch time, in seconds, every 10 ms until the test ends. */
function sampleClock(clock: () => number): { currentTime: number; at: number }[] {
  const samples: { currentTime: number; at: number }[] = []
  const timer = setInterval(
    () => samples.push({ currentTime: clock(), at: (performance.timeOrigin + performance.now()) / 1000 }),
    10,
  )
  onTestFinished(() => clearInterval(timer))
  return samples
}

function median(values: number[]): number {
  const sorted = [...values].sort((a, b) => a - b)
  return sorted[Math.floor(sorted.length / 2)]!
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
