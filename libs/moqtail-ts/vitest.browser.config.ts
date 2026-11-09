import { defineConfig } from 'vitest/config'
import { playwright } from '@vitest/browser-playwright'
import path from 'path'

// Real-browser tests in headless Chrome against the test sources on relay.moqtail.dev (test/browser/harness).
// `npm run test:browser`, about 47 s.

export default defineConfig({
  resolve: {
    alias: {
      '@': path.resolve(__dirname, './src'),
    },
  },
  test: {
    testTimeout: 60_000,
    // Files run side by side, each in its own iframe; its sessions start one at a time.
    fileParallelism: true,
    // Playback over the remote relay stalls at random; a real regression still fails every attempt.
    retry: { count: 4, delay: 2_000 },
    browser: { enabled: true, headless: true, provider: playwright(), instances: [{ browser: 'chromium' }] },
    // VOD fetches as fast as the link allows, which stalls live sessions next to it: it runs after them.
    projects: [
      {
        extends: true,
        test: {
          name: 'live',
          include: ['test/browser/**/*.browser.test.ts'],
          exclude: ['test/browser/vod.browser.test.ts', 'test/browser/component.browser.test.ts'],
          sequence: { groupOrder: 0 },
        },
      },
      {
        extends: true,
        test: { name: 'vod', include: ['test/browser/vod.browser.test.ts'], sequence: { groupOrder: 1 } },
      },
      // Plays live and VOD, so it runs alone
      {
        extends: true,
        test: { name: 'component', include: ['test/browser/component.browser.test.ts'], sequence: { groupOrder: 2 } },
      },
    ],
  },
})
