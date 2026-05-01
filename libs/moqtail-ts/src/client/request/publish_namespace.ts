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

import { RequestOk, SubscribeError } from '../../model/control'
import { SubscribeErrorCode } from '../../model/control/constant'
import { PublishNamespace } from '../../model/control/publish_namespace'
import { ReasonPhrase } from '../../model/common/reason_phrase'

// TODO: add publish namespace done
export class PublishNamespaceRequest implements PromiseLike<RequestOk | SubscribeError> {
  public readonly requestId: bigint
  public readonly message: PublishNamespace
  private _resolve!: (value: RequestOk | SubscribeError | PromiseLike<RequestOk | SubscribeError>) => void
  private _reject!: (reason?: any) => void
  private promise: Promise<RequestOk | SubscribeError>

  constructor(requestId: bigint, message: PublishNamespace) {
    this.requestId = requestId
    this.message = message
    this.promise = new Promise<RequestOk | SubscribeError>((resolve, reject) => {
      this._resolve = resolve
      this._reject = reject
    })
  }

  public resolve(value: RequestOk | SubscribeError | PromiseLike<RequestOk | SubscribeError>): void {
    this._resolve(value)
  }

  public reject(reason?: any): void {
    this._reject(reason)
  }

  public then<TResult1 = RequestOk | SubscribeError, TResult2 = never>(
    onfulfilled?: ((value: RequestOk | SubscribeError) => TResult1 | PromiseLike<TResult1>) | undefined | null,
    onrejected?: ((reason: any) => TResult2 | PromiseLike<TResult2>) | undefined | null,
  ): PromiseLike<TResult1 | TResult2> {
    return this.promise.then(onfulfilled, onrejected)
  }

  public catch<TResult = never>(
    onrejected?: ((reason: any) => TResult | PromiseLike<TResult>) | undefined | null,
  ): Promise<RequestOk | SubscribeError | TResult> {
    return this.promise.catch(onrejected)
  }

  public finally(onfinally?: (() => void) | undefined | null): Promise<RequestOk | SubscribeError> {
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

    test('should resolve with SubscribeError on protocol error', async () => {
      const announceMessage = {} as PublishNamespace
      const request = new PublishNamespaceRequest(123n, announceMessage)
      const announceError = new SubscribeError(123n, SubscribeErrorCode.InternalError, new ReasonPhrase('wololo'))
      setTimeout(() => request.resolve(announceError), 0)
      const result = await request
      expect(result).toBeInstanceOf(SubscribeError)
      expect(result.requestId).toBe(123n)
      if (result instanceof SubscribeError) {
        expect(result.errorCode).toBe(SubscribeErrorCode.InternalError)
      } else {
        throw new Error('Expected SubscribeError')
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
      const announceError = new SubscribeError(
        789n,
        SubscribeErrorCode.MalformedAuthToken,
        new ReasonPhrase('bad token'),
      )
      setTimeout(() => request.resolve(announceError), 10)
      const result = await request
      expect(result).toBeInstanceOf(SubscribeError)
      expect(result.requestId).toBe(789n)
      if (result instanceof SubscribeError) {
        expect(result.errorCode).toBe(SubscribeErrorCode.MalformedAuthToken)
      } else {
        throw new Error('Expected SubscribeError')
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
