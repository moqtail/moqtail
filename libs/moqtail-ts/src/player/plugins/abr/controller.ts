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

import type { MediaSinkElement, MediaSourceElement, Rendition } from '@/player/element/interfaces'
import { rangeAt } from '@/player/elements/mse/gaps'
import { eachClient, followData } from '@/player/follow'
import { parseMsfUrl } from '@/player/msf_url'
import type { MOQtailPlayer } from '@/player/player'
import type { AbrController } from '@/player/plugin'
import type { PlayerState } from '@/player/state'
import { FullTrackName } from '@/model/data/full_track_name'
import { logger } from '@/util'
import {
  Action,
  climbTarget,
  downTarget,
  features,
  legalActions,
  SETTLE_TIMEOUT,
  SHORT,
  type Observation,
} from './features'
import type { Policy } from './policy'
import { planProbe, probeBytes, Prober, type ProbeTarget } from './probe'
import { Signals } from './signals'

/** @public What the controller decided on a tick, for logging. */
export interface Decision {
  action: Action
  observation: Observation
  features: number[]
  /** Set on the tick a probe ended: what it measured, in bytes per second. */
  probeResult?: number
  /** Set on a switch: the index in `observation.ladder` it asked for. */
  switchedTo?: number
  /** Set while too little live video arrived to judge the link; the action is then `Stay`. */
  warmingUp?: true
}

/** The default decision interval in ms; the bundled model was trained at it. */
export const TICK_MS = 500
/** Seconds after a switch before the requested rendition follows the one playing. */
const RESYNC = 4

/** A player's video source and the renditions ABR may choose from, by bitrate. */
interface Ladder {
  source: MediaSourceElement
  renditions: Rendition[]
}

/** Everything that lives as long as one attachment. */
interface Session {
  player: MOQtailPlayer
  sink: MediaSinkElement
  signals: Signals
  prober: Prober
  /** Index in the ladder of the rendition last asked for; set on the first tick. */
  requested: number | undefined
  switchedAt: number
  /** The switch taking effect: from the old track to the new, asked for at `since`. */
  settling: { from: string; to: string; since: number } | undefined
  settledAt: number
  /** The last stall, or the last time frames fell behind. */
  troubleAt: number
  stop: () => void
}

/**
 * @public
 * ABR for live playback: every tick a small neural network picks a switch down, a switch up, a probe
 * or nothing, among the actions `legalActions` allows.
 */
export class MlAbrController implements AbrController {
  readonly #tickMs: number
  readonly #onDecision: ((decision: Decision) => void) | undefined
  /** By rendition `src`. */
  readonly #targets = new Map<string, ProbeTarget>()
  #policy: Policy | undefined
  #session: Session | undefined

  constructor(tickMs: number, policy: Promise<Policy>, onDecision?: (decision: Decision) => void) {
    this.#tickMs = tickMs
    this.#onDecision = onDecision
    policy.then(
      (loaded) => {
        this.#policy = loaded
      },
      (error: unknown) => logger.error('abr', 'the model failed to load, so ABR stays idle', error),
    )
  }

  attach(player: MOQtailPlayer, sink: MediaSinkElement): void {
    this.detach()
    const signals = new Signals()
    const prober = new Prober()
    const onState = (state: PlayerState) => signals.playerState(state, seconds())
    const onAutoplayBlocked = () => signals.autoplayBlocked(player.state, seconds())
    player.on('statechange', onState)
    player.on('autoplayblocked', onAutoplayBlocked)
    onState(player.state)
    // Known before the first tick, so the first groups count as video
    signals.videoTracks = this.#videoTracks(player)
    const unfollow = followData(player, (event, track, header) => {
      if (header.stream === 'subgroup') signals.received(event, track)
      else if (event.kind === 'object')
        prober.received(track, header.requestId, { at: event.at / 1000, bytes: event.payloadLength })
    })
    const unclient = eachClient(player, () => {
      signals.forgetStreams()
      prober.cancel()
      return () => {}
    })
    const timer = setInterval(() => this.#tick(), this.#tickMs)
    this.#session = {
      player,
      sink,
      signals,
      prober,
      requested: undefined,
      switchedAt: -Infinity,
      settling: undefined,
      settledAt: -Infinity,
      troubleAt: -Infinity,
      stop: () => {
        clearInterval(timer)
        unfollow()
        unclient()
        player.off('statechange', onState)
        player.off('autoplayblocked', onAutoplayBlocked)
      },
    }
  }

