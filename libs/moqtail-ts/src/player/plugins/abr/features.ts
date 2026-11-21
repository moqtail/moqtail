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

/**
 * The observation contract between the controller and its model: what the controller reads each
 * tick, the features the model sees, and the actions it may take. A model is trained on exactly
 * these, so changing them takes a new model and a new `FEATURES_VERSION`.
 */

/** @public What the controller reads each tick, in seconds, bytes and bits per second. */
export interface Observation {
  /** Buffered ahead of the playhead. */
  forwardBuffer: number
  /** 0 while stalled. */
  playbackRate: number
  /** Time stalled within the last `WINDOW`. */
  stalledTime: number
  /** The selectable bitrates, ascending. */
  ladder: readonly number[]
  /** Index in `ladder` of the rendition last asked for. */
  requested: number
  /** The bitrate of the rendition being received. */
  activeBitrate: number
  /** Live video bytes received within the last `WINDOW`. */
  receivedBytes: number
  /**
   * Live video bytes per second: the slower of the last `RECENT` seconds and its second half, from the
   * last settled switch on. Under trouble, live delivery saturates the link, so this is what it carries.
   */
  throughput: number
  /**
   * Video objects received within the last `WINDOW` over what the frame rate delivers in it: about 1
   * while the link keeps up, whatever the scene costs, and less once it falls behind.
   */
  arrivalRate: number
  /**
   * Link capacity recent probes saw, in bytes per second: the median of the last `CONFIRMING`, so one
   * outlier can't swing it; null before a probe, or once contradicted.
   */
  probeEstimate: number | null
  /** Since the last probe ended with an estimate. */
  probeAge: number | null
  /**
   * The median of the last `CONFIRMING` estimates when all of them are still evidence: each ages only
   * while the controller is free to act, and counts for `UP_EVIDENCE` seconds of that. Null otherwise.
   */
  evidence: number | null
  probing: boolean
  /** Probes in a row that found no room for the next rendition. */
  probeFailures: number
  /** Since the last probe ended; null before one. */
  sinceProbe: number | null
  sinceSwitch: number
  /** Between group starts; null until two are seen. */
  groupInterval: number | null
  /** Whether a recent group is known well enough to plan a probe from. */
  probeable: boolean
  /** A switch is taking effect: the old track still streams, or the new one hasn't arrived. */
  settling: boolean
  /** Since the last switch settled; windowed signals count from then on. */
  sinceSettled: number
  /** Since the last stall or the last time frames fell behind. */
  sinceTrouble: number
}

/** @public What the controller does with a tick. */
export const Action = { Down: 0, Stay: 1, Up: 2, Probe: 3 } as const
/** @public */
export type Action = (typeof Action)[keyof typeof Action]

/** Every windowed signal covers this many seconds. */
export const WINDOW = 2
/** An arrival rate below this, a clear slowdown, means the link no longer carries the rendition. */
export const SHORT = 0.8
/** An arrival rate below this means a probe would only take from recovery. */
export const STARVED = 0.5
/** Seconds of forward buffer below which playback is about to stall. */
const LOW_BUFFER = 0.5
/** Seconds, counted while the controller is free to act, that a probe estimate counts as evidence. */
export const UP_EVIDENCE = 10
/** Probes whose median must show room before a switch up: an odd count, so one outlier can't decide. */
export const CONFIRMING = 3
/** A switch up goes to the highest rendition within this share of the probe's estimate. */
const UP_MARGIN = 0.9
/** A switch down goes to the highest rendition within this share of `throughput`. */
const DOWN_MARGIN = 0.8
/** Seconds of arrivals a switch down measures the link over: recent, yet dozens of objects. */
export const RECENT = 1
/** Seconds without trouble or probes after which the probe backoff starts over. */
const STABLE_RESET = 30
/** Seconds measured after a switch settled before the next decision. */
const SETTLE_MEASURE = 1
/** Seconds after which a switch counts as settled even if the old track's streams never ended. */
export const SETTLE_TIMEOUT = 10
/** Version of the feature list below; a model carries it and the hash of the names. */
export const FEATURES_VERSION = 1
/** The model's inputs, in order. Only their hash travels with a model. */
const FEATURES = [
  'fwd',
  'rate',
  'stall',
  'qpos',
  'gap_up',
  'gap_dn',
  'recv',
  'arrival',
  'probe',
  'probe_age',
  'probe_busy',
  'since_sw',
  'gop',
] as const

/** Seconds before probing again after 1, 2, and 3 or more probes in a row that found no room. */
const PROBE_BACKOFF = [5, 10, 15] as const

/** FNV-1a over the comma-joined feature names. */
export function featuresHash(): number {
  let hash = 0x811c9dc5
  for (const byte of new TextEncoder().encode(FEATURES.join(','))) hash = Math.imul(hash ^ byte, 0x01000193) >>> 0

  return hash
}

