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
  Tuple,
  FullTrackName,
  MoqtObject,
  Location,
  KeyValuePair,
} from 'moqtail/model';
import { MOQtailClient, LiveTrackSource } from 'moqtail/client';
import { PlayoutBuffer } from '@/lib/playout_buffer';
import { RefObject } from 'react';

import DecodeWorker from '@/workers/decoderWorker?worker';
import PCMPlayerProcessorURL from '@/workers/pcmPlayerProcessor?url';

export async function connectToRelay(url: string) {
  return await MOQtailClient.new({ url, enableDatagrams: false });
}

export async function announceNamespaces(moqClient: MOQtailClient, namespace: Tuple) {
  await moqClient.publishNamespace(namespace);
}

export function setupTracks(
  moqClient: MOQtailClient,
  audioFullTrackName: FullTrackName,
  videoFullTrackName: FullTrackName,
) {
  let audioStreamController: ReadableStreamDefaultController<MoqtObject> | null = null;
  const audioStream = new ReadableStream<MoqtObject>({
    start(controller) {
      audioStreamController = controller;
    },
    cancel() {
      audioStreamController = null;
    },
  });
  let videoStreamController: ReadableStreamDefaultController<MoqtObject> | null = null;
  const videoStream = new ReadableStream<MoqtObject>({
    start(controller) {
      videoStreamController = controller;
    },
    cancel() {
      videoStreamController = null;
    },
  });

  const audioContentSource = new LiveTrackSource(audioStream);
  moqClient.addOrUpdateTrack({
    fullTrackName: audioFullTrackName,

    trackSource: { live: audioContentSource },
    publisherPriority: 128,
  });
  const videoContentSource = new LiveTrackSource(videoStream);
  moqClient.addOrUpdateTrack({
    fullTrackName: videoFullTrackName,

    trackSource: { live: videoContentSource },
    publisherPriority: 128,
  });
  return {
    audioStream,
    videoStream,
    getAudioStreamController: () => audioStreamController,
    getVideoStreamController: () => videoStreamController,
  };
}

