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

/** The `playout` sink with media made in the page, timestamped in epoch µs as LOC content is. Needs no relay. */

import { beforeAll, describe, expect, onTestFinished, test, vi } from 'vitest'
import { userEvent } from 'vitest/browser'
import { Location } from '@/model/common/location'
import { Emitter } from '@/util/emitter'
import { CapabilityCode, type Capabilities } from '@/player/element/define'
import type { Pad, PadProperties } from '@/player/element/pad'
import { Playout } from '@/player/elements/playout'
import { RENDER_QUEUE_LIMIT } from '@/player/elements/playout/engine'
import type { PlayoutSink } from '@/player/elements/playout/sink'
import type { MOQtailPlayer } from '@/player/player'
import { LIVE_BUFFER_DEFAULTS, LiveBufferController } from '@/player/plugins/buffer/live'
import { flush, testChain, testSource, type TestSource } from '@/player/testing/elements'
import { chainWorker } from './harness/thread'

const FPS = 30
const SAMPLE_RATE = 48_000
const WIDTH = 64
const HEIGHT = 48
/** Seconds of the audio chunks `open` writes. */
const CHUNK = 0.02
/** Every frame the tests made, to check the sink closed them. */
const frames: VideoFrame[] = []

interface Session {
  sink: PlayoutSink
  canvas: HTMLCanvasElement
  failures: Error[]
  video: Pad | undefined
  audio: Pad | undefined
  /** Epoch seconds of the first media. */
  base: number
}

beforeAll(async () => {
  // The page's user activation lets the AudioContext run
  const button = document.createElement('button')
  button.textContent = 'activate'
  document.body.append(button)
  await userEvent.click(button)
  button.remove()
})

