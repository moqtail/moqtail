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

import { useEffect, useState } from 'preact/hooks';

// Pages are addressed by the URL hash, such as `#/connections/42`, so links can be
// shared and the browser's back button works without a router.

export interface Route {
  page: string;
  id?: string;
}

function parse(hash: string): Route {
  const [page = 'overview', id] = hash.replace(/^#\/?/, '').split('/');
  return { page: page || 'overview', id: id ? decodeURIComponent(id) : undefined };
}

export function useRoute(): Route {
  const [route, setRoute] = useState(() => parse(location.hash));
  useEffect(() => {
    const update = () => setRoute(parse(location.hash));
    addEventListener('hashchange', update);
    return () => removeEventListener('hashchange', update);
  }, []);
  return route;
}

export function href(page: string, id?: string | number): string {
  return id === undefined ? `#/${page}` : `#/${page}/${encodeURIComponent(String(id))}`;
}
