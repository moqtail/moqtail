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

import { Publish, PublishOk, RequestError } from '@/model'
import { createLogger } from '../../util/logger'

const logger = createLogger('request/publish')

export class PublishRequest implements PromiseLike<PublishOk | RequestError> {
  public readonly requestId: bigint
  public readonly message: Publish
  private _resolve!: (value: PublishOk | RequestError | PromiseLike<PublishOk | RequestError>) => void
  private _reject!: (reason?: any) => void
  private promise: Promise<PublishOk | RequestError>

  constructor(msg: Publish) {
    this.requestId = msg.requestId
    this.message = msg
    this.promise = new Promise<PublishOk | RequestError>((resolve, reject) => {
      this._resolve = resolve
      this._reject = reject
    })
    logger.debug(`created requestId=${this.requestId} ftn="${msg.fullTrackName}"`)
  }

  public resolve(value: PublishOk | RequestError | PromiseLike<PublishOk | RequestError>): void {
    if (value instanceof RequestError) {
      logger.error(
        `resolved with error requestId=${this.requestId} code=${value.errorCode} reason="${value.reasonPhrase.phrase}"`,
      )
    } else if (value instanceof PublishOk) {
      logger.debug(`resolved with OK requestId=${this.requestId}`)
    }
    this._resolve(value)
  }

  public reject(reason?: any): void {
    logger.error(`rejected requestId=${this.requestId}`, reason)
    this._reject(reason)
  }

  public then<TResult1 = PublishOk | RequestError, TResult2 = never>(
    onfulfilled?: ((value: PublishOk | RequestError) => TResult1 | PromiseLike<TResult1>) | undefined | null,
    onrejected?: ((reason: any) => TResult2 | PromiseLike<TResult2>) | undefined | null,
  ): PromiseLike<TResult1 | TResult2> {
    return this.promise.then(onfulfilled, onrejected)
  }

  public catch<TResult = never>(
    onrejected?: ((reason: any) => TResult | PromiseLike<TResult>) | undefined | null,
  ): Promise<PublishOk | RequestError | TResult> {
    return this.promise.catch(onrejected)
  }

  public finally(onfinally?: (() => void) | undefined | null): Promise<PublishOk | RequestError> {
    return this.promise.finally(onfinally)
  }
}
