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

/**
 * MMTP receiver path (draft-ramadan-moq-mmt): each MoQ object on a track whose
 * catalog packaging is `mmtp` carries exactly one MMTP packet. This module
 * turns those objects back into MFUs (Media Fragment Units).
 *
 * Packet parsing and MFU reassembly are delegated to `@blockcast/mmt-container`,
 * an optional peer dependency: install it to use `moqtail/mmtp`. Nothing else in
 * moqtail imports it.
 */
export { MmtpTrackReceiver, type MmtpReceiverOptions, type MmtpReceiverStats } from './receiver'
export type { ReassembledMFU } from '@blockcast/mmt-container/mfu'
