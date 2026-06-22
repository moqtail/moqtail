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

import { useEffect, useRef } from 'preact/hooks';

interface LocalPreviewProps {
  cameraStream: MediaStream | null;
  screenStream: MediaStream | null;
}

function PreviewVideo({
  stream,
  className,
  label,
}: {
  stream: MediaStream;
  className?: string;
  label?: string;
}) {
  const ref = useRef<HTMLVideoElement | null>(null);

  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    el.srcObject = stream;
    el.play().catch(() => {});
    return () => {
      el.srcObject = null;
    };
  }, [stream]);

  return (
    <div className={`relative overflow-hidden ${className ?? ''}`}>
      <video ref={ref} autoPlay muted playsInline className="h-full w-full object-cover" />
      {label && (
        <span className="absolute bottom-2 left-2 rounded bg-black/60 px-2 py-0.5 font-mono text-[10px] text-white/70">
          {label}
        </span>
      )}
    </div>
  );
}

export function LocalPreview({ cameraStream, screenStream }: LocalPreviewProps) {
  const hasBoth = !!cameraStream && !!screenStream;
  const hasEither = !!cameraStream || !!screenStream;

  if (!hasEither) return null;

  if (hasBoth) {
    // Screen as main, camera PiP in bottom-right
    return (
      <div className="relative h-full w-full overflow-hidden rounded-xl bg-black">
        <PreviewVideo stream={screenStream!} className="h-full w-full" label="screen" />
        <div className="absolute right-4 bottom-4 h-[22%] w-[25%] overflow-hidden rounded-lg border-2 border-white/20 shadow-lg">
          <PreviewVideo stream={cameraStream!} className="h-full w-full" label="camera" />
        </div>
      </div>
    );
  }

  const stream = cameraStream ?? screenStream!;
  const label = cameraStream ? 'camera' : 'screen';
  return (
    <div className="h-full w-full overflow-hidden rounded-xl bg-black">
      <PreviewVideo stream={stream} className="h-full w-full" label={label} />
    </div>
  );
}
