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

import type { DataEvent } from '@/client/data_event'
import type { PlayerState } from '@/player/state'
import { RECENT, STARVED, WINDOW } from './features'
import type { Arrival, KnownGroup, KnownObject } from './probe'

/** Seconds of history kept. */
const KEEP = 8
/** Seconds of per-second object counts a frame rate is inferred from, when the catalog has none. */
const RATE_HISTORY = 30
/** Group starts the group interval averages over. */
const GROUP_STARTS = 5
/** Complete groups kept for planning probes. */
const KNOWN_GROUPS = 2
/** Seconds a start may take before waiting for it counts as a stall. */
const STARTUP_GRACE = 2
/** The shortest window a rate is measured over, one tick: a settle can shorten the window to it. */
const SHORTEST_WINDOW = 0.5

interface Stall {
  start: number
  /** Infinity while stalled. */
  end: number
}

/**
 * What the controller learns from the player's data events, its states, and the sink's buffer,
 * over the last few seconds. Times are in seconds, from `performance.now()`.
 */
export class Signals {
  /** Video objects received in each whole second, for inferring the frame rate. */
  readonly #perSecond = new Map<number, number>()
  /** The group each video track is receiving, by track name. */
  readonly #receiving = new Map<string, { group: bigint; objects: Map<bigint, number> }>()
  /** Open live streams of the video tracks: stream id → track. */
  readonly #openStreams = new Map<number, string>()
  /** When each video track last delivered an object. */
  readonly #lastObject = new Map<string, number>()
  #videoTracks: ReadonlyMap<string, string> = new Map()
  #video: Arrival[] = []
  /** Every other live object, such as audio: it shares the link. */
  #others: Arrival[] = []
  #firstVideo: number | undefined
  #known: KnownGroup[] = []
  #groupStarts: number[] = []
  #stalls: Stall[] = []
  #autoplayBlocked = false
  /** Windowed signals count from here on: the last settled switch. */
  #measuredFrom = -Infinity

  /** The tracks that carry video, each to the rendition it delivers: only their objects are measured as live media. */
  set videoTracks(heads: ReadonlyMap<string, string>) {
    this.#videoTracks = heads
  }

  /** Stream ids are per client: after a reconnect the old client's streams never end. */
  forgetStreams(): void {
    this.#openStreams.clear()
  }

  /** A data event of a live stream, of `track` once the client knows it. */
  received(event: DataEvent, track: string | undefined): void {
    if (event.kind === 'end') this.#openStreams.delete(event.streamId)
    if (event.kind !== 'object') return

    const arrival: Arrival = { at: event.at / 1000, bytes: event.payloadLength }
    const head = track === undefined ? undefined : this.#videoTracks.get(track)
    if (head === undefined) {
      this.#others.push(arrival)
      return
    }

    this.#video.push(arrival)
    this.#firstVideo ??= arrival.at
    this.#openStreams.set(event.streamId, head)
    this.#lastObject.set(head, arrival.at)
    const second = Math.floor(arrival.at)
    this.#perSecond.set(second, (this.#perSecond.get(second) ?? 0) + 1)

    // Probes are planned from the heads' groups, which the controller FETCHes by the head's name
    if (track !== head) return

    const receiving = this.#receiving.get(track)

    // Each group has its own stream, so on a busy link the next group starts before this one's tail
    // arrives: only a newer group starts one, and a late object completes the group it belongs to
    if (receiving && event.groupId < receiving.group) {
      this.#lateObject(track, event)
      return
    }

    if (receiving?.group !== event.groupId) this.#startGroup(track, event.groupId, arrival.at)
    this.#receiving.get(track)!.objects.set(event.objectId, event.payloadLength)
  }

