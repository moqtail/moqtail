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

import { createContext } from 'preact';
import { useContext } from 'preact/hooks';
import type { ApiClient } from './api/client';
import type { ApiIndex } from './api/types';
import type { Relay } from './lib/relays';

export interface Dashboard {
  relay: Relay;
  /** Reads, with the read token when one is set. */
  api: ApiClient;
  /** Writes, with the write token; `undefined` when none is set. */
  writeApi?: ApiClient;
  refreshMs: number;
  /** What the relay serves; `undefined` until it has answered. */
  index?: ApiIndex;
}

export const DashboardContext = createContext<Dashboard | null>(null);

export function useDashboard(): Dashboard {
  const value = useContext(DashboardContext);
  if (!value) throw new Error('useDashboard outside the dashboard');
  return value;
}

/** Whether the relay serves a feature; unknown counts as yes until it answers. */
export function hasFeature(dashboard: Dashboard, feature: string): boolean {
  return !dashboard.index || dashboard.index.features.includes(feature);
}
