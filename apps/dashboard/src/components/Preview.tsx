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

import type { VideoHTMLAttributes } from 'preact';
import { useEffect, useRef, useState } from 'preact/hooks';
import type { MOQtailVideoElement } from 'moqtail/player/component';
import type { ApiClient } from '../api/client';
import type { Page, TrackInfo } from '../api/types';
import type { Relay } from '../lib/relays';
import { useDashboard } from '../context';
import { usePoll } from '../lib/poll';
import { Card } from './ui';

declare module 'preact' {
  namespace JSX {
    interface IntrinsicElements {
      'moqtail-video': VideoHTMLAttributes<MOQtailVideoElement>;
    }
  }
}

/** The relay's MoQ address: the one set for the relay, or a guess from the API's
 *  address. An API with its own port is the relay itself, which listens for MoQ
 *  on its bind port; one without is behind a proxy on 443, as MoQ then is too. */
export async function moqUrl(relay: Relay, api: ApiClient): Promise<string> {
  if (relay.moqUrl) return relay.moqUrl;
  const apiUrl = new URL(relay.baseUrl || '/', location.href);
  if (!apiUrl.port) return `moqt://${apiUrl.hostname}`;
  const { bind } = await api.get<{ bind: string }>('/config');
  return `moqt://${apiUrl.hostname}:${bind.slice(bind.lastIndexOf(':') + 1)}`;
}

/** The player's address for a namespace's catalog. The namespace is already in
 *  the `a-b` form the address uses. */
export const catalogUrl = (moq: string, namespace: string) =>
  `${moq.replace(/^[a-z][\w+.-]*:\/\//i, 'moqt://')}#msf:${namespace}--catalog`;

/** Plays a namespace's media from its catalog, when the relay has one. It starts
 *  only on request, since playing subscribes through the relay like any viewer. */
export function Preview(props: { namespace: string }) {
  const { api, relay } = useDashboard();
  const catalog = usePoll(
    signal => api.get<Page<TrackInfo>>('/tracks', { prefix: props.namespace, limit: 100 }, signal),
    30_000,
    [api, props.namespace],
  );
  const [src, setSrc] = useState<string>();
  const [error, setError] = useState<string>();
  const video = useRef<MOQtailVideoElement>(null);

  useEffect(() => {
    setSrc(undefined);
    setError(undefined);
  }, [relay.id, props.namespace]);

  useEffect(() => {
    const element = video.current;
    if (!element) return;
    const onError = () => setError(element.error?.message ?? 'Playback failed');
    element.addEventListener('error', onError);
    // A bad address fails as soon as it is set, before this listener is added
    if (element.error) onError();
    return () => element.removeEventListener('error', onError);
  }, [src]);

  // shortcut: looks among the first 100 tracks under the namespace, page through when namespaces get that crowded
  const hasCatalog = catalog.data?.items.some(
    t => t.namespace === props.namespace && t.name === 'catalog',
  );
  if (!hasCatalog) return null;

  const play = async () => {
    setError(undefined);
    try {
      const url = catalogUrl(await moqUrl(relay, api), props.namespace);
      // The player is loaded only when it is wanted; it is most of the dashboard's size
      (await import('moqtail/player/component')).defineMOQtailVideo();
      setSrc(url);
    } catch (e) {
      setError(`Could not start the player: ${e instanceof Error ? e.message : String(e)}`);
    }
  };

  return (
    <Card
      title="Preview"
      actions={
        <button type="button" class="btn" onClick={src ? () => setSrc(undefined) : play}>
          {src ? 'Stop' : 'Play'}
        </button>
      }
    >
      {src ? (
        <>
          <moqtail-video
            ref={video}
            src={src}
            class="block aspect-video w-full max-w-3xl rounded-lg bg-black"
            controls
            autoplay
            muted
            playsinline
          />
          <p class="mt-2 font-mono text-xs break-all text-neutral-500">{src}</p>
        </>
      ) : (
        <p class="text-sm text-neutral-500">
          This namespace has a catalog. Play subscribes to it through the relay.
        </p>
      )}
      {error && <p class="mt-2 text-sm text-red-400">{error}</p>}
    </Card>
  );
}
