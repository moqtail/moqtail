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

import { useEffect, useRef, useState } from 'preact/hooks';
import { ApiError } from '../api/client';

export interface Polled<T> {
  data?: T;
  error?: ApiError;
  /** When the data was last loaded, in milliseconds since the epoch. */
  loadedAt?: number;
}

/**
 * Loads `load` now and then every `intervalMs`, while the browser tab is visible.
 * A hidden tab does not poll, so a forgotten dashboard costs the relay nothing.
 * The load starts over when `deps` change.
 */
export function usePoll<T>(
  load: (signal: AbortSignal) => Promise<T>,
  intervalMs: number,
  deps: unknown[],
): Polled<T> {
  const [state, setState] = useState<Polled<T>>({});
  const loadRef = useRef(load);
  loadRef.current = load;

  useEffect(() => {
    setState({});
    let timer: ReturnType<typeof setTimeout> | undefined;
    let controller = new AbortController();
    let stopped = false;

    const tick = async () => {
      if (stopped) return;
      if (document.visibilityState === 'visible') {
        controller = new AbortController();
        try {
          const data = await loadRef.current(controller.signal);
          if (!stopped) setState({ data, loadedAt: Date.now() });
        } catch (e) {
          if (!stopped && !(e instanceof DOMException && e.name === 'AbortError')) {
            const error = e instanceof ApiError ? e : new ApiError(0, 'error', String(e));
            setState(previous => ({ ...previous, error }));
          }
        }
      }
      if (!stopped) timer = setTimeout(tick, intervalMs);
    };
    const onVisible = () => {
      if (document.visibilityState === 'visible') {
        clearTimeout(timer);
        tick();
      }
    };
    tick();
    document.addEventListener('visibilitychange', onVisible);
    return () => {
      stopped = true;
      clearTimeout(timer);
      controller.abort();
      document.removeEventListener('visibilitychange', onVisible);
    };
  }, [intervalMs, ...deps]);

  return state;
}
