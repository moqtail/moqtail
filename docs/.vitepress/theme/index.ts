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

/// <reference types="vite/client" />
import type { Theme } from 'vitepress'
// Without the bundled Inter: the fonts are self-hosted through fontsource.
import DefaultTheme from 'vitepress/theme-without-fonts'
import CopyOrDownloadAsMarkdownButtons from 'vitepress-plugin-llms/vitepress-components/CopyOrDownloadAsMarkdownButtons.vue'
import '@fontsource-variable/source-sans-3'
import '@fontsource-variable/geist-mono'
import './style.css'
import './aurora.css'
import Mermaid from './Mermaid.vue'

export default {
  extends: DefaultTheme,
  enhanceApp({ app }) {
    app.component('CopyOrDownloadAsMarkdownButtons', CopyOrDownloadAsMarkdownButtons)
    app.component('Mermaid', Mermaid)
    if (!import.meta.env.SSR) void startAnalytics()
  },
} satisfies Theme

// Only in builds that set both; CI takes them from the POSTHOG_KEY and POSTHOG_HOST secrets.
async function startAnalytics() {
  const key = import.meta.env.VITE_POSTHOG_KEY
  const host = import.meta.env.VITE_POSTHOG_HOST
  if (!key || !host) {
    return
  }
  const { default: posthog } = await import('posthog-js')
  // No cookies or browser storage, so no consent banner
  posthog.init(key, { api_host: host, cookieless_mode: 'always', capture_pageview: 'history_change' })
}
