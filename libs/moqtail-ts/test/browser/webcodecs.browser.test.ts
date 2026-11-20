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

/** The `webcodecs` transform with the browser's decoders, on locally encoded LOC packets. Needs no relay. */

import { describe, expect, test } from 'vitest'
import { Location } from '@/model/common/location'
import { Timestamp } from '@/model/property/timestamp'
import { VideoFrameMarking } from '@/model/property/video_frame_marking'
import type { Frame, Packet, PadProperties } from '@/player/element/pad'
import { locCapabilities } from '@/player/elements/loc/track'
import { WebCodecs } from '@/player/elements/webcodecs'
import {
  collectingSink,
  detachedContext,
  testChain,
  testSource,
  type CollectingSink,
  type TestSource,
} from '@/player/testing/elements'

const WIDTH = 320
const HEIGHT = 240
const AUDIO_FRAMES = 960

describe('webcodecs', () => {
  test('length-prefixed H.264 decodes into VideoFrames with the LOC timestamps and object locations', async () => {
    const chunks = await encodedVideo(8)
    const packets = chunks.map((chunk, i) =>
      packetOf(i, chunk, lengthPrefixed(bytesOf(chunk)), [
        new VideoFrameMarking(new Uint8Array([chunk.type === 'key' ? 0x20 : 0])).toKeyValuePair(),
      ]),
    )
    const frames = await decoded('video', { codec: 'avc3.42001f', codedWidth: WIDTH, codedHeight: HEIGHT }, packets)
    expect(frames.map((f) => f.location)).toEqual(packets.map((p) => p.location))
    for (const [i, frame] of frames.entries()) {
      expect(frame.payload).toBeInstanceOf(VideoFrame)
      expect(frame.payload.timestamp).toBe(chunks[i]!.timestamp)
      frame.payload.close()
    }
  })

  test('Opus decodes into AudioData with the LOC timestamps and object locations', async () => {
    const chunks = await encodedAudio(10)
    const packets = chunks.map((chunk, i) => packetOf(i, chunk, bytesOf(chunk)))
    const frames = await decoded('audio', { codec: 'opus', sampleRate: 48_000, numberOfChannels: 2 }, packets)
    expect(frames.length).toBeGreaterThan(0)
    for (const frame of frames) {
      const packet = packets.find((p) => p.location.equals(frame.location))!
      expect(frame.payload).toBeInstanceOf(AudioData)
      expect(frame.payload.timestamp).toBe(chunks[packets.indexOf(packet)]!.timestamp)
      frame.payload.close()
    }
  })
})

/** The frames a `webcodecs` instance makes of `packets` on a `role` pad, once their `eos` is through. */
async function decoded(
  role: string,
  decoderConfig: NonNullable<PadProperties['decoderConfig']>,
  packets: Packet[],
): Promise<Frame[]> {
  let source: TestSource | undefined
  let sink: CollectingSink | undefined
  const { chain, failures } = testChain()
  const script = (s: TestSource) => {
    source = s
    s.addPad(locCapabilities('out', role))
    s.emit('nomorepads')
  }
  chain.addSource(testSource({ script }), {})
  chain.addTransform(await WebCodecs.init({ ...detachedContext(), fail: (error) => chain.fail(error) }))
  chain.addSink(collectingSink({ packaging: 'raw', setup: (k) => (sink = k) }))
  await chain.init(new AbortController().signal)
  const pad = source!.pads[0]!
  await pad.write({ type: 'properties', properties: { track: 'v1', decoderConfig } })
  for (const packet of packets) await pad.write({ type: 'packet', packet })
  await pad.write({ type: 'event', event: { type: 'eos' } })
  await expect.poll(() => sink!.items().at(-1)).toEqual({ type: 'event', event: { type: 'eos' } })
  expect(failures).toEqual([])
  const frames = sink!.items().flatMap((item) => (item.type === 'frame' ? [item.frame] : []))
  await chain.destroy()
  return frames
}

function packetOf(
  object: number,
  chunk: EncodedVideoChunk | EncodedAudioChunk,
  payload: Uint8Array,
  properties: Packet['properties'] = [],
): Packet {
  return {
    track: 'v1',
    location: new Location(0n, BigInt(object)),
    properties: [new Timestamp(BigInt(chunk.timestamp)).toKeyValuePair(), ...properties],
    payload,
  }
}

function bytesOf(chunk: EncodedVideoChunk | EncodedAudioChunk): Uint8Array {
  const bytes = new Uint8Array(chunk.byteLength)
  chunk.copyTo(bytes)
  return bytes
}

/** `count` frames of a changing colour, H.264 Annex B with in-band parameter sets, the first a key frame. */
async function encodedVideo(count: number): Promise<EncodedVideoChunk[]> {
  const chunks: EncodedVideoChunk[] = []
  // flush() rejects with any encoding error
  const encoder = new VideoEncoder({ output: (chunk) => chunks.push(chunk), error: () => {} })
  encoder.configure({ codec: 'avc1.42001f', width: WIDTH, height: HEIGHT, avc: { format: 'annexb' } })
  const canvas = new OffscreenCanvas(WIDTH, HEIGHT)
  const context = canvas.getContext('2d')!
  for (let i = 0; i < count; i++) {
    context.fillStyle = `hsl(${i * 40}, 80%, 50%)`
    context.fillRect(0, 0, WIDTH, HEIGHT)
    const frame = new VideoFrame(canvas, { timestamp: 1_700_000_000_000_000 + i * 33_333 })
    encoder.encode(frame, { keyFrame: i === 0 })
    frame.close()
  }
  await encoder.flush()
  encoder.close()
  return chunks
}

/** `count` 20 ms Opus frames of a stereo tone. */
async function encodedAudio(count: number): Promise<EncodedAudioChunk[]> {
  const chunks: EncodedAudioChunk[] = []
  // flush() rejects with any encoding error
  const encoder = new AudioEncoder({ output: (chunk) => chunks.push(chunk), error: () => {} })
  encoder.configure({ codec: 'opus', sampleRate: 48_000, numberOfChannels: 2 })
  for (let i = 0; i < count; i++) {
    const samples = Float32Array.from({ length: AUDIO_FRAMES * 2 }, (_, n) => Math.sin(n / 10) / 2)
    const data = new AudioData({
      format: 'f32-planar',
      sampleRate: 48_000,
      numberOfChannels: 2,
      numberOfFrames: AUDIO_FRAMES,
      timestamp: 1_700_000_000_000_000 + i * 20_000,
      data: samples,
    })
    encoder.encode(data)
    data.close()
  }
  await encoder.flush()
  encoder.close()
  return chunks
}

/** `annexB` with 4-byte NAL unit lengths in place of start codes, as moqlivemock sends `avc3`. */
function lengthPrefixed(annexB: Uint8Array): Uint8Array {
  const units: Uint8Array[] = []
  let start = -1
  for (let i = 0; i + 2 < annexB.length; i++) {
    if (annexB[i] !== 0 || annexB[i + 1] !== 0 || annexB[i + 2] !== 1) continue
    // A 4-byte start code leaves a zero at the end of the unit before it
    let end = i
    while (end > start && annexB[end - 1] === 0) end--
    if (start !== -1) units.push(annexB.subarray(start, end))
    start = i + 3
    i += 2
  }
  units.push(annexB.subarray(start))
  const output = new Uint8Array(units.reduce((length, unit) => length + 4 + unit.length, 0))
  const view = new DataView(output.buffer)
  let offset = 0
  for (const unit of units) {
    view.setUint32(offset, unit.length)
    output.set(unit, offset + 4)
    offset += 4 + unit.length
  }
  return output
}
