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

import { Fetch, FetchOk, MoqtObject, RequestError } from '@/model'
import { createLogger } from '../../util/logger'

const logger = createLogger('request/fetch')

// TODO: add timeout mechanism for cancelled requests
// (we cant know how many in-flight objects there are)
export class FetchRequest implements PromiseLike<FetchOk | RequestError> {
  public readonly requestId: bigint
  public readonly message: Fetch
  private _resolve!: (value: FetchOk | RequestError | PromiseLike<FetchOk | RequestError>) => void
  private _reject!: (reason?: any) => void
  private promise: Promise<FetchOk | RequestError>
  public controller?: ReadableStreamDefaultController<MoqtObject>
  public stream: ReadableStream<MoqtObject>
  public isActive: boolean = true
  public isResolved: boolean = false

  constructor(message: Fetch) {
    this.requestId = message.requestId
    this.message = message
    this.stream = new ReadableStream<MoqtObject>({
      start: (controller) => {
        this.controller = controller
      },
    })
    this.promise = new Promise<FetchOk | RequestError>((resolve, reject) => {
      this._resolve = resolve
      this._reject = reject
    })
    logger.debug(`created requestId=${this.requestId}`)
  }

  public resolve(value: FetchOk | RequestError | PromiseLike<FetchOk | RequestError>): void {
    if (value instanceof RequestError) {
      logger.error(
        `resolved with error requestId=${this.requestId} code=${value.errorCode} reason="${value.reasonPhrase.phrase}"`,
      )
    } else if (value instanceof FetchOk) {
      logger.debug(`resolved with OK requestId=${this.requestId}`)
    }
    this._resolve(value)
  }

  public reject(reason?: any): void {
    logger.error(`rejected requestId=${this.requestId}`, reason)
    this._reject(reason)
  }

  public then<TResult1 = FetchOk | RequestError, TResult2 = never>(
    onfulfilled?: ((value: FetchOk | RequestError) => TResult1 | PromiseLike<TResult1>) | undefined | null,
    onrejected?: ((reason: any) => TResult2 | PromiseLike<TResult2>) | undefined | null,
  ): PromiseLike<TResult1 | TResult2> {
    return this.promise.then(onfulfilled, onrejected)
  }

  public catch<TResult = never>(
    onrejected?: ((reason: any) => TResult | PromiseLike<TResult>) | undefined | null,
  ): Promise<FetchOk | RequestError | TResult> {
    return this.promise.catch(onrejected)
  }

  public finally(onfinally?: (() => void) | undefined | null): Promise<FetchOk | RequestError> {
    return this.promise.finally(onfinally)
  }
}
