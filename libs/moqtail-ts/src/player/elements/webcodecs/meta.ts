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

import { CapabilityCode, type Identifiers, type TransformElementDefinition } from '@/player/element/define'
import { decoderConfigOf } from '@/player/elements/loc/track'
import type { Codecs } from './decoding'

const ROLES = ['video', 'audio']

/** The WebCodecs transform without its `init`, which the built-in loads on first use. */
export const WEBCODECS_META = {
  type: 'transform',
  name: 'webcodecs',
  capabilities: [
    [
      { direction: 'in', code: CapabilityCode.PACKAGING, value: 'loc' },
      ...ROLES.map((value) => ({ direction: 'in' as const, code: CapabilityCode.ROLE, value })),
    ],
    [
      { direction: 'out', code: CapabilityCode.PACKAGING, value: 'raw' },
      ...ROLES.map((value) => ({ direction: 'out' as const, code: CapabilityCode.ROLE, value })),
    ],
  ],
  supports: decodable,
  threads: (environment) => (environment.worker?.has('element:webcodecs') ? 'both' : 'main'),
} satisfies Omit<TransformElementDefinition, 'init'>

/** Whether the browser decodes `track` with the config the LOC source would write for it. */
async function decodable(track: Identifiers): Promise<boolean> {
  const { VideoDecoder, AudioDecoder } = globalThis as Partial<Codecs>
  if (!track.codec) return false

  const config = decoderConfigOf(track, track.codec)

  // A config WebCodecs finds invalid, such as audio without a sample rate, rejects with a TypeError
  const support =
    'sampleRate' in config
      ? AudioDecoder?.isConfigSupported(config).catch(() => undefined)
      : VideoDecoder?.isConfigSupported(config).catch(() => undefined)

  return (await support)?.supported === true
}

if (import.meta.vitest) {
  const { test, expect, vi } = import.meta.vitest
  const { convertedCapabilities } = await import('@/player/element/negotiation')
  const { locCapabilities } = await import('@/player/elements/loc/track')

  test('a loc pad converts to a raw pad of its role', () => {
    for (const role of ROLES) {
      expect(convertedCapabilities(WEBCODECS_META.capabilities, locCapabilities('out', role))).toEqual([
        { direction: 'out', code: CapabilityCode.PACKAGING, value: 'raw' },
        { direction: 'out', code: CapabilityCode.ROLE, value: role },
      ])
    }
    expect(convertedCapabilities(WEBCODECS_META.capabilities, locCapabilities('out', 'subtitle'))).toBeUndefined()
  })

  test('supports asks the decoder for the config the LOC source would write', async () => {
    const asked: unknown[] = []
    const isConfigSupported = async (config: { codec: string }) => {
      asked.push(config)
      return { supported: config.codec !== 'hvc1.1.6.L93.B0' }
    }

    vi.stubGlobal('VideoDecoder', { isConfigSupported })
    vi.stubGlobal('AudioDecoder', { isConfigSupported: () => Promise.reject(new TypeError('invalid')) })

    try {
      const video = { packaging: 'loc', role: 'video', width: 1280, height: 720 }
      const answers = await Promise.all([
        WEBCODECS_META.supports({ ...video, codec: 'avc3.64001f' }),
        WEBCODECS_META.supports({ ...video, codec: 'hvc1.1.6.L93.B0' }),
        WEBCODECS_META.supports({
          packaging: 'loc',
          role: 'audio',
          codec: 'opus',
          samplerate: 48_000,
          channelConfig: '2',
        }),
      ])

      expect(answers).toEqual([true, false, false])
      expect(asked[0]).toEqual({ codec: 'avc3.64001f', codedWidth: 1280, codedHeight: 720 })
    } finally {
      vi.unstubAllGlobals()
    }
  })
}
