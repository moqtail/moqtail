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

import { SubscribeNamespace, RequestError, RequestOk } from '@/model'
import { logger } from '../../util/logger'

export class SubscribeNamespaceRequest implements PromiseLike<RequestOk | RequestError> {
  public readonly requestId: bigint
  public readonly message: SubscribeNamespace
  private _resolve!: (value: RequestOk | RequestError | PromiseLike<RequestOk | RequestError>) => void
  private _reject!: (reason?: any) => void
  private promise: Promise<RequestOk | RequestError>

  constructor(msg: SubscribeNamespace) {
    this.requestId = msg.requestId
    this.message = msg
    this.promise = new Promise<RequestOk | RequestError>((resolve, reject) => {
      this._resolve = resolve
      this._reject = reject
    })
    logger.debug(
      'request/subscribe_namespace',
      `created requestId=${this.requestId} namespace="${msg.trackNamespacePrefix}"`,
    )
  }

  public resolve(value: RequestOk | RequestError | PromiseLike<RequestOk | RequestError>): void {
    if (value instanceof RequestError) {
      logger.error(
        'request/subscribe_namespace',
        `resolved with error requestId=${this.requestId} code=${value.errorCode} reason="${value.reasonPhrase.phrase}"`,
      )
    } else if (value instanceof RequestOk) {
      logger.debug('request/subscribe_namespace', `resolved with OK requestId=${this.requestId}`)
    }
    this._resolve(value)
  }

  public reject(reason?: any): void {
    logger.error('request/subscribe_namespace', `rejected requestId=${this.requestId}`, reason)
    this._reject(reason)
  }

  public then<TResult1 = RequestOk | RequestError, TResult2 = never>(
    onfulfilled?: ((value: RequestOk | RequestError) => TResult1 | PromiseLike<TResult1>) | undefined | null,
    onrejected?: ((reason: any) => TResult2 | PromiseLike<TResult2>) | undefined | null,
  ): PromiseLike<TResult1 | TResult2> {
    return this.promise.then(onfulfilled, onrejected)
  }

  public catch<TResult = never>(
    onrejected?: ((reason: any) => TResult | PromiseLike<TResult>) | undefined | null,
  ): Promise<RequestOk | RequestError | TResult> {
    return this.promise.catch(onrejected)
  }

  public finally(onfinally?: (() => void) | undefined | null): Promise<RequestOk | RequestError> {
    return this.promise.finally(onfinally)
  }
}
