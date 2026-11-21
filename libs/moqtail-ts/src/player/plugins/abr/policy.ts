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
 * The model: a small multilayer perceptron with tanh hidden layers and int4 weights, shipped as a
 * few hundred bytes of base64. The packed form is
 *
 *   u8 FEATURES_VERSION | u32le featuresHash() | u8 layer count
 *   | per layer: u8 inputs, u8 outputs, f16le weight scale, f16le bias scale
 *   | int4 codes, two per byte, low nibble first: per layer the weights row by row, then the biases
 *
 * optionally deflate-raw compressed, whichever is smaller.
 */

import { Action, FEATURES_VERSION, featuresHash } from './features'

/** @public A model as shipped: base64 of the packed form, and whether it is compressed. */
export interface ModelBlob {
  data: string
  codec: 'deflate-raw' | 'raw'
}

interface Layer {
  inputs: number
  outputs: number
  /** Row-major: `outputs` rows of `inputs` weights. */
  weights: Float64Array
  biases: Float64Array
}

/** A decoded model: logits over `Action` for a feature vector. */
export class Policy {
  readonly #layers: readonly Layer[]

  constructor(layers: readonly Layer[]) {
    this.#layers = layers
  }

  /**
   * Decodes a blob.
   * @throws TypeError for a model trained on another feature list, which would read the inputs wrong.
   */
  static async load(blob: ModelBlob): Promise<Policy> {
    const packed = base64Bytes(blob.data)
    return new Policy(parseLayers(blob.codec === 'deflate-raw' ? await inflate(packed) : packed))
  }

  logits(features: readonly number[]): number[] {
    let values: readonly number[] = features
    this.#layers.forEach((layer, index) => {
      const hidden = index < this.#layers.length - 1
      values = applyLayer(layer, values).map((v) => (hidden ? Math.tanh(v) : v))
    })

    return [...values]
  }

  /**
   * Draws an allowed action by the model's odds instead of taking the likeliest: some choices, such
   * as an occasional probe on a calm link, it learned only as odds. `random` returns numbers in [0, 1).
   */
  choose(features: readonly number[], legal: readonly boolean[], random: () => number = Math.random): Action {
    const logits = this.logits(features)
    const top = Math.max(...logits.filter((_, action) => legal[action]))
    const weights = logits.map((logit, action) => (legal[action] ? Math.exp(logit - top) : 0))
    let draw = random() * weights.reduce((sum, weight) => sum + weight, 0)

    for (let action = 0; action < weights.length; action++) {
      draw -= weights[action]!
      if (draw < 0 && weights[action]! > 0) return action as Action
    }

    return Action.Stay
  }
}

function applyLayer({ inputs, outputs, weights, biases }: Layer, values: readonly number[]): number[] {
  return Array.from({ length: outputs }, (_, row) => {
    let sum = biases[row]!
    for (let column = 0; column < inputs; column++) sum += weights[row * inputs + column]! * values[column]!
    return sum
  })
}

function base64Bytes(data: string): Uint8Array {
  return Uint8Array.from(atob(data), (char) => char.charCodeAt(0))
}

async function inflate(bytes: Uint8Array): Promise<Uint8Array> {
  const stream = new Blob([bytes as BlobPart]).stream().pipeThrough(new DecompressionStream('deflate-raw'))
  return new Uint8Array(await new Response(stream).arrayBuffer())
}

function parseLayers(bytes: Uint8Array): Layer[] {
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength)
  if (view.getUint8(0) !== FEATURES_VERSION || view.getUint32(1, true) !== featuresHash())
    throw new TypeError('The model was trained on another feature list')

  const shapes = Array.from({ length: view.getUint8(5) }, (_, index) => {
    const at = 6 + index * 6
    return {
      inputs: view.getUint8(at),
      outputs: view.getUint8(at + 1),
      weightScale: float16(view.getUint16(at + 2, true)),
      biasScale: float16(view.getUint16(at + 4, true)),
    }
  })

  const codes = int4Codes(bytes, 6 + shapes.length * 6)

  return shapes.map(({ inputs, outputs, weightScale, biasScale }) => ({
    inputs,
    outputs,
    weights: Float64Array.from({ length: inputs * outputs }, () => codes.next() * weightScale),
    biases: Float64Array.from({ length: outputs }, () => codes.next() * biasScale),
  }))
}

/** Reads signed 4-bit codes from `offset` on, two per byte, low nibble first. */
function int4Codes(bytes: Uint8Array, offset: number): { next(): number } {
  let nibble = offset * 2
  return {
    next() {
      const byte = bytes[nibble >> 1]!
      const code = nibble % 2 === 0 ? byte & 0xf : byte >> 4
      nibble++

      return code >= 8 ? code - 16 : code
    },
  }
}

/** IEEE 754 half precision; `DataView.getFloat16` is too new to rely on. Scales are finite. */
function float16(bits: number): number {
  const sign = bits & 0x8000 ? -1 : 1
  const exponent = (bits >> 10) & 0x1f
  const fraction = bits & 0x3ff

  return exponent === 0 ? sign * fraction * 2 ** -24 : sign * (1 + fraction / 1024) * 2 ** (exponent - 15)
}
