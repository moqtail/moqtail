/**
 * Copyright 2025 The MOQtail Authors
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

import {
  ExtensionHeaders,
  ObjectForwardingPreference,
  FilterType,
  GroupOrder,
  SubscribeError,
  Tuple,
  FullTrackName,
  MoqtObject,
  Location,
} from 'moqtail-ts/model'
import { MOQtailClient, LiveTrackSource, SubscribeOptions } from 'moqtail-ts/client'
import { PlayoutBuffer, NetworkTelemetry } from 'moqtail-ts/util'
import { RefObject } from 'react'

import DecodeWorker from '@/workers/decoderWorker?worker'
import PCMPlayerProcessorURL from '@/workers/pcmPlayerProcessor?url'

export async function connectToRelay(url: string) {
  return await MOQtailClient.new({ url, enableDatagrams: false })
}

export async function announceNamespaces(moqClient: MOQtailClient, namespace: Tuple) {
  await moqClient.publishNamespace(namespace)
}

export function setupTracks(
  moqClient: MOQtailClient,
  audioFullTrackName: FullTrackName,
  videoFullTrackName: FullTrackName,
) {
  let audioStreamController: ReadableStreamDefaultController<MoqtObject> | null = null
  const audioStream = new ReadableStream<MoqtObject>({
    start(controller) {
      audioStreamController = controller
    },
    cancel() {
      audioStreamController = null
    },
  })
  let videoStreamController: ReadableStreamDefaultController<MoqtObject> | null = null
  const videoStream = new ReadableStream<MoqtObject>({
    start(controller) {
      videoStreamController = controller
    },
    cancel() {
      videoStreamController = null
    },
  })

  const audioContentSource = new LiveTrackSource(audioStream)
  moqClient.addOrUpdateTrack({
    fullTrackName: audioFullTrackName,
    forwardingPreference: ObjectForwardingPreference.Subgroup,
    trackSource: { live: audioContentSource },
    publisherPriority: 128,
  })
  const videoContentSource = new LiveTrackSource(videoStream)
  moqClient.addOrUpdateTrack({
    fullTrackName: videoFullTrackName,
    forwardingPreference: ObjectForwardingPreference.Subgroup,
    trackSource: { live: videoContentSource },
    publisherPriority: 128,
  })
  return {
    audioStream,
    videoStream,
    getAudioStreamController: () => audioStreamController,
    getVideoStreamController: () => videoStreamController,
  }
}

export async function startAudioEncoder({
  stream,
  audioFullTrackName,
  audioStreamController,
  publisherPriority,
  audioGroupId,
  objectForwardingPreference,
}: {
  stream: MediaStream
  audioFullTrackName: FullTrackName
  audioStreamController: ReadableStreamDefaultController<MoqtObject> | null
  publisherPriority: number
  audioGroupId: number
  objectForwardingPreference: ObjectForwardingPreference
}) {
  let audioObjectId = 0n
  let currentAudioGroupId = audioGroupId
  let shouldEncode = true

  setInterval(() => {
    currentAudioGroupId += 1
    audioObjectId = 0n
  }, 2000)

  const audioContext = new AudioContext({ sampleRate: 48000 })
  await audioContext.audioWorklet.addModule(PCMPlayerProcessorURL)

  const source = audioContext.createMediaStreamSource(stream) // same stream as video
  const audioNode = new AudioWorkletNode(audioContext, 'audio-encoder-processor')
  source.connect(audioNode)
  audioNode.connect(audioContext.destination)

  let audioEncoder: AudioEncoder | null = null
  if (typeof AudioEncoder !== 'undefined') {
    audioEncoder = new AudioEncoder({
      output: (chunk) => {
        if (!shouldEncode) return

        const payload = new Uint8Array(chunk.byteLength)
        chunk.copyTo(payload)

        const captureTime = Math.round(Date.now())
        const locHeaders = new ExtensionHeaders().addCaptureTimestamp(captureTime)

        // console.warn('Audio Group ID is:', currentAudioGroupId)
        const moqt = MoqtObject.newWithPayload(
          audioFullTrackName,
          new Location(BigInt(currentAudioGroupId), BigInt(audioObjectId++)),
          publisherPriority,
          objectForwardingPreference,
          BigInt(Math.round(Date.now())),
          locHeaders.build(),
          payload,
        )
        audioStreamController?.enqueue(moqt)
      },
      error: console.error,
    })
    audioEncoder.configure(window.appSettings.audioEncoderConfig)
  }

  let pcmBuffer: Float32Array[] = []
  const AUDIO_PACKET_SAMPLES = 960

  audioNode.port.onmessage = (event) => {
    if (!audioEncoder) return
    if (!shouldEncode) return

    const samples = event.data as Float32Array
    pcmBuffer.push(samples)

    let totalSamples = pcmBuffer.reduce((sum, arr) => sum + arr.length, 0)
    while (totalSamples >= AUDIO_PACKET_SAMPLES) {
      let out = new Float32Array(AUDIO_PACKET_SAMPLES)
      let offset = 0
      while (offset < AUDIO_PACKET_SAMPLES && pcmBuffer.length > 0) {
        let needed = AUDIO_PACKET_SAMPLES - offset
        let chunk = pcmBuffer[0]
        if (chunk.length <= needed) {
          out.set(chunk, offset)
          offset += chunk.length
          pcmBuffer.shift()
        } else {
          out.set(chunk.subarray(0, needed), offset)
          pcmBuffer[0] = chunk.subarray(needed)
          offset += needed
        }
      }
      const audioData = new AudioData({
        format: 'f32',
        sampleRate: 48000,
        numberOfFrames: AUDIO_PACKET_SAMPLES,
        numberOfChannels: 1,
        timestamp: performance.now() * 1000,
        data: out.buffer,
      })
      audioEncoder.encode(audioData)
      audioData.close()
      totalSamples -= AUDIO_PACKET_SAMPLES
    }
  }

  return {
    audioNode,
    audioEncoder,
    setEncoding: (enabled: boolean) => {
      shouldEncode = enabled
      if (!enabled) {
        pcmBuffer = []
      }
    },
  }
}

export function initializeVideoEncoder({
  videoFullTrackName,
  videoStreamController,
  publisherPriority,
  objectForwardingPreference,
}: {
  videoFullTrackName: FullTrackName
  videoStreamController: ReadableStreamDefaultController<MoqtObject> | null
  publisherPriority: number
  objectForwardingPreference: ObjectForwardingPreference
}) {
  let videoEncoder: VideoEncoder | null = null
  let encoderActive = true
  let videoGroupId = 0
  let videoObjectId = 0n
  let isFirstKeyframeSent = false
  let videoConfig: ArrayBuffer | null = null
  let frameCounter = 0
  const pendingVideoTimestamps: number[] = []
  let videoReader: ReadableStreamDefaultReader<any> | null = null

  const createVideoEncoder = () => {
    isFirstKeyframeSent = false
    //videoGroupId = 0 //if problematic, open this
    videoObjectId = 0n
    frameCounter = 0
    pendingVideoTimestamps.length = 0
    //videoConfig = null

    videoEncoder = new VideoEncoder({
      output: async (chunk, meta) => {
        if (chunk.type === 'key') {
          videoGroupId++
          videoObjectId = 0n
        }

        let captureTime = pendingVideoTimestamps.shift()
        if (captureTime === undefined) {
          console.warn('No capture time available for video frame, skipping')
          captureTime = Math.round(Date.now())
        }

        const locHeaders = new ExtensionHeaders()
          .addCaptureTimestamp(captureTime)
          .addVideoFrameMarking(chunk.type === 'key' ? 1 : 0)

        const desc = meta?.decoderConfig?.description
        if (!isFirstKeyframeSent && desc instanceof ArrayBuffer) {
          videoConfig = desc
          locHeaders.addVideoConfig(new Uint8Array(desc))
          isFirstKeyframeSent = true
        }
        if (isFirstKeyframeSent && videoConfig instanceof ArrayBuffer) {
          locHeaders.addVideoConfig(new Uint8Array(videoConfig))
        }
        const frameData = new Uint8Array(chunk.byteLength)
        chunk.copyTo(frameData)

        const moqt = MoqtObject.newWithPayload(
          videoFullTrackName,
          new Location(BigInt(videoGroupId), BigInt(videoObjectId++)),
          publisherPriority,
          objectForwardingPreference,
          0n,
          locHeaders.build(),
          frameData,
        )
        if (videoStreamController) {
          videoStreamController.enqueue(moqt)
        } else {
          console.error('videoStreamController is not available')
        }
      },
      error: console.error,
    })
    console.debug('Configuring video encoder with settings:', window.appSettings.videoEncoderConfig)
    videoEncoder.configure(window.appSettings.videoEncoderConfig)
  }

  createVideoEncoder()

  const stop = async () => {
    encoderActive = false
    if (videoReader) {
      try {
        await videoReader.cancel()
      } catch (e) {
        // ignore cancel errors
      }
      videoReader = null
    }
    if (videoEncoder) {
      try {
        await videoEncoder.flush()
        videoEncoder.close()
      } catch (e) {
        // ignore close errors
      }
      videoEncoder = null
    }
  }

  return {
    videoEncoder,
    encoderActive,
    pendingVideoTimestamps,
    frameCounter,
    start: async (stream: MediaStream) => {
      // Stop previous encoder and reset state
      if (videoEncoder && encoderActive) {
        encoderActive = false
        await stop()
      }

      if (!stream) {
        return { videoEncoder: null, videoReader: null }
      }

      encoderActive = true
      createVideoEncoder()

      const videoTrack = stream.getVideoTracks()[0]
      if (!videoTrack) {
        return { videoEncoder: null, videoReader: null }
      }

      videoReader = new (window as any).MediaStreamTrackProcessor({
        track: videoTrack,
      }).readable.getReader()

      const readAndEncode = async (reader: ReadableStreamDefaultReader<any>) => {
        while (encoderActive) {
          try {
            const result = await reader.read()
            if (result.done) break

            const captureTime = Math.round(Date.now())
            pendingVideoTimestamps.push(captureTime)

            try {
              let insert_keyframe = false
              if (window.appSettings.keyFrameInterval !== 'auto') {
                insert_keyframe = frameCounter % (window.appSettings.keyFrameInterval || 0) === 0
              }

              if (insert_keyframe) {
                videoEncoder?.encode(result.value, { keyFrame: insert_keyframe })
              } else {
                videoEncoder?.encode(result.value)
              }
              frameCounter++
            } catch (encodeError) {
              console.error('Error encoding video frame:', encodeError)
            } finally {
              if (result.value && typeof result.value.close === 'function') {
                result.value.close()
              }
            }
          } catch (readError) {
            console.error('Error reading video frame:', readError)
            if (!encoderActive) break
          }
        }
      }

      if (!videoReader) {
        console.error('Failed to create video reader')
        return
      }
      if (videoReader) {
        readAndEncode(videoReader)
      }
      return { videoEncoder, videoReader }
    },
    stop,
  }
}

export async function startVideoEncoder({
  stream,
  videoFullTrackName,
  videoStreamController,
  publisherPriority,
  objectForwardingPreference,
}: {
  stream: MediaStream
  videoFullTrackName: FullTrackName
  videoStreamController: ReadableStreamDefaultController<MoqtObject> | null
  publisherPriority: number
  objectForwardingPreference: ObjectForwardingPreference
}) {
  if (!stream) {
    console.error('No stream provided to video encoder')
    return { stop: async () => {} }
  }

  let videoEncoder: VideoEncoder | null = null
  let videoReader: ReadableStreamDefaultReader<any> | null = null
  let encoderActive = true
  let videoGroupId = 0
  let videoObjectId = 0n
  let isFirstKeyframeSent = false
  let videoConfig: ArrayBuffer | null = null
  let frameCounter = 0
  const pendingVideoTimestamps: number[] = []

  const createVideoEncoder = () => {
    isFirstKeyframeSent = false
    videoGroupId = 0
    videoObjectId = 0n
    frameCounter = 0
    pendingVideoTimestamps.length = 0
    videoConfig = null

    videoEncoder = new VideoEncoder({
      output: async (chunk, meta) => {
        if (chunk.type === 'key') {
          videoGroupId++
          videoObjectId = 0n
        }

        let captureTime = pendingVideoTimestamps.shift()
        if (captureTime === undefined) {
          console.warn('No capture time available for video frame, skipping')
          captureTime = Math.round(Date.now())
        }

        const locHeaders = new ExtensionHeaders()
          .addCaptureTimestamp(captureTime)
          .addVideoFrameMarking(chunk.type === 'key' ? 1 : 0)

        const desc = meta?.decoderConfig?.description
        if (!isFirstKeyframeSent && desc instanceof ArrayBuffer) {
          videoConfig = desc
          locHeaders.addVideoConfig(new Uint8Array(desc))
          isFirstKeyframeSent = true
        }
        if (isFirstKeyframeSent && videoConfig instanceof ArrayBuffer) {
          locHeaders.addVideoConfig(new Uint8Array(videoConfig))
        }
        const frameData = new Uint8Array(chunk.byteLength)
        chunk.copyTo(frameData)

        const moqt = MoqtObject.newWithPayload(
          videoFullTrackName,
          new Location(BigInt(videoGroupId), BigInt(videoObjectId++)),
          publisherPriority,
          objectForwardingPreference,
          0n,
          locHeaders.build(),
          frameData,
        )
        if (videoStreamController) {
          videoStreamController.enqueue(moqt)
        } else {
          console.error('videoStreamController is not available')
        }
      },
      error: console.error,
    })
    videoEncoder.configure(window.appSettings.videoEncoderConfig)
  }

  createVideoEncoder()

  const videoTrack = stream.getVideoTracks()[0]
  if (!videoTrack) {
    console.error('No video track available in stream')
    return { stop: async () => {} }
  }

  videoReader = new (window as any).MediaStreamTrackProcessor({
    track: videoTrack,
  }).readable.getReader()

  const readAndEncode = async (reader: ReadableStreamDefaultReader<any>) => {
    while (encoderActive) {
      try {
        const result = await reader.read()
        if (result.done) break

        const captureTime = Math.round(Date.now())
        pendingVideoTimestamps.push(captureTime)

        // Our video is 25 fps. Each 2s, we can send a new keyframe.
        const insert_keyframe = frameCounter % 50 === 0

        try {
          videoEncoder?.encode(result.value, { keyFrame: insert_keyframe })
          frameCounter++
        } catch (encodeError) {
          console.error('Error encoding video frame:', encodeError)
        } finally {
          if (result.value && typeof result.value.close === 'function') {
            result.value.close()
          }
        }
      } catch (readError) {
        console.error('Error reading video frame:', readError)
        if (!encoderActive) break
      }
    }
  }

  if (!videoReader) {
    console.error('Failed to create video reader')
    return { stop: async () => {} }
  }
  readAndEncode(videoReader)

  const stop = async () => {
    encoderActive = false
    if (videoReader) {
      try {
        await videoReader.cancel()
      } catch (e) {
        // ignore cancel errors
      }
      videoReader = null
    }
    if (videoEncoder) {
      try {
        await videoEncoder.flush()
        videoEncoder.close()
      } catch (e) {
        // ignore close errors
      }
      videoEncoder = null
    }
  }

  return { videoEncoder, videoReader, stop }
}

const canvasWorkerMap = new WeakMap<HTMLCanvasElement, Worker>()
const canvasAudioNodeMap = new WeakMap<HTMLCanvasElement, AudioWorkletNode>()

function getOrCreateWorkerAndCanvas(canvas: HTMLCanvasElement) {
  const existingWorker = canvasWorkerMap.get(canvas)
  if (existingWorker) {
    existingWorker.postMessage({
      type: 'updateDecoderConfig',
      decoderConfig: window.appSettings.videoDecoderConfig,
    })

    resizeCanvasWorker(
      canvas,
      window.appSettings.videoDecoderConfig.codedHeight || 640,
      window.appSettings.videoDecoderConfig.codedWidth || 360,
    )
    return existingWorker
  }

  try {
    const worker = new DecodeWorker()
    const offscreen = canvas.transferControlToOffscreen()
    worker.postMessage({ type: 'init', canvas: offscreen, decoderConfig: window.appSettings.videoDecoderConfig }, [
      offscreen,
    ])

    canvasWorkerMap.set(canvas, worker)

    const originalTerminate = worker.terminate
    worker.terminate = function () {
      canvasWorkerMap.delete(canvas)
      return originalTerminate.call(this)
    }

    return worker
  } catch (error) {
    if (error instanceof DOMException && error.name === 'InvalidStateError') {
      console.error('Canvas control already transferred. This should not happen with proper cleanup.')
    }
    throw error
  }
}

export function cleanupCanvasWorker(canvas: HTMLCanvasElement): boolean {
  const worker = canvasWorkerMap.get(canvas)
  if (worker) {
    worker.terminate()
    canvasWorkerMap.delete(canvas)
    return true
  }
  return false
}

/**
 * Creates (or reuses) a decode worker + audio pipeline for the given canvas.
 * Idempotent: safe to call multiple times for the same canvas.
 */