describe('playout', () => {
  test('becomes ready with a picture of the first frame', async () => {
    const s = await open({ write: (s) => writeFrames(s, [0], [0], 'rgb(0, 128, 255)') })
    expect([s.canvas.width, s.canvas.height]).toEqual([WIDTH, HEIGHT])
    const [r, g, b] = s.canvas.getContext('2d')!.getImageData(WIDTH / 2, HEIGHT / 2, 1, 1).data
    expect([r, g, b]).toEqual([0, 128, 255])
  })

  test('video follows the audio clock and timeupdate fires', async () => {
    const s = await open()
    const feed = live(s, { video: false })
    const timeupdates = count(s.sink, 'timeupdate')
    await s.sink.play()
    await until(() => s.sink.currentTime > s.base + 0.2, 'the audio clock runs')
    const drawn = spyDraws(s.sink)
    // Written well ahead of the audio, so only the clock decides when each is drawn
    const from = s.sink.currentTime - s.base + 0.5
    await writeFrames(s, range(from, from + 1, 1 / FPS), [])
    await until(() => drawn.length >= FPS * 0.8, 'the frames are drawn')
    expect(drawn[0]!.timestamp).toBeGreaterThanOrEqual(s.base + from - 1e-6)
    expectInSync(drawn)
    expect(timeupdates()).toBeGreaterThan(2)
    feed.stop()
  })

  test('liveBuffer catches up to its target, and jumps forward from far behind it', async () => {
    // Audio only: the render queue holds less video than a jump needs to be seen
    const s = await open({ video: false })
    const feed = live(s, { lead: 1.2 })
    const player = Object.assign(new Emitter(), { sources: [], mediaSources: [] }) as unknown as MOQtailPlayer
    const controller = new LiveBufferController({ ...LIVE_BUFFER_DEFAULTS, catchupRate: 1.5, targetLatency: 0.5 })
    controller.attach(player, s.sink)
    onTestFinished(() => controller.detach())
    await s.sink.play()
    await until(() => s.sink.playbackRate > 1, 'catching up')
    await until(() => s.sink.playbackRate === 1 && forward(s.sink) < 0.75, 'back at the target')
    const seeked = count(s.sink, 'seeked')
    feed.lead = 3
    await until(() => seeked() > 0, 'a jump forward')
    expect(forward(s.sink)).toBeLessThan(1)
    feed.stop()
  })

  test('the rate speeds up the clock and video with it; seeks go forward only', async () => {
    const s = await open()
    const feed = live(s, { lead: 1.5 })
    await s.sink.play()
    await until(() => s.sink.currentTime > s.base + 0.2, 'the audio clock runs')
    s.sink.setPlaybackRate(1.1)
    const drawn = spyDraws(s.sink)
    const [time, wall] = [s.sink.currentTime, performance.now()]
    await new Promise((resolve) => setTimeout(resolve, 1000))
    const rate = (s.sink.currentTime - time) / ((performance.now() - wall) / 1000)
    expect(rate).toBeGreaterThan(1.05)
    expect(rate).toBeLessThan(1.15)
    expectInSync(drawn)

    const events = [count(s.sink, 'seeking'), count(s.sink, 'seeked')]
    const target = s.sink.currentTime + 0.5
    await s.sink.seek(target)
    expect(events.map((e) => e())).toEqual([1, 1])
    expect(drawn.at(-1)!.timestamp).toBeGreaterThanOrEqual(target)
    expect(s.sink.currentTime).toBeGreaterThanOrEqual(target - 0.05)
    await expect(s.sink.seek(s.sink.currentTime - 1)).rejects.toThrow(TypeError)
    feed.stop()
  })

  test('without an audio pad, or with a timescale, video follows the wall clock', async () => {
    for (const options of [{ audio: false }, { properties: { timescale: 1_000_000 } }]) {
      const s = await open(options)
      // Audio that started well before the video: following it would hold the video back
      // Ahead of the preroll, so the wall clock starts at once
      const feed = live(s, { audioOffset: -5, lead: 0.6 })
      await s.sink.play()
      const drawn = spyDraws(s.sink)
      await new Promise((resolve) => setTimeout(resolve, 1000))
      const span = drawn.at(-1)!.timestamp - drawn[0]!.timestamp
      expect(span).toBeGreaterThan(0.8)
      expect(span).toBeLessThan(1.1)
      feed.stop()
    }
  })

  test('the render queue holds a hardware decoder to its bound without stalling it', async ({ skip }) => {
    const config = { codec: 'avc1.42001f', hardwareAcceleration: 'prefer-hardware' as const }
    if (!(await VideoDecoder.isConfigSupported(config)).supported) skip('no hardware decoder')
    const chunks = await encodedVideo(RENDER_QUEUE_LIMIT + 30)
    const held: VideoFrame[] = []
    let failure: DOMException | undefined
    const decoder = new VideoDecoder({ output: (frame) => held.push(frame), error: (e) => (failure = e) })
    decoder.configure(config)
    for (const chunk of chunks) decoder.decode(chunk)
    // Held like the render queue holds them: the oldest goes once the bound is reached
    await until(() => {
      while (held.length > RENDER_QUEUE_LIMIT) held.shift()!.close()
      return decoder.decodeQueueSize === 0 && held.length === RENDER_QUEUE_LIMIT
    }, 'the decoder outputs every chunk')
    expect(failure).toBeUndefined()
    for (const frame of held) frame.close()
    decoder.close()
  })

  test('refuses non-live content', async () => {
    const { failures } = await open({ properties: { live: false }, ready: false })
    await until(() => failures.length > 0, 'a failure')
    expect(failures[0]).toMatchObject({ code: 'UNSUPPORTED_SOURCE', element: 'playout' })
  })

  test('a gap in the video dispatches waiting, and its next frame playing', async () => {
    for (const options of [{ audio: false }, {}]) {
      const s = await open(options)
      const feed = live(s, { lead: 0.1 })
      await s.sink.play()
      await until(() => s.sink.currentTime > s.base + 0.3, 'playing')
      const [waiting, playing] = [count(s.sink, 'waiting'), count(s.sink, 'playing')]
      // Longer than the preroll ahead of the clock
      feed.skip.video = FPS
      await until(() => waiting() === 1, 'waiting')
      const time = s.sink.currentTime
      await until(() => playing() === 1, 'playing again')
      expect(waiting()).toBe(1)
      // The audio played on through the gap
      if (s.audio) expect(s.sink.currentTime).toBeGreaterThan(time + 0.2)
      feed.stop()
    }
  })

  test('a gap in the audio longer than a chunk dispatches waiting, and the next audio playing', async () => {
    const chunk = 0.2
    const s = await open({ video: false })
    const feed = live(s, { lead: 0.1, chunk })
    await s.sink.play()
    await until(() => s.sink.currentTime > s.base + 0.3, 'playing')
    const [waiting, playing] = [count(s.sink, 'waiting'), count(s.sink, 'playing')]
    // The preroll covers one chunk missing; five run the audio out for longer than a chunk
    feed.skip.audio = 1
    await new Promise((resolve) => setTimeout(resolve, 500))
    expect([waiting(), playing()]).toEqual([0, 0])
    feed.skip.audio = 5
    await until(() => playing() === 1, 'playing again')
    expect(waiting()).toBe(1)
    feed.stop()
  })

  test('pause() drops what arrives, and play() resumes at the live edge', async () => {
    const s = await open()
    const feed = live(s)
    await s.sink.play()
    await until(() => s.sink.currentTime > s.base + 0.3, 'playing')
    const pauses = count(s.sink, 'pause')
    await s.sink.pause()
    expect([s.sink.paused, pauses()]).toEqual([true, 1])
    const paused = s.sink.currentTime
    await new Promise((resolve) => setTimeout(resolve, 1000))
    expect(s.sink.currentTime).toBe(paused)
    await s.sink.play()
    await until(() => s.sink.currentTime > paused + 0.9, 'at the live edge')
    expect(s.sink.paused).toBe(false)
    feed.stop()
  })

  test('eos on every pad ends it', async () => {
    const s = await open()
    const ended = count(s.sink, 'ended')
    const eos = { type: 'event' as const, event: { type: 'eos' as const } }
    // Each pad on its own, as a source writes them: frames past the render queue wait for the clock
    void writeFrames(s, range(1 / FPS, 0.5, 1 / FPS), []).then(() => s.video!.write(eos))
    void writeFrames(s, [], range(CHUNK, 0.5, CHUNK)).then(() => s.audio!.write(eos))
    await s.sink.play()
    await until(() => ended() === 1, 'ended')
    expect([s.sink.ended, s.sink.paused]).toEqual([true, true])
  })

  test('every frame it received is closed', async () => {
    const before = frames.length
    const s = await open()
    const feed = live(s, { lead: 1 })
    await s.sink.play()
    await until(() => s.sink.currentTime > s.base + 0.3, 'playing')
    await s.sink.seek(s.sink.currentTime + 0.3)
    await s.sink.pause()
    await s.sink.play()
    await new Promise((resolve) => setTimeout(resolve, 300))
    feed.stop()
    await s.destroy()
    expect(frames.slice(before).filter((f) => f.format !== null)).toEqual([])
  })
})