export async function startAudioEncoder({
  stream,
  audioFullTrackName,
  audioStreamController,
  publisherPriority,
  audioGroupId,
  objectForwardingPreference,
}: {
  stream: MediaStream;
  audioFullTrackName: FullTrackName;
  audioStreamController: ReadableStreamDefaultController<MoqtObject> | null;
  publisherPriority: number;
  audioGroupId: number;
  objectForwardingPreference: ObjectForwardingPreference;
}) {
  let audioObjectId = 0n;
  let currentAudioGroupId = audioGroupId;
  let shouldEncode = true;

  // Speech-activity VAD state (key 0x12): 0=SILENT, 1=SPEAKING, 2=SPEECH_START
  // Only emitted when topN is configured (no wire overhead otherwise).
  const topNEnabled = !!window.appSettings?.topN;
  const SILENT_THRESH = 0.01;
  const SPEAK_THRESH = 0.03;
  // Hangover: natural speech has short pauses between words/sentences. Without this,
  // every such pause reports SILENT, which drops the speaker out of the relay's top-N
  // candidates and re-triggers a SPEECH_START blip (which itself outranks ongoing
  // SPEAKING) the moment they resume — causing rapid top-N churn during normal talking.
  const SILENCE_HANGOVER_MS = 500;
  let lastSpeechActivity = 0n;
  let lastActiveAt = 0;

  setInterval(() => {
    currentAudioGroupId += 1;
    audioObjectId = 0n;
  }, 2000);

  const audioContext = new AudioContext({ sampleRate: 48000 });
  await audioContext.audioWorklet.addModule(PCMPlayerProcessorURL);

  const source = audioContext.createMediaStreamSource(stream); // same stream as video
  const audioNode = new AudioWorkletNode(audioContext, 'audio-encoder-processor');
  source.connect(audioNode);
  audioNode.connect(audioContext.destination);

  let audioEncoder: AudioEncoder | null = null;
  if (typeof AudioEncoder !== 'undefined') {
    audioEncoder = new AudioEncoder({
      output: chunk => {
        if (!shouldEncode) return;

        const payload = new Uint8Array(chunk.byteLength);
        chunk.copyTo(payload);

        const captureTime = Math.round(Date.now());
        const locHeaders = new ExtensionHeaders().addCaptureTimestamp(captureTime);
        if (topNEnabled) {
          // Speech-activity VAD extension (key 0x12); updated from PCM RMS in onmessage
          locHeaders.addRaw(KeyValuePair.tryNewVarInt(0x12n, lastSpeechActivity));
        }

        // console.warn('Audio Group ID is:', currentAudioGroupId)
        const moqt = MoqtObject.newWithPayload(
          audioFullTrackName,
          new Location(BigInt(currentAudioGroupId), BigInt(audioObjectId++)),
          publisherPriority,
          objectForwardingPreference,
          BigInt(Math.round(Date.now())),
          locHeaders.build(),
          payload,
        );
        audioStreamController?.enqueue(moqt);
      },
      error: console.error,
    });
    audioEncoder.configure(window.appSettings.audioEncoderConfig);
  }

  let pcmBuffer: Float32Array[] = [];
  const AUDIO_PACKET_SAMPLES = 960;

  audioNode.port.onmessage = event => {
    console.log('Audio node message received:', event.data, topNEnabled);
    if (!audioEncoder) return;
    if (!shouldEncode) return;

    const samples = event.data as Float32Array;

    if (topNEnabled) {
      const rms = Math.sqrt(samples.reduce((s, v) => s + v * v, 0) / samples.length);
      const now = performance.now();

      if (rms >= SILENT_THRESH) {
        const wasSilent = lastSpeechActivity === 0n;
        lastSpeechActivity = wasSilent && rms >= SPEAK_THRESH ? 2n : 1n;
        lastActiveAt = now;
      } else if (lastSpeechActivity !== 0n && now - lastActiveAt < SILENCE_HANGOVER_MS) {
        // Within the hangover window: treat as a natural pause, not silence.
        lastSpeechActivity = 1n;
      } else {
        lastSpeechActivity = 0n;
      }
    }

    pcmBuffer.push(samples);

    let totalSamples = pcmBuffer.reduce((sum, arr) => sum + arr.length, 0);
    while (totalSamples >= AUDIO_PACKET_SAMPLES) {
      let out = new Float32Array(AUDIO_PACKET_SAMPLES);
      let offset = 0;
      while (offset < AUDIO_PACKET_SAMPLES && pcmBuffer.length > 0) {
        let needed = AUDIO_PACKET_SAMPLES - offset;
        let chunk = pcmBuffer[0];
        if (chunk.length <= needed) {
          out.set(chunk, offset);
          offset += chunk.length;
          pcmBuffer.shift();
        } else {
          out.set(chunk.subarray(0, needed), offset);
          pcmBuffer[0] = chunk.subarray(needed);
          offset += needed;
        }
      }
      const audioData = new AudioData({
        format: 'f32',
        sampleRate: 48000,
        numberOfFrames: AUDIO_PACKET_SAMPLES,
        numberOfChannels: 1,
        timestamp: performance.now() * 1000,
        data: out.buffer,
      });
      audioEncoder.encode(audioData);
      audioData.close();
      totalSamples -= AUDIO_PACKET_SAMPLES;
    }
  };

  return {
    audioNode,
    audioEncoder,
    setActive: (active: boolean) => {
      shouldEncode = active;
      if (!active) {
        pcmBuffer = [];
      }
    },
  };
}

