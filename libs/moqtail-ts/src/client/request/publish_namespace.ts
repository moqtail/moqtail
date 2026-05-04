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

import { RequestOk, RequestError } from '../../model/control'
import { RequestErrorCode } from '../../model/control/constant'
import { PublishNamespace } from '../../model/control/publish_namespace'
import { ReasonPhrase } from '../../model/common/reason_phrase'
import { logger } from '../../util/logger'

// TODO: add publish namespace done
export class PublishNamespaceRequest implements PromiseLike<RequestOk | RequestError> {
  public readonly requestId: bigint
  public readonly message: PublishNamespace
  private _resolve!: (value: RequestOk | RequestError | PromiseLike<RequestOk | RequestError>) => void
  private _reject!: (reason?: any) => void
  private promise: Promise<RequestOk | RequestError>

  constructor(requestId: bigint, message: PublishNamespace) {
    this.requestId = requestId
    this.message = message
    this.promise = new Promise<RequestOk | RequestError>((resolve, reject) => {
      this._resolve = resolve
      this._reject = reject
    })
    logger.debug(
      'request/publish_namespace',
      `created requestId=${this.requestId} namespace="${message.trackNamespace}"`,
    )
  }

  public resolve(value: RequestOk | RequestError | PromiseLike<RequestOk | RequestError>): void {
    if (value instanceof RequestError) {
      logger.error(
        'request/publish_namespace',
        `resolved with error requestId=${this.requestId} code=${value.errorCode} reason="${value.reasonPhrase.phrase}"`,
      )
    } else if (value instanceof RequestOk) {
      logger.debug('request/publish_namespace', `resolved with OK requestId=${this.requestId}`)
    }
    this._resolve(value)
  }

  public reject(reason?: any): void {
    logger.error('request/publish_namespace', `rejected requestId=${this.requestId}`, reason)
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

if (import.meta.vitest) {
  const { describe, test, expect, vi } = import.meta.vitest

  describe('PublishNamespaceRequest', () => {
    test('should resolve with RequestOk on success', async () => {
      const announceMessage = {} as PublishNamespace
      const request = new PublishNamespaceRequest(123n, announceMessage)
      const announceOkResponse = new RequestOk(123n)
      setTimeout(() => request.resolve(announceOkResponse), 0)
      const result = await request
      expect(result).toBeInstanceOf(RequestOk)
      expect(result.requestId).toBe(123n)
    })

    test('should resolve with RequestError on protocol error', async () => {
      const announceMessage = {} as PublishNamespace
      const request = new PublishNamespaceRequest(123n, announceMessage)
      const announceError = new RequestError(123n, RequestErrorCode.InternalError, 0n, new ReasonPhrase('wololo'))
      setTimeout(() => request.resolve(announceError), 0)
      const result = await request
      expect(result).toBeInstanceOf(RequestError)
      expect(result.requestId).toBe(123n)
      if (result instanceof RequestError) {
        expect(result.errorCode).toBe(RequestErrorCode.InternalError)
      } else {
        throw new Error('Expected RequestError')
      }
    })

    test('should reject on exception', async () => {
      const announceMessage = {} as PublishNamespace
      const request = new PublishNamespaceRequest(123n, announceMessage)
      const error = new Error('Network failure')
      setTimeout(() => request.reject(error), 0)
      await expect(request).rejects.toBe(error)
    })

    test('can be used with async/await for success', async () => {
      const announceMessage = {} as PublishNamespace
      const request = new PublishNamespaceRequest(456n, announceMessage)
      const announceOkResponse = new RequestOk(456n)
      setTimeout(() => request.resolve(announceOkResponse), 10)
      const result = await request
      expect(result).toBeInstanceOf(RequestOk)
      expect(result.requestId).toBe(456n)
    })

    test('can be used with async/await for protocol error', async () => {
      const announceMessage = {} as PublishNamespace
      const request = new PublishNamespaceRequest(789n, announceMessage)
      const announceError = new RequestError(
        789n,
        RequestErrorCode.MalformedAuthToken,
        0n,
        new ReasonPhrase('bad token'),
      )
      setTimeout(() => request.resolve(announceError), 10)
      const result = await request
      expect(result).toBeInstanceOf(RequestError)
      expect(result.requestId).toBe(789n)
      if (result instanceof RequestError) {
        expect(result.errorCode).toBe(RequestErrorCode.MalformedAuthToken)
      } else {
        throw new Error('Expected RequestError')
      }
    })

    test('finally block is executed on resolve', async () => {
      const announceMessage = {} as PublishNamespace
      const request = new PublishNamespaceRequest(111n, announceMessage)
      const announceOkResponse = new RequestOk(111n)
      const finallyCallback = vi.fn()
      setTimeout(() => request.resolve(announceOkResponse), 0)
      await request.finally(finallyCallback)
      expect(finallyCallback).toHaveBeenCalledTimes(1)
    })

    test('finally block is executed on reject', async () => {
      const announceMessage = {} as PublishNamespace
      const request = new PublishNamespaceRequest(222n, announceMessage)
      const error = new Error('timeout')
      const finallyCallback = vi.fn()
      setTimeout(() => request.reject(error), 0)
      try {
        await request.finally(finallyCallback)
      } catch (e) {
        // Expected rejection
      }
      expect(finallyCallback).toHaveBeenCalledTimes(1)
    })
  })
}