export async function prepareReceiverForCanvas(canvasRef: RefObject<HTMLCanvasElement | null>): Promise<Worker | null> {
  const canvas = canvasRef.current
  if (!canvas) return null
  const worker = getOrCreateWorkerAndCanvas(canvas)
  if (!canvasAudioNodeMap.has(canvas)) {
    const audioNode = await setupAudioPlayback(new AudioContext({ sampleRate: 48000 }))
    canvasAudioNodeMap.set(canvas, audioNode)
    worker.onmessage = (event) => {
      if (event.data.type === 'audio') {
        audioNode.port.postMessage(new Float32Array(event.data.samples))
      }
    }
  }
  return worker
}

/**
 * Wraps an existing ReadableStream<MoqtObject> in a PlayoutBuffer and pipes
 * objects to the decode worker. Use after prepareReceiverForCanvas.
 */
export function pipeStreamToCanvas(
  stream: ReadableStream<MoqtObject>,
  trackType: 'moq' | 'moq-audio',
  worker: Worker,
): void {
  const buffer = new PlayoutBuffer(stream, {
    targetLatencyMs: window.appSettings.playoutBufferConfig.targetLatencyMs,
    maxLatencyMs: window.appSettings.playoutBufferConfig.maxLatencyMs,
    clock: { now: () => Date.now() },
  })
  buffer.onObject = (obj) => {
    if (!obj || !obj.payload) return
    worker.postMessage(
      { type: trackType, extensions: obj.extensionHeaders, payload: obj, serverTimestamp: Date.now() },
      [obj.payload.buffer],
    )
  }
}

