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

import type { ObjectOrderer } from '@/client/ordering'
import type { EndOfRangeHandler } from '@/client/types'
import { ObjectStream } from '@/client/util/object_stream'
import type { Fetch, FetchOk, RequestError } from '@/model/messages'
import { PendingRequest } from './pending'

// TODO: add timeout mechanism for cancelled requests
// (we cant know how many in-flight objects there are)
export class FetchRequest extends PendingRequest<Fetch, FetchOk | RequestError> {
  readonly objects = new ObjectStream()
  readonly stream = this.objects.readable
  onEndOfRange: EndOfRangeHandler | undefined
  orderer: ObjectOrderer | undefined
}
