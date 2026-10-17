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

const DEFAULT_SAMPLE_SIZE = 5
const DEFAULT_TIME_SERVER = 'https://time.akamai.com/?ms'

export class ClockNormalizer {
  private offset: number
  private timeServerUrl: string
  private numSamples: number

  private constructor(timeServerUrl: string, offset: number, numSamples: number) {
    this.timeServerUrl = timeServerUrl
    this.offset = offset
    this.numSamples = numSamples
  }

  public static async create(timeServerUrl?: string, numberOfSamples?: number): Promise<ClockNormalizer> {
    const url = timeServerUrl ? timeServerUrl : DEFAULT_TIME_SERVER
    const numSamples = numberOfSamples ? numberOfSamples : DEFAULT_SAMPLE_SIZE
    const offset = await ClockNormalizer.calculateSkew(url, numSamples)
    return new ClockNormalizer(url, offset, numSamples)
  }

  public getSkew(): number {
    return this.offset
  }

  public now(): number {
    return Date.now() - this.offset
  }

  public async recalibrate(): Promise<number> {
    this.offset = await ClockNormalizer.calculateSkew(this.timeServerUrl, this.numSamples)
    return this.offset
  }

  private static async calculateSkew(timeServerUrl: string, numSamples: number): Promise<number> {
    const samples: Array<{ offset: number; rtt: number }> = []

    for (let i = 0; i < numSamples; i++) {
      const sample = await ClockNormalizer.takeSingleSample(timeServerUrl)
      if (sample !== null) {
        samples.push(sample)
      }

      // Small delay between samples to avoid overwhelming the server
      if (i < numSamples - 1) {
        await new Promise((resolve) => setTimeout(resolve, 20))
      }
    }

    if (samples.length === 0) {
      throw new Error('Failed to get any valid samples')
    }

    const offsets = samples.map((s) => s.offset)
    const average = offsets.reduce((sum, offset) => sum + offset, 0) / offsets.length

    return Math.round(average)
  }

  private static async takeSingleSample(timeServerUrl: string): Promise<{ offset: number; rtt: number } | null> {
    const separator = timeServerUrl.includes('?') ? '&' : '?'
    const url = `${timeServerUrl}${separator}_=${Date.now()}`

    const t0 = performance.now()
    const localTimeBeforeRequest = Date.now()
    const response = await fetch(url, { cache: 'no-store' })
    const serverTimeText = await response.text()
    const t1 = performance.now()
    const localTimeAfterRequest = Date.now()

    const serverTime = parseFloat(serverTimeText)
    if (isNaN(serverTime)) {
      return null
    }

    // Convert server time from seconds to milliseconds
    // Servers like Akamai returns time as sssssssss.mmm but local clock returns only decimals
    const serverTimeMs = serverTime * 1000

    const rtt = t1 - t0
    const oneWayDelay = rtt / 2

    const avgLocalTime = (localTimeBeforeRequest + localTimeAfterRequest) / 2
    const localTimeAtServer = avgLocalTime - oneWayDelay

    const offset = localTimeAtServer - serverTimeMs

    return { offset, rtt }
  }
}
