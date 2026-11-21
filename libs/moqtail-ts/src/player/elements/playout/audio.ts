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

import { PROCESSOR_NAME, PROCESSOR_SOURCE, type ProcessorOptions } from './worklet'

/** The main-thread side of audio playout. */
export interface AudioOutput {
  context: AudioContext
  node: AudioWorkletNode
}

/**
 * A suspended context at the media's sample rate, so samples map one to one to media time at
 * rate 1, and the playout processor, waiting for `preroll` media seconds, connected to its output.
 * shortcut: keeps the first chunk's sample rate and channel count, rebuild it if a switch changes them
 */
export async function createAudioOutput(
  sampleRate: number,
  numberOfChannels: number,
  preroll: number,
): Promise<AudioOutput> {
  const context = new AudioContext({ latencyHint: 'interactive', sampleRate })
  void context.suspend()

  // The package ships no worklet file. A strict CSP can block `blob:` modules.
  const url = URL.createObjectURL(new Blob([PROCESSOR_SOURCE], { type: 'text/javascript' }))

  try {
    await context.audioWorklet.addModule(url)
  } catch (error) {
    void context.close()
    throw error
  } finally {
    URL.revokeObjectURL(url)
  }

  const node = new AudioWorkletNode(context, PROCESSOR_NAME, {
    numberOfInputs: 0,
    outputChannelCount: [numberOfChannels],
    processorOptions: { preroll } satisfies ProcessorOptions['processorOptions'],
  })

  // shortcut: no volume or mute, a GainNode goes here once asked
  node.connect(context.destination)
  return { context, node }
}
