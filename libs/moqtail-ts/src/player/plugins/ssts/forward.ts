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
 * What reaches an SSTS source's pad: packets from whichever child the relay streams, their
 * `properties` when the arriving child changes, one `eos`, and one answer per seek.
 */

import type { SubscribeResult } from '@/client/types'
import { Location } from '@/model/common/location'
import type { MoqtObject } from '@/model/data/object'
import type { BaseElement } from '@/player/element/base'
import type { ElementContext } from '@/player/element/context'
import type { SourceElementDefinition } from '@/player/element/define'
import type { CatalogTrack, MediaSourceElement } from '@/player/element/interfaces'
import type { Pad, PadEvent, PadItem, PadProperties } from '@/player/element/pad'

/** One track of the set, played by its own child source. */
export interface Child {
  track: CatalogTrack
  element?: BaseElement & MediaSourceElement
  /** The latest properties it wrote, merged. */
  properties: Partial<PadProperties>
  /** Its current subscription in the set, while selected. */
  subscription?: { result: SubscribeResult }
  eos: boolean
}

/** A child loaded for a seek behind the live edge, forwarded instead of the set until it ends. */
interface SeekChild {
  element?: BaseElement
  properties: Partial<PadProperties>
  started: boolean
}

interface ForwardHost {
  readonly context: ElementContext
  readonly pad: Pad
  /** The definition each child is loaded from. */
  readonly child: SourceElementDefinition
  /** The selected children of the set. */
  selected(): Child[]
  childOf(element: BaseElement | undefined): Child | undefined
  /** The source's sink pad taking `child`'s pad. */
  sinkPadOf(child: Child): Pad | undefined
  /** The arriving track changed, after its first packet. */
  renditionChanged(track: CatalogTrack): void
}

export class Forwarder {
  readonly #host: ForwardHost
  /** The child whose packets reached the pad last. */
  arriving: Child | undefined
  /** The track of the last packet from the set. */
  #emitted: CatalogTrack | undefined
  #last: Location | undefined
  #seekChild: SeekChild | undefined
  #seeks = 0
  /** Seeks answered here, before the next answer: superseded, or served by a seek child. */
  #owed = 0
  #gap = false
  #ended = false

  constructor(host: ForwardHost) {
    this.#host = host
  }

  async receive(from: BaseElement | undefined, item: PadItem): Promise<void> {
    const { pad } = this.#host
    const seekChild = this.#seekChild
    if (seekChild && from === seekChild.element) return this.#forwardSeekChild(seekChild, item)
    const child = this.#host.childOf(from)
    if (!child) return
    if (item.type === 'properties') {
      child.properties = { ...child.properties, ...item.properties }
      if (child === this.arriving && !seekChild) await pad.write(item)
      return
    }
    if (seekChild) return // the set waits behind the seek child's history
    if (child !== this.arriving) {
      // The relay switched tracks: the pad carries the new one from here
      this.arriving = child
      await pad.write({ type: 'properties', properties: child.properties })
    }
    // The first arrival too: until then no rendition is active
    const changed = this.#emitted !== child.track
    this.#emitted = child.track
    await this.#writePacket(item.object)
    if (changed) this.#host.renditionChanged(child.track)
  }

  /**
   * Answers every seek once. Behind the live edge, a separate child fetches the history:
   * routed to one child, a live history seek would move only that child behind the edge
   * while the relay keeps the others there. Otherwise the arriving child serves it.
   */
  async seek(mediaTime: number): Promise<void> {
    const call = ++this.#seeks
    this.#dropSeekChild('superseded')
    const child = this.arriving ?? this.#host.selected()[0]
    if (!child) {
      this.#owed++
      return
    }
    const last = this.#last
    const timeline = child.track.isLive && last ? await this.#host.context.catalog?.timeline(child.track) : undefined
    if (call !== this.#seeks) {
      this.#owed++ // superseded: answered with the next answer
      return
    }
    const target = timeline?.timeToLocation(mediaTime * 1000)
    if (target && last && target.group <= last.group) {
      await this.#loadSeekChild(child, target, last)
    } else {
      const sinkPad = this.#host.sinkPadOf(child)
      if (sinkPad) sinkPad.signal({ type: 'seek', mediaTime })
      else this.#owed++
    }
  }