/**
 * The model's inputs. Every one is a ratio, a log ratio, or a clipped duration, so none depends on
 * the ladder's bitrates or the link's absolute speed.
 */
export function features(o: Observation): number[] {
  const { ladder, requested: k } = o
  const top = ladder.length - 1
  const bitrate = ladder[k]!

  return [
    Math.log1p(clamp(o.forwardBuffer, 0, 30)),
    o.playbackRate - 1,
    clamp(o.stalledTime / WINDOW, 0, 1),
    top > 0 ? Math.log(bitrate / ladder[0]!) / Math.log(ladder[top]! / ladder[0]!) : 1,
    k < top ? Math.log(ladder[k + 1]! / bitrate) : 0,
    k > 0 ? Math.log(bitrate / ladder[k - 1]!) : 0,
    clamp((o.receivedBytes * 8) / WINDOW / o.activeBitrate, 0, 4),
    clamp(o.arrivalRate, 0, 4),
    o.probeEstimate === null ? 0 : clamp(Math.log((Math.max(o.probeEstimate, 1) * 8) / bitrate), -3, 3),
    o.probeAge === null ? 1 : clamp(o.probeAge / 30, 0, 1),
    o.probing ? 1 : 0,
    clamp(o.sinceSwitch / 10, 0, 1),
    o.groupInterval === null ? 0.25 : clamp(o.groupInterval, 0, 4) / 4,
  ]
}

/**
 * Which actions are allowed, by `Action`. Every switch costs the viewer, so:
 * - only stay while a switch settles and for `SETTLE_MEASURE` after: until then the old track still
 *   shares the link, and what arrives measures neither track;
 * - down only on trouble: a stall, frames arriving under `SHORT` of their rate, or an emptying buffer;
 * - up only when the median of `CONFIRMING` fresh probes shows room for the next rendition;
 * - a probe only below the top, from a known group, with none in flight, while not starved, and once
 *   the backoff after probes that found no room has passed.
 */
export function legalActions(o: Observation): boolean[] {
  if (o.settling || o.sinceSettled < SETTLE_MEASURE) return [false, true, false, false]

  const k = o.requested
  const top = o.ladder.length - 1

  const trouble = o.stalledTime > 0 || o.arrivalRate < SHORT || o.forwardBuffer < LOW_BUFFER
  const room = k < top && o.evidence !== null && o.evidence * 8 * UP_MARGIN >= o.ladder[k + 1]!
  const starved = o.stalledTime > 0 || o.arrivalRate < STARVED
  const calm = o.sinceTrouble >= STABLE_RESET && (o.sinceProbe === null || o.sinceProbe >= STABLE_RESET)
  const backingOff = o.sinceProbe !== null && o.sinceProbe < probeBackoff(calm ? 0 : o.probeFailures)

  // Once probes show room, another one says nothing new
  const canProbe = k < top && o.probeable && !o.probing && !starved && !backingOff && !room

  return [k > 0 && trouble, true, room, canProbe]
}

/** The highest rendition within `UP_MARGIN` of the evidence, at least one up. Only valid when up is allowed. */
export function climbTarget(o: Observation): number {
  const budget = (o.evidence ?? 0) * 8 * UP_MARGIN
  let target = o.requested + 1
  for (let r = target; r < o.ladder.length; r++) if (o.ladder[r]! <= budget) target = r

  return target
}

/**
 * The highest rendition within `DOWN_MARGIN` of `throughput`, at least one down: one switch rather
 * than one per rung, each waiting for the last to settle.
 */
export function downTarget(o: Observation): number {
  const budget = o.throughput * 8 * DOWN_MARGIN
  let target = 0
  for (let r = 0; r < o.requested; r++) if (o.ladder[r]! <= budget) target = r

  return target
}

/** Seconds to wait before the next probe after `failures` probes in a row found no room. */
function probeBackoff(failures: number): number {
  return failures > 0 ? PROBE_BACKOFF[Math.min(failures, PROBE_BACKOFF.length) - 1]! : 0
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(Math.max(value, min), max)
}