/**
 * A chain of a test source and the playout sink on a new canvas, ready once `write` wrote the
 * first frames. `properties` go to every pad with `live: true`.
 */
async function open(
  options: {
    video?: boolean
    audio?: boolean
    properties?: Partial<PadProperties>
    write?: (s: Session) => Promise<void>
    ready?: boolean
  } = {},
): Promise<Session & { destroy: () => Promise<void> }> {
  const { video = true, audio = true, properties = {}, write = (s) => writeFrames(s, [0], [0]), ready = true } = options
  const canvas = document.createElement('canvas')
  document.body.append(canvas)
  // Its pads are main-thread pads, so playout stays there either way
  const { chain, failures } = testChain(undefined, undefined, undefined, undefined, await chainWorker())
  let source: TestSource | undefined
  const script = (s: TestSource) => {
    source = s
    if (video) s.addPad(raw('video'))
    if (audio) s.addPad(raw('audio'))
    s.emit('nomorepads')
  }
  chain.addSource(testSource({ script }), {})
  chain.addSink(Playout, { canvas })
  const initialized = chain.init(new AbortController().signal)
  await until(() => source !== undefined && chain.sinks.length > 0, 'the chain')
  const [videoPad, audioPad] = [video ? source!.pads[0] : undefined, audio ? source!.pads.at(-1) : undefined]
  const session: Session = {
    sink: chain.sinks[0] as PlayoutSink,
    canvas,
    failures,
    video: videoPad,
    audio: audioPad,
    base: Math.round(Date.now() / 1000),
  }
  let destroyed: Promise<void> | undefined
  const destroy = () => (destroyed ??= chain.destroy().then(() => canvas.remove()))
  onTestFinished(destroy)
  for (const pad of source!.pads) {
    await pad.write({ type: 'properties', properties: { track: 't', live: true, ...properties } })
  }
  await write(session)
  if (ready) await initialized
  return { ...session, destroy }
}

/** Writes media whose timestamps reach `lead` seconds past the time it runs, as a live source does. */
function live(s: Session, options: { lead?: number; video?: boolean; chunk?: number; audioOffset?: number } = {}) {
  const { video = true, chunk = CHUNK, audioOffset = 0 } = options
  const start = performance.now()
  // The first frames went out with `open`
  let [nextVideo, nextAudio] = [1 / FPS, CHUNK + audioOffset]
  /** `skip` counts the frames and chunks to leave out next. */
  const feed = { lead: options.lead ?? 0.3, skip: { video: 0, audio: 0 }, stopped: false, stop: () => {} }
  feed.stop = () => {
    feed.stopped = true
  }
  const edge = () => (performance.now() - start) / 1000 + feed.lead
  // A loop per role, as a source delivers each track: a frame waiting to be taken holds no audio back
  const runVideo = async () => {
    while (!feed.stopped) {
      for (; nextVideo <= edge(); nextVideo += 1 / FPS)
        if (feed.skip.video > 0) feed.skip.video--
        else await writeFrames(s, [nextVideo], [])
      await flush()
    }
  }
  const runAudio = async () => {
    while (!feed.stopped) {
      for (; nextAudio <= edge() + audioOffset; nextAudio += chunk)
        if (feed.skip.audio > 0) feed.skip.audio--
        else await writeFrames(s, [], [nextAudio], undefined, chunk)
      await flush()
    }
  }
  if (video && s.video) void runVideo()
  if (s.audio) void runAudio()
  onTestFinished(feed.stop)
  return feed
}

