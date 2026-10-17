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

// Type alias for a function that continues execution (used for queued lock requests)
type Continuation = () => void

// A simple async lock implementation for mutual exclusion
export class SimpleLock {
  // Indicates if the lock is currently held
  private acquiredLock = false

  // Queue of waiting continuations (resolvers for pending acquire calls)
  private readonly contQueue: Continuation[] = []

  /**
   * Acquires the lock. If the lock is already held, waits until it is released.
   */
  public async acquire(): Promise<void> {
    if (!this.acquiredLock) {
      // Lock is free, acquire immediately
      this.acquiredLock = true
    } else {
      // Lock is held, queue the continuation
      return new Promise<void>((resolve, _) => {
        this.contQueue.push(resolve)
      })
    }
  }

  /**
   * Releases the lock. If there are queued waiters, resumes the next one.
   */
  public async release(): Promise<void> {
    if (this.contQueue.length === 0 && this.acquiredLock) {
      // No waiters, simply release the lock
      this.acquiredLock = false
      return
    }

    // There are waiters, resume the next one in the queue
    const continuation = this.contQueue.shift()
    return new Promise((res: Continuation) => {
      continuation!() // Resume the next waiting acquire
      res()
    })
  }
}
