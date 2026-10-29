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

import { readFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { describe, expect, test, vi } from 'vitest'
import { MmtpTrackReceiver } from '@/mmtp'
import { FullTrackName } from '@/model/data/full_track_name'
import { Location } from '@/model/common/location'
import { MoqtObject } from '@/model/data/object'
import { ObjectForwardingPreference, ObjectStatus } from '@/model/data/constant'

interface VectorCase {
  name: string
  objects: string[]
  mfus: { mpuSequenceNumber: number; rapFlag: boolean; data: string }[]
  rejected: number
}

const VECTORS = resolve(dirname(fileURLToPath(import.meta.url)), '../../../dev/conformance/mmtp/receiver-v1.json')
const { cases } = JSON.parse(readFileSync(VECTORS, 'utf8')) as { cases: VectorCase[] }
const track = FullTrackName.tryNew('vectors', 'mmtp')

const fromHex = (hex: string) =>
  Uint8Array.from({ length: hex.length / 2 }, (_, i) => parseInt(hex.slice(i * 2, i * 2 + 2), 16))
const toHex = (bytes: Uint8Array) => Array.from(bytes, (b) => b.toString(16).padStart(2, '0')).join('')

function object(id: number, payload: Uint8Array): MoqtObject {
  return MoqtObject.newWithPayload(
    track,
    new Location(0n, BigInt(id)),
    128,
    ObjectForwardingPreference.Subgroup,
    0n,
    null,
    payload,
  )
}

describe('MmtpTrackReceiver vectors', () => {
  test('the vector file is non-empty and has negative controls', () => {
    expect(cases.length).toBeGreaterThan(0)
    expect(cases.some((c) => c.rejected > 0)).toBe(true)
    expect(cases.some((c) => c.mfus.length > 0)).toBe(true)
  })

  for (const c of cases) {
    test(c.name, () => {
      const rejects: unknown[] = []
      const receiver = new MmtpTrackReceiver({ onReject: (error) => rejects.push(error) })
      const mfus = c.objects.flatMap((hex, i) => receiver.push(object(i, fromHex(hex))) ?? [])
      expect(
        mfus.map((m) => ({ mpuSequenceNumber: m.mpuSequenceNumber, rapFlag: m.rapFlag, data: toHex(m.data) })),
      ).toEqual(c.mfus)
      expect(rejects).toHaveLength(c.rejected)
      expect(receiver.stats).toEqual({
        packets: c.objects.length,
        statusObjects: 0,
        mfus: c.mfus.length,
        rejected: c.rejected,
        evicted: 0,
      })
    })
  }
})

describe('MmtpTrackReceiver', () => {
  const [complete] = cases.find((c) => c.name === 'complete-mfu-rap')!.objects

  test('status objects carry no MMTP and are counted, not fed', () => {
    const receiver = new MmtpTrackReceiver()
    const end = MoqtObject.newWithStatus(
      track,
      new Location(0n, 1n),
      128,
      ObjectForwardingPreference.Subgroup,
      0n,
      null,
      ObjectStatus.EndOfGroup,
    )
    expect(receiver.push(end)).toBeNull()
    expect(receiver.stats).toEqual({ packets: 0, statusObjects: 1, mfus: 0, rejected: 0, evicted: 0 })
  })

  test('a rejected object does not stop the track', () => {
    const receiver = new MmtpTrackReceiver()
    expect(receiver.push(object(0, new Uint8Array(3)))).toBeNull()
    expect(receiver.push(object(1, fromHex(complete!)))).not.toBeNull()
    expect(receiver.stats.rejected).toBe(1)
  })

  test('transform() reassembles a subscription object stream', async () => {
    const receiver = new MmtpTrackReceiver()
    const fragmented = cases.find((c) => c.name === 'fragmented-in-order')!.objects
    const objects = [complete!, ...fragmented].map((hex, i) => object(i, fromHex(hex)))
    const source = new ReadableStream<MoqtObject>({
      start(controller) {
        for (const o of objects) controller.enqueue(o)
        controller.close()
      },
    })
    const out = []
    for await (const mfu of source.pipeThrough(receiver.transform())) out.push(toHex(mfu.data))
    expect(out).toEqual(['01020304', '0a0b0c0d0e'])
  })

  test('reassembly is clock-free: a parked MFU survives any delay', () => {
    vi.useFakeTimers()
    try {
      const receiver = new MmtpTrackReceiver()
      const [last, middle, first] = cases.find((c) => c.name === 'fragmented-continuations-first')!.objects
      receiver.push(object(0, fromHex(last!)))
      receiver.push(object(1, fromHex(middle!)))
      vi.advanceTimersByTime(24 * 60 * 60 * 1000)
      expect(toHex(receiver.push(object(2, fromHex(first!)))!.data)).toBe('0a0b0c0d0e')
    } finally {
      vi.useRealTimers()
    }
  })

  test('reset() drops a partially received MFU', () => {
    const receiver = new MmtpTrackReceiver()
    const [first, ...rest] = cases.find((c) => c.name === 'fragmented-in-order')!.objects
    receiver.push(object(0, fromHex(first!)))
    receiver.reset()
    expect(rest.map((hex, i) => receiver.push(object(i + 1, fromHex(hex))))).toEqual([null, null])
  })

  test('an MFU evicted by maxBuffered is counted, and survives reset()', () => {
    const receiver = new MmtpTrackReceiver({ maxBuffered: 1 })
    const [run2] = cases.find((c) => c.name === 'fragmented-in-order')!.objects
    // The same head fragment re-addressed to MPU 3: a second open data unit.
    const run3 = run2!.replace(/^(.{16})000000c8(.{8})00000002/, '$1000000d0$200000003')
    expect(receiver.push(object(0, fromHex(run2!)))).toBeNull()
    expect(receiver.push(object(1, fromHex(run3)))).toBeNull()
    expect(receiver.stats.evicted).toBe(1)
    receiver.reset()
    expect(receiver.stats.evicted).toBe(1)
  })
})