  detach(): void {
    const session = this.#session
    if (!session) return
    this.#session = undefined
    session.stop()
    session.prober.cancel()
  }

  #tick(): void {
    const session = this.#session
    const policy = this.#policy
    // Only the viewer's pause holds decisions: a start or a seek the link can't feed is ABR's to solve
    if (!session || !policy || session.player.state === 'paused') return
    const ladder = videoLadder(session.player)
    if (!ladder) return
    const now = seconds()
    const { signals, prober } = session
    signals.videoTracks = this.#videoTracks(session.player)
    signals.prune(now)
    // Evidence ages only while the controller is free to act on it
    if (!session.settling) prober.age(this.#tickMs / 1000)
    const active = ladder.renditions.findIndex((r) => r.active)
    this.#settle(session, now)
    // After the settle, which can move where the windows start
    const stalledTime = signals.stalledTime(now)
    const requested = this.#requested(session, ladder, active, stalledTime, now)
    session.requested = requested
    const playing = ladder.renditions[active === -1 ? requested : active]!
    const arrivalRate = signals.arrivalRate(now, playing.frameRate)
    if (this.#contradicted(session, playing, stalledTime, now)) prober.forgetEstimates()
    const probeResult = prober.settle(now, signals.arrivals(), ladder.renditions[requested + 1]?.bitrate)
    if (stalledTime > 0 || arrivalRate < SHORT) session.troubleAt = now
    const observation = this.#observe(session, ladder, requested, playing, { stalledTime, arrivalRate }, now)
    const inputs = features(observation)
    const reported = { observation, features: inputs, ...(probeResult !== undefined && { probeResult }) }
    // A stall still gets a decision while warming up
    if (!signals.warmedUp(now) && observation.stalledTime === 0) {
      this.#onDecision?.({ ...reported, action: Action.Stay, warmingUp: true })
      return
    }
    const action = policy.choose(inputs, legalActions(observation))
    const switchedTo =
      action === Action.Up ? climbTarget(observation) : action === Action.Down ? downTarget(observation) : undefined
    if (switchedTo !== undefined) this.#switch(session, ladder, requested, switchedTo, now)
    if (action === Action.Probe) this.#probe(session, ladder, requested, now)
    this.#onDecision?.({ ...reported, action, ...(switchedTo !== undefined && { switchedTo }) })
  }

  /**
   * The rendition to count from. A superseded switch can land after the one that replaced it, so
   * once switches had time to land, it follows the one playing; not while stalled, where no switch
   * lands and asking again only supersedes more requests.
   */
  #requested(session: Session, ladder: Ladder, active: number, stalledTime: number, now: number): number {
    const { requested, signals } = session
    if (requested === undefined || requested >= ladder.renditions.length) return Math.max(active, 0)
    const settled = now - session.switchedAt > Math.max(RESYNC, 2 * (signals.groupInterval() ?? 0))
    return active !== -1 && settled && stalledTime === 0 ? active : requested
  }

