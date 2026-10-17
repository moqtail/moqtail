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

import { DataEventObserver, MOQtailClient } from 'moqtail/client';
import { ControlMessage } from 'moqtail/model';

export type SessionObserver = {
  onMessageSent?: (msg: ControlMessage) => void;
  onMessageReceived?: (msg: ControlMessage) => void;
  onDataEvent?: DataEventObserver;
};

export async function createMOQtailClient(
  relayUrl?: string,
  observer: SessionObserver = {},
): Promise<MOQtailClient> {
  return await MOQtailClient.new({
    url: relayUrl ?? window.appSettings.relayUrl,
    transportOptions: {},
    enableDatagrams: false,
    // Passed at connect time so SETUP, the first message each way, is observed too.
    callbacks: observer,
  });
}