async function setupAudioPlayback(audioContext: AudioContext) {
  await audioContext.audioWorklet.addModule(PCMPlayerProcessorURL)
  const audioNode = new AudioWorkletNode(audioContext, 'pcm-player-processor')
  audioNode.connect(audioContext.destination)
  return audioNode
}

function subscribeAndPipeToWorker(
  moqClient: MOQtailClient,
  subscribeArgs: SubscribeOptions,
  worker: Worker,
  type: 'moq' | 'moq-audio',
): Promise<bigint | undefined> {
  return moqClient.subscribe(subscribeArgs).then((response) => {
    window.appSettings.playoutBufferConfig.maxLatencyMs
    if (!(response instanceof SubscribeError)) {
      const { requestId, stream } = response
      const buffer = new PlayoutBuffer(stream, {
        targetLatencyMs: window.appSettings.playoutBufferConfig.targetLatencyMs,
        maxLatencyMs: window.appSettings.playoutBufferConfig.maxLatencyMs,
        clock: { now: () => Date.now() },
      })
      buffer.onObject = (obj) => {
        if (!obj) {
          // Stream ended or error
          console.warn(`Buffer terminated ${type}`)
          return
        }

        if (!obj.payload) {
          console.warn('Received MoqtObject without payload, skipping:', obj)
          // Request next object immediately
          return
        }
        // Send to worker
        worker.postMessage(
          {
            type,
            extensions: obj.extensionHeaders,
            payload: obj,
            serverTimestamp: Date.now(),
          },
          [obj.payload.buffer],
        )
      }

      return requestId
    } else {
      console.error('Subscribe Error:', response)
      return undefined
    }
  })
}