  /**
   * A switch settled once every stream of the old track ended and the new track delivered. From then
   * on the windows measure the new track alone.
   */
  #settle(session: Session, now: number): void {
    const { settling, signals } = session
    if (!settling) return
    const settled = !signals.streaming(settling.from) && signals.lastObjectAt(settling.to) > settling.since
    if (!settled && now - settling.since < SETTLE_TIMEOUT) return
    session.settling = undefined
    session.settledAt = now
    signals.measureFrom(now)
  }

  /**
   * A stall, or frames arriving at under half their rate, shows the link no longer carries a probe
   * estimate above the bitrate being received. One below it agrees, and keeps its backoff.
   */
  #contradicted({ signals, prober }: Session, playing: Rendition, stalledTime: number, now: number): boolean {
    const estimate = prober.estimate
    const short = stalledTime > 0 || signals.starved(now, playing.frameRate)
    return short && estimate !== undefined && estimate * 8 > playing.bitrate
  }

  #observe(
    session: Session,
    ladder: Ladder,
    requested: number,
    playing: Rendition,
    { stalledTime, arrivalRate }: Pick<Observation, 'stalledTime' | 'arrivalRate'>,
    now: number,
  ): Observation {
    const { player, sink, signals, prober } = session
    const { currentTime, buffered } = sink
    const range = rangeAt(currentTime, buffered)
    return {
      forwardBuffer: range === -1 ? 0 : buffered.end(range) - currentTime,
      playbackRate: player.state === 'playing' ? sink.playbackRate : 0,
      stalledTime,
      ladder: ladder.renditions.map((r) => r.bitrate),
      requested,
      activeBitrate: playing.bitrate,
      receivedBytes: signals.receivedBytes(now),
      throughput: signals.throughput(now),
      arrivalRate,
      ...prober.observe(now),
      sinceSwitch: now - session.switchedAt,
      groupInterval: signals.groupInterval(),
      probeable: signals.knownGroups().length > 0,
      settling: session.settling !== undefined,
      sinceSettled: now - session.settledAt,
      sinceTrouble: now - session.troubleAt,
    }
  }

  #switch(session: Session, { source, renditions }: Ladder, from: number, to: number, now: number): void {
    session.requested = to
    session.switchedAt = now
    session.settling = {
      from: this.#target(renditions[from]!).name,
      to: this.#target(renditions[to]!).name,
      since: now,
    }
    if (to > from) session.prober.forgetFailures()
    else session.prober.steppedDown(now)
    // The latest switch wins: a superseded one rejects with ABORTED
    source.switch(renditions[to]!.id).catch(() => {})
  }

  #probe({ player, signals, prober }: Session, { source, renditions }: Ladder, requested: number, now: number): void {
    const groups = signals.knownGroups()
    const probed = source.getRenditions().find((r) => this.#target(r).name === groups[0]?.track)
    const client = player.client
    if (!client || !probed) return
    const bytes = probeBytes(renditions[requested]!.bitrate, renditions[requested + 1]!.bitrate)
    prober.start(client, this.#target(probed), planProbe(groups, bytes), now)
  }

  #videoTracks(player: MOQtailPlayer): Map<string, string> {
    const heads = new Map<string, string>()
    for (const r of videoSource(player)?.getRenditions() ?? []) {
      const head = this.#target(r).name
      for (const member of r.ladder ?? [head]) heads.set(member, head)
    }
    return heads
  }

  /** The track behind `rendition`, as a probe FETCHes it. */
  #target(rendition: Rendition): ProbeTarget {
    let target = this.#targets.get(rendition.src)
    if (!target) {
      const { track, namespace, trackBytes } = parseMsfUrl(rendition.src)
      target = { name: track, fullTrackName: FullTrackName.tryNew(namespace, trackBytes) }
      this.#targets.set(rendition.src, target)
    }
    return target
  }
}

/** The source with video renditions, else the one with the most renditions. */
function videoSource(player: MOQtailPlayer): MediaSourceElement | undefined {
  const sources = player.mediaSources
  const sized = sources.find((s) => s.getRenditions().some((r) => r.width !== undefined || r.height !== undefined))
  return sized ?? [...sources].sort((a, b) => b.getRenditions().length - a.getRenditions().length)[0]
}

/** Undefined when there is nothing to choose between, or a bitrate is unknown. */
function videoLadder(player: MOQtailPlayer): Ladder | undefined {
  const source = videoSource(player)
  const renditions = (source?.getRenditions() ?? []).filter((r) => r.selected).sort((a, b) => a.bitrate - b.bitrate)
  if (!source || renditions.length < 2 || renditions.some((r) => !(r.bitrate > 0))) return undefined
  return { source, renditions }
}

function seconds(): number {
  return performance.now() / 1000
}
