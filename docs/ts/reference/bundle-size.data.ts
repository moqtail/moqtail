import { readFileSync } from 'node:fs'
import { defineLoader } from 'vitepress'

const FILE = '../../../libs/moqtail-ts/build-info.json'

interface Size {
  min: number
  gzip: number
}

export interface BuildInfo {
  version: string
  entries: { name: string; from: string; initial: Size }[]
  additions: { name: string; from: string; gzip: number }[]
  lazy: { name: string; gzip: number }[]
}

declare const data: BuildInfo
export { data }

export default defineLoader({
  watch: [FILE],
  load([file]): BuildInfo {
    if (!file) throw new Error('No build-info.json: run `npm --prefix libs/moqtail-ts run transpile` first')
    return JSON.parse(readFileSync(file, 'utf8'))
  },
})
