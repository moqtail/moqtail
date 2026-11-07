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

import type { CatalogTrack, PlayerState } from 'moqtail/player';

export type Track = CatalogTrack;
export type Status = PlayerState;

export interface PresetNamespace {
  name: string;
  label: string;
  videoTracks: number;
  audioTracks: number;
}

export interface PresetRelay {
  url: string;
  namespaces: PresetNamespace[];
}

export interface Presets {
  relays: PresetRelay[];
}

export type PublishStatus = 'idle' | 'connecting' | 'publishing' | 'error';
export type SourceKind = 'camera' | 'screen' | 'test';

export interface SourceState {
  kind: SourceKind;
  enabled: boolean;
  available: boolean;
  unavailableReason?: string;
  embedTimestamp: boolean;
  stream?: MediaStream;
}