export function subscribeOnlyVideo(moqClient: MOQtailClient, videoFullTrackName: FullTrackName) {
  const setup = async (): Promise<{ videoRequestId?: bigint; cleanup: () => Promise<void> }> => {
    try {
      const response = await moqClient.subscribe({
        fullTrackName: videoFullTrackName,
        groupOrder: GroupOrder.Original,
        filterType: FilterType.LatestObject,
        forward: true,
        priority: 0,
      })

      if (response instanceof SubscribeError) {
        console.error('subscribeOnlyVideo: subscribe returned error', response)
        return { cleanup: async () => {} }
      }

      const { requestId } = response

      return {
        videoRequestId: requestId,
        cleanup: async () => {
          try {
            await moqClient.unsubscribe(requestId)
          } catch (err) {
            console.warn('subscribeOnlyVideo: failed to unsubscribe', err)
          }
        },
      }
    } catch (err) {
      console.error('subscribeOnlyVideo: unexpected error', err)
      return { cleanup: async () => {} }
    }
  }
  return setup
}

function handleWorkerMessages(
  worker: Worker,
  audioNode: AudioWorkletNode,
  videoTelemetry?: NetworkTelemetry,
  audioTelemetry?: NetworkTelemetry,
) {
  worker.onmessage = (event) => {
    if (event.data.type === 'audio') {
      audioNode.port.postMessage(new Float32Array(event.data.samples))
    }
    if (event.data.type === 'video-telemetry') {
      if (videoTelemetry) {
        videoTelemetry.push({
          latency: Math.abs(event.data.latency),
          size: event.data.throughput,
        })
      }
    }
    if (event.data.type === 'audio-telemetry') {
      if (audioTelemetry) {
        audioTelemetry.push({
          latency: Math.abs(event.data.latency),
          size: event.data.throughput,
        })
      }
    }
  }
}