export async function startVideoEncoder({
  stream,
  videoFullTrackName,
  videoStreamController,
  publisherPriority,
  objectForwardingPreference,
}: {
  stream: MediaStream;
  videoFullTrackName: FullTrackName;
  videoStreamController: ReadableStreamDefaultController<MoqtObject> | null;
  publisherPriority: number;
  objectForwardingPreference: ObjectForwardingPreference;
}) {
  if (!stream) {
    console.error('No stream provided to video encoder');
    return { stop: async () => {} };
  }

  let videoEncoder: VideoEncoder | null = null;
  let videoReader: ReadableStreamDefaultReader<any> | null = null;
  let encoderActive = true;
  let videoGroupId = 0;
  let videoObjectId = 0n;
  let isFirstKeyframeSent = false;
  let videoConfig: ArrayBuffer | null = null;
  let frameCounter = 0;
  const pendingVideoTimestamps: number[] = [];

  const createVideoEncoder = () => {
    isFirstKeyframeSent = false;
    //videoGroupId = 0 //if problematic, open this
    videoObjectId = 0n;
    frameCounter = 0;
    pendingVideoTimestamps.length = 0;
    //videoConfig = null

    videoEncoder = new VideoEncoder({
      output: async (chunk, meta) => {
        if (chunk.type === 'key') {
          videoGroupId++;
          videoObjectId = 0n;
        }

        let captureTime = pendingVideoTimestamps.shift();
        if (captureTime === undefined) {
          console.warn('No capture time available for video frame, skipping');
          captureTime = Math.round(Date.now());
        }

        const locHeaders = new ExtensionHeaders()
          .addCaptureTimestamp(captureTime)
          .addVideoFrameMarking(chunk.type === 'key' ? 1 : 0);

        const desc = meta?.decoderConfig?.description;
        if (!isFirstKeyframeSent && desc instanceof ArrayBuffer) {
          videoConfig = desc;
          locHeaders.addVideoConfig(new Uint8Array(desc));
          isFirstKeyframeSent = true;
        }
        if (isFirstKeyframeSent && videoConfig instanceof ArrayBuffer) {
          locHeaders.addVideoConfig(new Uint8Array(videoConfig));
        }
        const frameData = new Uint8Array(chunk.byteLength);
        chunk.copyTo(frameData);

        const moqt = MoqtObject.newWithPayload(
          videoFullTrackName,
          new Location(BigInt(videoGroupId), BigInt(videoObjectId++)),
          publisherPriority,
          objectForwardingPreference,
          0n,
          locHeaders.build(),
          frameData,
        );
        if (videoStreamController) {
          videoStreamController.enqueue(moqt);
        } else {
          console.error('videoStreamController is not available');
        }
      },
      error: console.error,
    });
    videoEncoder.configure(window.appSettings.videoEncoderConfig);
  };

  createVideoEncoder();

  const videoTrack = stream.getVideoTracks()[0];
  if (!videoTrack) {
    console.error('No video track available in stream');
    return { stop: async () => {} };
  }

  videoReader = new (window as any).MediaStreamTrackProcessor({
    track: videoTrack,
  }).readable.getReader();

  const readAndEncode = async (reader: ReadableStreamDefaultReader<any>) => {
    while (encoderActive) {
      try {
        const result = await reader.read();
        if (result.done) break;

        const captureTime = Math.round(Date.now());
        pendingVideoTimestamps.push(captureTime);

        try {
          let insert_keyframe = false;
          if (window.appSettings.keyFrameInterval !== 'auto') {
            insert_keyframe = frameCounter % (window.appSettings.keyFrameInterval || 0) === 0;
          }

          if (insert_keyframe) {
            videoEncoder?.encode(result.value, { keyFrame: insert_keyframe });
          } else {
            videoEncoder?.encode(result.value);
          }
          frameCounter++;
        } catch (encodeError) {
          console.error('Error encoding video frame:', encodeError);
        } finally {
          if (result.value && typeof result.value.close === 'function') {
            result.value.close();
          }
        }
      } catch (readError) {
        console.error('Error reading video frame:', readError);
        if (!encoderActive) break;
      }
    }
  };

  if (!videoReader) {
    console.error('Failed to create video reader');
    return { stop: async () => {} };
  }
  readAndEncode(videoReader);

  const stop = async () => {
    encoderActive = false;
    if (videoReader) {
      try {
        await videoReader.cancel();
      } catch (e) {
        // ignore cancel errors
      }
      videoReader = null;
    }
    if (videoEncoder) {
      try {
        await videoEncoder.flush();
        videoEncoder.close();
      } catch (e) {
        // ignore close errors
      }
      videoEncoder = null;
    }
  };

  return { videoEncoder, videoReader, stop };
}