  /**
   * Stalls by the player's state. Waiting to start counts too, after a grace period: a start the
   * link can't feed would otherwise leave the controller waiting for playback, and playback for it.
   */
  playerState(state: PlayerState, now: number): void {
    if (state === 'playing') this.#autoplayBlocked = false

    const waiting = state === 'stalled' || (state === 'ready' && !this.#autoplayBlocked)
    const current = this.#stalls.at(-1)
    const open = current?.end === Infinity

    if (waiting && !open) this.#stalls.push({ start: state === 'ready' ? now + STARTUP_GRACE : now, end: Infinity })
    if (!waiting && open) current!.end = Math.max(now, current!.start)
  }

  /** The viewer has to press play: waiting for that is not the network's doing. */
  autoplayBlocked(state: PlayerState, now: number): void {
    this.#autoplayBlocked = true
    this.playerState(state, now)
  }

  /** Forgets what is older than any signal looks back. */
  prune(now: number): void {
    const recent = ({ at }: { at: number }) => at > now - KEEP
    this.#video = this.#video.filter(recent)
    this.#others = this.#others.filter(recent)

    for (const second of this.#perSecond.keys()) if (second < now - RATE_HISTORY) this.#perSecond.delete(second)

    this.#stalls = this.#stalls.filter(({ end }) => end > now - KEEP)
  }

  /** Whether `WINDOW` seconds of live video arrived: until then, startup reads as congestion. */
  warmedUp(now: number): boolean {
    return this.#firstVideo !== undefined && now - this.#firstVideo >= WINDOW
  }

  stalledTime(now: number): number {
    const from = now - WINDOW
    return this.#stalls.reduce(
      (sum, { start, end }) => sum + Math.max(0, Math.min(end, now) - Math.max(start, from)),
      0,
    )
  }

  /** Whether `track` still has a live stream open: it hasn't delivered its last group yet. */
  streaming(track: string): boolean {
    return [...this.#openStreams.values()].includes(track)
  }

  /** When `track` last delivered an object; -Infinity before it did. */
  lastObjectAt(track: string): number {
    return this.#lastObject.get(track) ?? -Infinity
  }

  /** Windowed signals count from `now` on: a switch settled, and what came before measured another track. */
  measureFrom(now: number): void {
    this.#measuredFrom = now
  }

  /** Live video bytes received in the measured window, scaled to a whole `WINDOW`. */
  receivedBytes(now: number): number {
    return this.#bytesPerSecond(now, WINDOW) * WINDOW
  }

  /**
   * Video objects received in the measured window over what `frameRate` delivers in it. Without a
   * frame rate from the catalog it uses the rate they usually arrive at, so only a slowdown reads as one.
   */
  arrivalRate(now: number, frameRate: number | undefined): number {
    const { start, length } = this.#window(now, WINDOW)
    const expected = (frameRate ?? this.#usualRate(now)) * length
    const received = this.#video.filter(({ at }) => at > start && at <= now).length

    return expected > 0 ? received / expected : 1
  }

  /**
   * Live video bytes per second, the slower of the last `RECENT` seconds and its second half: a drop
   * shows in the recent half first, and the whole second keeps a short gap between objects from
   * reading as one.
   */
  throughput(now: number): number {
    return Math.min(...[RECENT, RECENT / 2].map((seconds) => this.#bytesPerSecond(now, seconds)))
  }

  /** Frames arriving at under half their rate: the link is far behind. */
  starved(now: number, frameRate: number | undefined): boolean {
    return this.warmedUp(now) && this.arrivalRate(now, frameRate) < STARVED
  }

  /** Mean time between group starts; null until two are seen. */
  groupInterval(): number | null {
    const starts = this.#groupStarts
    return starts.length > 1 ? (starts.at(-1)! - starts[0]!) / (starts.length - 1) : null
  }

  /** The last one or two complete groups of the track that completed one last, oldest first. */
  knownGroups(): KnownGroup[] {
    const last = this.#known.at(-1)
    return last ? this.#known.slice(-2).filter(({ track }) => track === last.track) : []
  }

  /** Every live object, video and others, for measuring a probe against. */
  arrivals(): Arrival[] {
    return [...this.#video, ...this.#others]
  }

  /** The last `seconds`, from the last settled switch on, and its length, at least `SHORTEST_WINDOW`. */
  #window(now: number, seconds: number): { start: number; length: number } {
    const start = Math.max(now - seconds, this.#measuredFrom)
    return { start, length: Math.max(now - start, SHORTEST_WINDOW) }
  }

  #bytesPerSecond(now: number, seconds: number): number {
    const { start, length } = this.#window(now, seconds)
    return this.#video.reduce((sum, { at, bytes }) => (at > start && at <= now ? sum + bytes : sum), 0) / length
  }

  /** A new group closes the one the track was receiving: its objects' sizes are now known. */
  #startGroup(track: string, group: bigint, at: number): void {
    const previous = this.#receiving.get(track)
    if (previous) {
      const objects: KnownObject[] = [...previous.objects].map(([id, bytes]) => ({ id, bytes })).sort(byId)
      this.#known = [...this.#known, { track, group: previous.group, objects }].slice(-KNOWN_GROUPS)
    }

    this.#receiving.set(track, { group, objects: new Map() })
    this.#groupStarts = [...this.#groupStarts, at].slice(-GROUP_STARTS)
  }

  /** An object of a group a newer one already closed: added to it, if it is still known. */
  #lateObject(track: string, { groupId, objectId, payloadLength }: DataEvent & { kind: 'object' }): void {
    this.#known = this.#known.map((known) =>
      known.track === track && known.group === groupId
        ? { ...known, objects: [...known.objects, { id: objectId, bytes: payloadLength }].sort(byId) }
        : known,
    )
  }

  /**
   * Objects per second in a typical whole second of the last `RATE_HISTORY`: the 75th percentile,
   * so a congested stretch doesn't drag it down and a catch-up burst doesn't lift it.
   */
  #usualRate(now: number): number {
    const counts = [...this.#perSecond]
      .filter(([second]) => second < Math.floor(now)) // the current second is still filling
      .map(([, count]) => count)
      .sort((a, b) => a - b)
    return counts.length > 0 ? counts[Math.floor((counts.length - 1) * 0.75)]! : 0
  }
}

function byId(a: KnownObject, b: KnownObject): number {
  return a.id < b.id ? -1 : 1
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest

  const object = (groupId: number, objectId: number, at: number, payloadLength = 1000): DataEvent => ({
    kind: 'object',
    direction: 'in',
    streamId: 1,
    at: at * 1000,
    groupId: BigInt(groupId),
    objectId: BigInt(objectId),
    payloadLength,
  })

  const tracking = () => {
    const signals = new Signals()
    signals.videoTracks = new Map([['v', 'v']])
    return signals
  }

  describe('Signals', () => {
    test('counts video in the window, and other tracks apart', () => {
      const signals = tracking()
      signals.received(object(1, 0, 0.5), 'v')
      signals.received(object(1, 1, 1.5), 'v')
      signals.received(object(1, 0, 1.5), 'audio')

      expect(signals.receivedBytes(2.6)).toBe(1000) // 0.5 is out of the window
      expect(signals.arrivals()).toHaveLength(3)
      expect(signals.warmedUp(2.4)).toBe(false)
      expect(signals.warmedUp(2.5)).toBe(true)
    })

    test('knows a group once the next one starts, and times group starts', () => {
      const signals = tracking()
      signals.received(object(1, 1, 0, 300), 'v')
      signals.received(object(1, 0, 0.1, 500), 'v')

      expect(signals.knownGroups()).toEqual([])

      signals.received(object(2, 0, 1), 'v')
      signals.received(object(3, 0, 2), 'v')

      expect(signals.knownGroups().map((g) => [g.group, g.objects.map((o) => [o.id, o.bytes])])).toEqual([
        [
          1n,
          [
            [0n, 500],
            [1n, 300],
          ],
        ],
        [2n, [[0n, 1000]]],
      ])
      expect(signals.groupInterval()).toBe(1)
    })

    test('keeps groups whole and in order when their streams interleave', () => {
      const signals = tracking()
      signals.received(object(1, 0, 0), 'v')
      signals.received(object(2, 0, 1), 'v')
      signals.received(object(1, 1, 1.1), 'v') // the tail of group 1, after group 2 started
      signals.received(object(2, 1, 1.2), 'v')
      signals.received(object(3, 0, 2), 'v')

      expect(signals.knownGroups().map((g) => [g.group, g.objects.map((o) => o.id)])).toEqual([
        [1n, [0n, 1n]],
        [2n, [0n, 1n]],
      ])
      expect(signals.groupInterval()).toBe(1)

      expect(signals.receivedBytes(2)).toBe(4000) // the late object still counts as delivered
    })

    test('counts a start that takes longer than the grace period as a stall, unless autoplay is blocked', () => {
      const signals = tracking()
      signals.playerState('ready', 0)
      expect(signals.stalledTime(2)).toBe(0)
      expect(signals.stalledTime(3)).toBe(1)

      signals.playerState('playing', 3)
      expect(signals.stalledTime(4)).toBe(1)

      const blocked = tracking()
      blocked.playerState('ready', 0)
      blocked.autoplayBlocked('ready', 0.1)
      expect(blocked.stalledTime(5)).toBe(0)
    })

    test("knows a track's open streams until they end, and when it last delivered", () => {
      const signals = tracking()
      signals.received(object(1, 0, 1), 'v')
      expect(signals.streaming('v')).toBe(true)
      expect(signals.lastObjectAt('v')).toBe(1)

      signals.received({ kind: 'end', direction: 'in', streamId: 1, at: 1100, reason: 'fin' }, 'v')
      expect(signals.streaming('v')).toBe(false)
    })

    test("counts a short GoP of a ladder as its head's video, but plans probes from the head only", () => {
      const signals = new Signals()
      signals.videoTracks = new Map([
        ['v60', 'v60'],
        ['v15', 'v60'],
      ])

      signals.received(object(1, 0, 1), 'v15')
      signals.received(object(2, 0, 1.5), 'v15')

      expect(signals.receivedBytes(2)).toBe(2000)
      expect([signals.streaming('v60'), signals.lastObjectAt('v60')]).toEqual([true, 1.5])
      expect(signals.groupInterval()).toBeNull()

      signals.forgetStreams()
      expect(signals.streaming('v60')).toBe(false)
    })

    test('reads throughput from the slower of the last second and its second half', () => {
      const signals = tracking()
      for (let t = 0; t < 1.5; t += 0.05) signals.received(object(1, 0, t, 1000), 'v') // 20 kB/s

      for (let t = 1.5; t < 2; t += 0.1) signals.received(object(1, 0, t, 1000), 'v') // then 10 kB/s

      expect(signals.throughput(1.99)).toBeCloseTo(10_000, -3)
    })

    test('measures from the last settle on, over the time since', () => {
      const signals = tracking()
      for (let t = 0; t < 3; t += 0.05) signals.received(object(1, 0, t, 100), 'v') // 20 a second

      signals.measureFrom(2)

      expect(signals.arrivalRate(2.99, 20)).toBeCloseTo(1, 1)
      expect(signals.receivedBytes(2.99)).toBeCloseTo(20 * 100 * 2, -2) // per second, scaled to the window
    })

    test('measures frames against the catalog frame rate, else against the usual rate', () => {
      const signals = tracking()

      // 20 objects a second for 10 s, then 8 a second
      for (let t = 0; t < 10; t += 0.05) signals.received(object(1, 0, t), 'v')
      for (let t = 10; t < 12; t += 0.125) signals.received(object(1, 0, t), 'v')

      expect(signals.arrivalRate(9.99, 20)).toBeCloseTo(1, 1)
      expect(signals.arrivalRate(11.99, 20)).toBeCloseTo(0.4, 1)
      expect(signals.arrivalRate(11.99, undefined)).toBeCloseTo(0.4, 1) // usual rate inferred: 20
      expect(signals.starved(11.99, 20)).toBe(true)
    })
  })
}
