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

import { defineElement } from '@/player/element/define'
import { PlayerError } from '@/player/error'
import type { Codecs } from './decoding'
import { WEBCODECS_META } from './meta'
import { WebCodecsTransform } from './transform'

/**
 * @public
 * Decodes a `loc` pad with WebCodecs into a `raw` pad of `VideoFrame`s or `AudioData`, one pad
 * per instance. The player loads it on first use; importing it from `moqtail/player/elements/webcodecs`
 * and passing it to `registerElement()` puts its code in your bundle.
 */
export const WebCodecs = defineElement({
  ...WEBCODECS_META,
  init: async (context) => {
    const { VideoDecoder, AudioDecoder, EncodedVideoChunk, EncodedAudioChunk } = globalThis as Partial<Codecs>
    if (!VideoDecoder || !AudioDecoder || !EncodedVideoChunk || !EncodedAudioChunk) {
      throw new PlayerError('UNSUPPORTED_SOURCE', 'WebCodecs is not available', { element: 'webcodecs' })
    }
    return new WebCodecsTransform(context, { VideoDecoder, AudioDecoder, EncodedVideoChunk, EncodedAudioChunk })
  },
})

if (import.meta.vitest) {
  const { test, expect } = import.meta.vitest

  test('init fails with UNSUPPORTED_SOURCE without WebCodecs', async () => {
    const { detachedContext } = await import('@/player/testing/elements')
    await expect(WebCodecs.init(detachedContext())).rejects.toMatchObject({ code: 'UNSUPPORTED_SOURCE' })
  })
}