const canvasWorkerMap = new WeakMap<HTMLCanvasElement, Worker>();
const canvasAudioNodeMap = new WeakMap<HTMLCanvasElement, AudioWorkletNode>();

function getOrCreateWorkerAndCanvas(canvas: HTMLCanvasElement) {
  const existingWorker = canvasWorkerMap.get(canvas);
  if (existingWorker) {
    console.log(
      'getOrCreateWorkerAndCanvas | sending updateDecoderConfig to existingWorker',
      window.appSettings.videoDecoderConfig,
    );
    existingWorker.postMessage({
      type: 'updateDecoderConfig',
      decoderConfig: window.appSettings.videoDecoderConfig,
    });

    resizeCanvasWorker(
      canvas,
      window.appSettings.videoDecoderConfig.codedHeight || 640,
      window.appSettings.videoDecoderConfig.codedWidth || 360,
    );
    return existingWorker;
  }

  try {
    const worker = new DecodeWorker();
    const offscreen = canvas.transferControlToOffscreen();
    worker.postMessage(
      { type: 'init', canvas: offscreen, decoderConfig: window.appSettings.videoDecoderConfig },
      [offscreen],
    );

    canvasWorkerMap.set(canvas, worker);

    const originalTerminate = worker.terminate;
    worker.terminate = function () {
      canvasWorkerMap.delete(canvas);
      return originalTerminate.call(this);
    };

    return worker;
  } catch (error) {
    if (error instanceof DOMException && error.name === 'InvalidStateError') {
      console.error(
        'Canvas control already transferred. This should not happen with proper cleanup.',
      );
    } else {
      console.error('getOrCreateWorkerAndCanvas | error', error);
    }

    throw error;
  }
}

export function cleanupCanvasWorker(canvas: HTMLCanvasElement): boolean {
  const worker = canvasWorkerMap.get(canvas);
  if (worker) {
    worker.terminate();
    canvasWorkerMap.delete(canvas);
    return true;
  }
  return false;
}

/**
 * Creates (or reuses) a decode worker + audio pipeline for the given canvas.
 * Idempotent: safe to call multiple times for the same canvas.
 */
export async function prepareReceiverForCanvas(
  canvasRef: RefObject<HTMLCanvasElement | null>,
  onSpeechActivity?: (value: number) => void,
): Promise<Worker | null> {
  const canvas = canvasRef.current;
  if (!canvas) {
    console.error('prepareReceiverForCanvas | canvas is null');
    return null;
  }
  const worker = getOrCreateWorkerAndCanvas(canvas);
  let audioNode = canvasAudioNodeMap.get(canvas);
  if (!audioNode) {
    audioNode = await setupAudioPlayback(new AudioContext({ sampleRate: 48000 }));
    canvasAudioNodeMap.set(canvas, audioNode);
  }
  // Rewired on every call (not just once per canvas) so a canvas slot reused by a
  // different peer always dispatches speech-activity updates for the current peer.
  worker.onmessage = event => {
    if (event.data.type === 'audio') {
      audioNode!.port.postMessage(new Float32Array(event.data.samples));
    }
    if (event.data.type === 'speech-activity') {
      onSpeechActivity?.(event.data.value);
    }
  };
  return worker;
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
  });
  buffer.onObject = obj => {
    if (!obj || !obj.payload) return;
    worker.postMessage(
      {
        type: trackType,
        extensions: obj.extensionHeaders,
        payload: obj,
        serverTimestamp: Date.now(),
      },
      [obj.payload.buffer],
    );
  };
}

async function setupAudioPlayback(audioContext: AudioContext) {
  await audioContext.audioWorklet.addModule(PCMPlayerProcessorURL);
  const audioNode = new AudioWorkletNode(audioContext, 'pcm-player-processor');
  audioNode.connect(audioContext.destination);
  return audioNode;
}

export function resizeCanvasWorker(
  canvas: HTMLCanvasElement,
  newWidth: number,
  newHeight: number,
): void {
  const worker = canvasWorkerMap.get(canvas);
  if (worker) {
    worker.postMessage({
      type: 'resize',
      newWidth,
      newHeight,
    });
  }
}