/** Video frames and audio chunks at those seconds past `base`, the frames of one colour. */
async function writeFrames(
  s: Session,
  videoTimes: number[],
  audioTimes: number[],
  colour = 'red',
  chunk = CHUNK,
): Promise<void> {
  const canvas = new OffscreenCanvas(WIDTH, HEIGHT)
  const context = canvas.getContext('2d')!
  context.fillStyle = colour
  context.fillRect(0, 0, WIDTH, HEIGHT)
  const micros = (time: number) => Math.round((s.base + time) * 1_000_000)
  for (const time of videoTimes) {
    const payload = new VideoFrame(canvas, { timestamp: micros(time) })
    frames.push(payload)
    await s.video?.write({
      type: 'frame',
      frame: { track: 't', location: new Location(0n, 0n), properties: [], payload },
    })
  }
  for (const time of audioTimes) {
    const numberOfFrames = Math.round(chunk * SAMPLE_RATE)
    const data = Float32Array.from({ length: numberOfFrames }, (_, i) => Math.sin(i / 10) / 4)
    const payload = new AudioData({
      format: 'f32-planar',
      sampleRate: SAMPLE_RATE,
      numberOfChannels: 1,
      numberOfFrames,
      timestamp: micros(time),
      data,
    })
    await s.audio?.write({
      type: 'frame',
      frame: { track: 't', location: new Location(0n, 0n), properties: [], payload },
    })
  }
}

/** Each frame drawn: its media time, and the sink's `currentTime` then. */
function spyDraws(sink: PlayoutSink): { timestamp: number; currentTime: number }[] {
  const drawn: { timestamp: number; currentTime: number }[] = []
  const spy = vi.spyOn(CanvasRenderingContext2D.prototype, 'drawImage')
  spy.mockImplementation(function (this: CanvasRenderingContext2D, ...args: unknown[]) {
    drawn.push({ timestamp: (args[0] as VideoFrame).timestamp / 1_000_000, currentTime: sink.currentTime })
  })
  onTestFinished(() => spy.mockRestore())
  return drawn
}

/** Within one video frame plus one display frame of the clock, which the sink last reported. */
function expectInSync(drawn: { timestamp: number; currentTime: number }[]): void {
  for (const { timestamp, currentTime } of drawn)
    expect(Math.abs(timestamp - currentTime)).toBeLessThan(1 / FPS + 1 / 60 + 0.01)
}

function forward(sink: PlayoutSink): number {
  const { buffered } = sink
  return buffered.length ? buffered.end(0) - sink.currentTime : 0
}

function count(target: EventTarget, type: string): () => number {
  let n = 0
  target.addEventListener(type, () => n++)
  return () => n
}

async function until(condition: () => boolean, what: string, timeout = 8000): Promise<void> {
  await vi.waitUntil(condition, { timeout, interval: 10 }).catch(() => {
    throw new Error(`Timed out waiting for ${what}`)
  })
}

function range(from: number, to: number, step: number): number[] {
  return Array.from({ length: Math.round((to - from) / step) }, (_, i) => from + i * step)
}

function raw(role: string): Capabilities {
  return [
    { direction: 'out', code: CapabilityCode.PACKAGING, value: 'raw' },
    { direction: 'out', code: CapabilityCode.ROLE, value: role },
  ]
}

/** `count` frames of H.264 Annex B, the first a key frame. */
async function encodedVideo(count: number): Promise<EncodedVideoChunk[]> {
  const chunks: EncodedVideoChunk[] = []
  // flush() rejects with any encoding error
  const encoder = new VideoEncoder({ output: (chunk) => chunks.push(chunk), error: () => {} })
  encoder.configure({ codec: 'avc1.42001f', width: 1280, height: 720, avc: { format: 'annexb' } })
  const canvas = new OffscreenCanvas(1280, 720)
  for (let i = 0; i < count; i++) {
    const frame = new VideoFrame(canvas, { timestamp: i * 16_667 })
    encoder.encode(frame, { keyFrame: i === 0 })
    frame.close()
  }
  await encoder.flush()
  encoder.close()
  return chunks
}
