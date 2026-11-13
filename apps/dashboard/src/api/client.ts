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

import type { ApiErrorBody } from './types';

/** An error answer from the API, or a failure to reach it. */
export class ApiError extends Error {
  readonly status: number;
  readonly code: string;
  readonly detail?: string;

  constructor(status: number, code: string, detail?: string) {
    super(detail ? `${code}: ${detail}` : code);
    this.status = status;
    this.code = code;
    this.detail = detail;
  }
}

export type Query = Record<string, string | number | boolean | undefined>;

/**
 * Calls one relay's API. The token goes in the Authorization header, never in a
 * URL. Streams are read with fetch rather than EventSource, which cannot send that
 * header.
 */
export class ApiClient {
  readonly baseUrl: string;
  private readonly token?: string;

  constructor(baseUrl: string, token?: string) {
    this.baseUrl = baseUrl.replace(/\/+$/, '');
    this.token = token || undefined;
  }

  url(path: string, query?: Query): string {
    const params = new URLSearchParams();
    for (const [key, value] of Object.entries(query ?? {})) {
      if (value !== undefined && value !== '') params.set(key, String(value));
    }
    const search = params.toString();
    return `${this.baseUrl}/api/v1${path}${search ? `?${search}` : ''}`;
  }

  private headers(): HeadersInit {
    return this.token ? { Authorization: `Bearer ${this.token}` } : {};
  }

  private async send(
    path: string,
    query?: Query,
    signal?: AbortSignal,
    init?: RequestInit,
  ): Promise<Response> {
    let response: Response;
    try {
      response = await fetch(this.url(path, query), {
        ...init,
        headers: { ...this.headers(), ...init?.headers },
        signal,
      });
    } catch (e) {
      if (e instanceof DOMException && e.name === 'AbortError') throw e;
      throw new ApiError(0, 'unreachable', `cannot reach ${this.baseUrl}: ${String(e)}`);
    }
    if (!response.ok) {
      let body: ApiErrorBody = { error: `http_${response.status}` };
      try {
        body = (await response.json()) as ApiErrorBody;
      } catch {
        // Not every error carries a JSON body, such as a refused CORS preflight.
      }
      throw new ApiError(response.status, body.error, body.detail);
    }
    return response;
  }

  async get<T>(path: string, query?: Query, signal?: AbortSignal): Promise<T> {
    const response = await this.send(path, query, signal);
    return (await response.json()) as T;
  }

  /** A change: POST, PUT or DELETE, with an optional JSON body. Returns the JSON
   *  answer, or `undefined` when there is none. */
  async write<T = unknown>(
    method: 'POST' | 'PUT' | 'DELETE',
    path: string,
    query?: Query,
    body?: unknown,
  ): Promise<T | undefined> {
    const init: RequestInit =
      body === undefined
        ? { method }
        : { method, body: JSON.stringify(body), headers: { 'Content-Type': 'application/json' } };
    const response = await this.send(path, query, undefined, init);
    const text = await response.text();
    return text ? (JSON.parse(text) as T) : undefined;
  }

  async text(path: string, query?: Query, signal?: AbortSignal): Promise<string> {
    return (await this.send(path, query, signal)).text();
  }

  /** Reads a streaming response, handing each piece of text to `onText`, until it
   *  ends or `signal` aborts. */
  async stream(
    path: string,
    query: Query,
    onText: (text: string) => void,
    signal: AbortSignal,
  ): Promise<void> {
    const response = await this.send(path, query, signal);
    const reader = response.body?.getReader();
    if (!reader) return;
    const decoder = new TextDecoder();
    try {
      for (;;) {
        const { done, value } = await reader.read();
        if (done) break;
        onText(decoder.decode(value, { stream: true }));
      }
    } finally {
      reader.releaseLock();
    }
  }
}