export function useVideoPublisher(
  moqClient: MOQtailClient,
  videoRef: RefObject<HTMLVideoElement>,
  mediaStream: RefObject<MediaStream | null>,
  _roomId: string,
  _userId: string,
  videoFullTrackName: FullTrackName,
  audioFullTrackName: FullTrackName,
) {
  const setup = async () => {
    const video = videoRef.current
    if (!video) {
      console.error('Video element is not available')
      return
    }

    const stream = mediaStream.current
    if (stream instanceof MediaStream) {
      video.srcObject = stream
    } else {
      console.error('Expected MediaStream, got:', stream)
    }
    if (!stream) {
      console.error('MediaStream is not available')
      return
    }
    video.muted = true
    announceNamespaces(moqClient, videoFullTrackName.namespace)

    let tracks = setupTracks(moqClient, audioFullTrackName, videoFullTrackName)

    const videoPromise = startVideoEncoder({
      stream,
      videoFullTrackName,
      videoStreamController: tracks.getVideoStreamController(),
      publisherPriority: 1,
      objectForwardingPreference: ObjectForwardingPreference.Subgroup,
    })

    const audioPromise = startAudioEncoder({
      stream,
      audioFullTrackName,
      audioStreamController: tracks.getAudioStreamController(),
      publisherPriority: 1,
      audioGroupId: 0,
      objectForwardingPreference: ObjectForwardingPreference.Subgroup,
    })

    await Promise.all([videoPromise, audioPromise])

    return () => {}
  }
  return setup
}