if (import.meta.vitest) {
  const { describe, test, expect } = import.meta.vitest

  const observation = (o: Partial<Observation> = {}): Observation => ({
    forwardBuffer: 1,
    playbackRate: 1,
    stalledTime: 0,
    ladder: [1e6, 2e6, 4e6],
    requested: 1,
    activeBitrate: 2e6,
    receivedBytes: 500_000,
    throughput: 250_000,
    arrivalRate: 1,
    probeEstimate: null,
    probeAge: null,
    evidence: null,
    probing: false,
    probeFailures: 0,
    sinceProbe: null,
    sinceSwitch: 10,
    groupInterval: 1,
    probeable: true,
    settling: false,
    sinceSettled: 100,
    sinceTrouble: 100,
    ...o,
  })

  describe('features', () => {
    test('place the rendition on the ladder whatever its bitrates', () => {
      const [, , , qpos, up, down] = features(observation())
      expect(qpos).toBe(0.5)
      expect([up, down]).toEqual([Math.log(2), Math.log(2)])

      const scaled = features(observation({ ladder: [10e6, 20e6, 40e6], activeBitrate: 20e6, receivedBytes: 5e6 }))
      expect(scaled).toEqual(features(observation()))
    })

    test('read no probe as a neutral estimate that is as old as it gets', () => {
      const x = features(observation())
      expect((['probe', 'probe_age', 'probe_busy'] as const).map((name) => x[FEATURES.indexOf(name)])).toEqual([
        0, 1, 0,
      ])
    })
  })

  describe('legalActions', () => {
    test('stay put on a healthy link, and probe only below the top', () => {
      expect(legalActions(observation())).toEqual([false, true, false, true])
      expect(legalActions(observation({ requested: 2 }))).toEqual([false, true, false, false])
    })

    test('hold everything while a switch takes effect, and for a second after', () => {
      const troubled = { arrivalRate: 0.5, evidence: 600_000 }
      expect(legalActions(observation({ ...troubled, settling: true }))).toEqual([false, true, false, false])
      expect(legalActions(observation({ ...troubled, sinceSettled: 0.5 }))).toEqual([false, true, false, false])
      expect(legalActions(observation({ ...troubled, sinceSettled: 1 }))[Action.Down]).toBe(true)
    })

    test('step down only when frames fall behind, after a stall, or on an emptying buffer', () => {
      expect(legalActions(observation({ arrivalRate: 0.85 }))[Action.Down]).toBe(false)
      expect(legalActions(observation({ arrivalRate: 0.75 }))[Action.Down]).toBe(true)
      expect(legalActions(observation({ stalledTime: 0.1 }))[Action.Down]).toBe(true)
      expect(legalActions(observation({ forwardBuffer: 0.4 }))[Action.Down]).toBe(true)
      expect(legalActions(observation({ requested: 0, stalledTime: 1 }))[Action.Down]).toBe(false)
    })

    test('step up only after a recent probe showed room, with a margin, and then probe no more', () => {
      expect(legalActions(observation({ evidence: 600_000 }))[Action.Probe]).toBe(false)
      expect(legalActions(observation({ evidence: 600_000 }))[Action.Up]).toBe(true) // 4.8 Mbps
      expect(legalActions(observation({ evidence: 520_000 }))[Action.Up]).toBe(false) // 4.16 Mbps
      expect(legalActions(observation({ probeEstimate: 600_000 }))[Action.Up]).toBe(false) // unconfirmed
    })

    test('step down to what the link carries, at least one rung, in one switch', () => {
      const ladder = [1e6, 2e6, 4e6, 8e6]
      expect(downTarget(observation({ ladder, requested: 3, throughput: 400_000 }))).toBe(1) // 0.8 x 3.2 Mbps
      expect(downTarget(observation({ ladder, requested: 3, throughput: 2_000_000 }))).toBe(2) // at least one down
      expect(downTarget(observation({ ladder, requested: 3, throughput: 10_000 }))).toBe(0)
    })

    test('climb as high as the estimate allows, in one switch', () => {
      const o = observation({ ladder: [1e6, 2e6, 4e6, 8e6], requested: 0, evidence: 1_000_000 })
      expect(climbTarget(o)).toBe(2) // 0.9 x 8 Mbps fits 4 Mbps, not 8
    })

    test('hold probes while one runs, while starving, or without a group to plan from', () => {
      expect(legalActions(observation({ probing: true }))[Action.Probe]).toBe(false)
      expect(legalActions(observation({ stalledTime: 0.1 }))[Action.Probe]).toBe(false)
      expect(legalActions(observation({ arrivalRate: 0.4 }))[Action.Probe]).toBe(false)
      expect(legalActions(observation({ probeable: false }))[Action.Probe]).toBe(false)
    })

    test('back off longer after each probe that found no room, until a calm stretch', () => {
      expect([0, 1, 2, 3, 9].map(probeBackoff)).toEqual([0, 5, 10, 15, 15])
      expect(legalActions(observation({ probeFailures: 2, sinceProbe: 9 }))[Action.Probe]).toBe(false)
      expect(legalActions(observation({ probeFailures: 2, sinceProbe: 10 }))[Action.Probe]).toBe(true)

      const waiting = { probeFailures: 9, sinceProbe: 14 }
      expect(legalActions(observation({ ...waiting }))[Action.Probe]).toBe(false)
      expect(legalActions(observation({ probeFailures: 9, sinceProbe: 15 }))[Action.Probe]).toBe(true) // the 15 s cap
    })
  })
}
