// PCMPlayerProcessor: Receives PCM data from main thread and outputs it
class PCMPlayerProcessor extends AudioWorkletProcessor {
  constructor() {
    super()
    this.buffer = []
    this.port.onmessage = (event) => {
      const data = new Float32Array(event.data)
      this.buffer.push(...data)
    }
  }

  process(inputs, outputs) {
    const output = outputs[0]
    const channel = output[0]
    for (let i = 0; i < channel.length; i++) {
      channel[i] = this.buffer.length > 0 ? this.buffer.shift() : 0
    }
    return true
  }
}
registerProcessor('pcm-player-processor', PCMPlayerProcessor)

// This is the processor that captures PCM data
class AudioEncoderProcessor extends AudioWorkletProcessor {
  process(inputs) {
    const input = inputs[0]
    if (input && input[0]) {
      // Send Float32Array data to main thread
      this.port.postMessage(input[0]) // mono channel
    }
    return true
  }
}
registerProcessor('audio-encoder-processor', AudioEncoderProcessor)