export function useVideoAndAudioSubscriber(
  moqClient: MOQtailClient,
  canvasRef: RefObject<HTMLCanvasElement | null>,
  videoFullTrackName: FullTrackName,
  audioFullTrackName: FullTrackName,
  videoTelemetry?: NetworkTelemetry,
  audioTelemetry?: NetworkTelemetry,
) {
  const setup = async (): Promise<{ videoRequestId?: bigint; audioRequestId?: bigint; cleanup: () => void }> => {
    const canvas = canvasRef.current
    if (!canvas) return { cleanup: () => {} }
    const worker = getOrCreateWorkerAndCanvas(canvas)
    const audioNode = await setupAudioPlayback(new AudioContext({ sampleRate: 48000 }))

    handleWorkerMessages(worker, audioNode, videoTelemetry, audioTelemetry)

    const audioRequestId = await subscribeAndPipeToWorker(
      moqClient,
      {
        fullTrackName: audioFullTrackName,
        groupOrder: GroupOrder.Original,
        filterType: FilterType.LatestObject,
        forward: true,
        priority: 0,
      },
      worker,
      'moq-audio',
    )
    console.info('Subscribed to audio', audioFullTrackName, 'with requestId:', audioRequestId)

    const videoRequestId = await subscribeAndPipeToWorker(
      moqClient,
      {
        fullTrackName: videoFullTrackName,
        groupOrder: GroupOrder.Original,
        filterType: FilterType.LatestObject,
        forward: true,
        priority: 0,
      },
      worker,
      'moq',
    )
    console.info('Subscribed to video', videoFullTrackName, 'with requestId:', videoRequestId)

    return {
      videoRequestId,
      audioRequestId,
      cleanup: () => {
        worker.terminate()
      },
    }
  }
  return setup
}