  /** A downstream event from a child; all are consumed here. */
  onChildEvent(from: BaseElement | undefined, event: PadEvent): boolean {
    if (this.#seekChild && from === this.#seekChild.element) {
      if (event.type === 'eos') this.#dropSeekChild('ended')
      else if (event.type === 'discontinuity' && event.reason === 'gap') void this.#writeEvent(event)
      return true
    }
    const child = this.#host.childOf(from)
    if (!child) return true
    if (event.type === 'eos') {
      child.eos = true
      this.endIfDone()
    } else if (event.type === 'discontinuity' && event.reason === 'seek') {
      // A routed seek's answer, behind any owed here
      void this.#answer('ok').then(() => this.#writeEvent(event))
    } else if (event.type === 'discontinuity' && child === this.arriving) {
      void this.#writeEvent(event)
    }
    return true
  }

  /** True if `from` was the seek child, whose removal ends it. */
  seekChildRemoved(from: BaseElement | undefined): boolean {
    if (!this.#seekChild || from !== this.#seekChild.element) return false
    const started = this.#seekChild.started
    this.#dropSeekChild('ended')
    if (!started) void this.#answer('failed')
    return true
  }

  /** One `eos`, once every selected child sent one. */
  endIfDone(): void {
    const selected = this.#host.selected()
    if (this.#ended || selected.length === 0 || !selected.every((c) => c.eos)) return
    this.#ended = true
    void this.#writeEvent({ type: 'eos' })
  }

  /** Loads a child that fetches the history from `target`'s group up to `last`'s, outside the set. */
  async #loadSeekChild(child: Child, target: Location, last: Location): Promise<void> {
    const { context } = this.#host
    this.#owed++
    const seekChild: SeekChild = { properties: {}, started: false }
    this.#seekChild = seekChild
    const locationRanges = [{ start: new Location(target.group, 0n), end: { group: last.group } }]
    try {
      // No request hooks: the history is fetched outside the set
      const element = await context.load(this.#host.child, {
        tracks: [child.track],
        shared: false,
        options: { ranges: { locationRanges, mediaTimeRanges: [], wallclockRanges: [] } },
      })
      seekChild.element = element
      if (this.#seekChild !== seekChild) void context.release(element)
    } catch {
      if (this.#seekChild === seekChild) this.#dropSeekChild('ended')
      await this.#answer('failed')
    }
  }

  #writeEvent(event: PadEvent): Promise<void> {
    return this.#host.pad.write({ type: 'event', event })
  }

  async #writePacket(object: MoqtObject): Promise<void> {
    if (this.#gap) {
      this.#gap = false
      await this.#writeEvent({ type: 'discontinuity', reason: 'gap' })
    }
    await this.#host.pad.write({ type: 'packet', object })
    this.#last = object.location
  }

  async #forwardSeekChild(seekChild: SeekChild, item: PadItem): Promise<void> {
    const { pad } = this.#host
    if (item.type === 'properties') {
      seekChild.properties = { ...seekChild.properties, ...item.properties }
      if (seekChild.started) await pad.write(item)
      return
    }
    if (!seekChild.started) {
      seekChild.started = true
      await pad.write({ type: 'properties', properties: seekChild.properties })
      await this.#answer('ok')
    }
    await this.#writePacket(item.object)
  }

  /** Releases the seek child. Once it ended, the set resumes after a gap, its properties written again. */
  #dropSeekChild(reason: 'superseded' | 'ended'): void {
    const seekChild = this.#seekChild
    if (!seekChild) return
    this.#seekChild = undefined
    if (seekChild.element) void this.#host.context.release(seekChild.element)
    if (reason === 'ended' && seekChild.started) {
      this.#gap = true
      this.arriving = undefined
    }
  }

  /** Writes the seeks owed here, the latest as failed for `failed`. */
  async #answer(outcome: 'ok' | 'failed'): Promise<void> {
    while (this.#owed > 0) {
      const last = --this.#owed === 0
      const failed = outcome === 'failed' && last
      const event: PadEvent = { type: 'discontinuity', reason: 'seek', ...(failed && { failed }) }
      await this.#writeEvent(event)
    }
  }
}
