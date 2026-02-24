window.appSettings = {
  relayUrl: 'https://localhost:4433',
  videoEncoderConfig: {
    codec: 'avc1.42E01F',
    width: 640,
    height: 360,
    bitrate: 300_000,
    framerate: 25,
    latencyMode: 'realtime',
    hardwareAcceleration: 'prefer-software',
  },
  videoDecoderConfig: {
    codec: 'avc1.42E01F',
    optimizeForLatency: true,
    hardwareAcceleration: 'prefer-software',
  },
  audioEncoderConfig: {
    codec: 'opus',
    sampleRate: 48000,
    numberOfChannels: 1,
    bitrate: 48_000,
  },
  audioDecoderConfig: {
    codec: 'opus',
    sampleRate: 48000,
    numberOfChannels: 1,
    bitrate: 48_000,
  },
  keyFrameInterval: 50,
  playoutBufferConfig: {
    targetLatencyMs: 100,
    maxLatencyMs: 1000,
  },
}