export function onlyUseVideoSubscriber(
  moqClient: MOQtailClient,
  canvasRef: RefObject<HTMLCanvasElement | null>,
  videoFullTrackName: FullTrackName,
  videoTelemetry?: NetworkTelemetry,
  onFirstFrame?: () => void,
) {
  const setup = async (): Promise<{ videoRequestId?: bigint; cleanup: () => void }> => {
    const canvas = canvasRef.current
    if (!canvas) return { cleanup: () => {} }
    const worker = getOrCreateWorkerAndCanvas(canvas)
    let firstFrameReceived = false

    worker.onmessage = (event: MessageEvent) => {
      if (event.data.type === 'video-telemetry') {
        // Track first frame
        if (!firstFrameReceived && onFirstFrame) {
          firstFrameReceived = true
          onFirstFrame()
        }

        if (videoTelemetry) {
          videoTelemetry.push({
            latency: Math.abs(event.data.latency),
            size: event.data.throughput,
          })
        }
      }
    }

    const videoRequestId = await subscribeAndPipeToWorker(
      moqClient,
      {
        fullTrackName: videoFullTrackName,
        groupOrder: GroupOrder.Original,
        filterType: FilterType.LatestObject,
        forward: true,
        priority: 0,
      },
      worker,
      'moq',
    )

    return {
      videoRequestId,
      cleanup: () => {},
    }
  }
  return setup
}

export function onlyUseAudioSubscriber(
  moqClient: MOQtailClient,
  audioFullTrackName: FullTrackName,
  audioTelemetry?: NetworkTelemetry,
  onFirstFrame?: () => void,
) {
  const setup = async (): Promise<{ audioRequestId?: bigint; cleanup: () => void }> => {
    const worker = new DecodeWorker()
    worker.postMessage({ type: 'init-audio-only', decoderConfig: window.appSettings.audioDecoderConfig })

    const audioNode = await setupAudioPlayback(new AudioContext({ sampleRate: 48000 }))

    let firstFrameReceived = false

    worker.onmessage = (event) => {
      if (event.data.type === 'audio') {
        // Track first audio frame
        if (!firstFrameReceived && onFirstFrame) {
          firstFrameReceived = true
          onFirstFrame()
        }

        audioNode.port.postMessage(new Float32Array(event.data.samples))
      }
      if (event.data.type === 'audio-telemetry') {
        if (audioTelemetry) {
          audioTelemetry.push({
            latency: Math.abs(event.data.latency),
            size: event.data.throughput,
          })
        }
      }
    }

    const audioRequestId = await subscribeAndPipeToWorker(
      moqClient,
      {
        fullTrackName: audioFullTrackName,
        groupOrder: GroupOrder.Original,
        filterType: FilterType.LatestObject,
        forward: true,
        priority: 0,
      },
      worker,
      'moq-audio',
    )

    return {
      audioRequestId,
      cleanup: () => {
        // ! Do not terminate the worker
      },
    }
  }
  return setup
}

export function resizeCanvasWorker(canvas: HTMLCanvasElement, newWidth: number, newHeight: number): void {
  const worker = canvasWorkerMap.get(canvas)
  if (worker) {
    worker.postMessage({
      type: 'resize',
      newWidth,
      newHeight,
    })
  }
}
