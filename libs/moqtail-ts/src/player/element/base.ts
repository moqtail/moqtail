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
 * The abstract element classes. They stay bare and only hold what the chain needs;
 * anything media specific is an interface the element lists in `provides`.
 */

import { Emitter } from '../../util/emitter'
import type { ElementContext } from './context'
import type { Capabilities, ElementDefinition } from './define'
import { matchesCapabilities } from './negotiation'
import { addPad, removePad, type Pad, type PadItem } from './pad'

/** @public Signals between an element and the player. */
export interface ElementEvents {
  padadded: (pad: Pad) => void
  /** `failed` is true when the pad was lost to a failure rather than, say, a catalog update. */
  padremoved: (pad: Pad, failed: boolean) => void
  /** Sources emit it once every expected pad exists; the player then emits it on sinks. */
  nomorepads: () => void
  /** Sinks emit it once they can play. */
  ready: () => void
}

/** @public Base of every element instance. */
export abstract class BaseElement extends Emitter<ElementEvents> {
  /** Set by `defineElement` once `init` resolves, so it can't be used from a constructor. */
  declare readonly definition: ElementDefinition
  readonly pads: Pad[] = []

  /** Calls `context.attach(this)`, so the player hears everything the instance emits. */
  constructor(protected readonly context: ElementContext) {
    super()
    context.attach(this)
  }

  /**
   * Removes the element's pads. The runtime aborts `context.signal` before and releases the
   * elements this one loaded after. Override for your own cleanup and call `super.destroy()`.
   */
  async destroy(): Promise<void> {
    for (const pad of [...this.pads]) removePad(pad)
  }

  /** True if the pad matches one of the definition's `in` capability bundles. */
  protected matchesCapabilities(pad: Pad): boolean {
    return matchesCapabilities(this.definition.capabilities, pad)
  }

  /** Creates a pad and emits `padadded`. */
  protected createPad(direction: 'src' | 'sink', capabilities: Capabilities): Pad {
    return addPad(this, direction, capabilities)
  }

  /** Removes a pad and emits `padremoved`. Pass `failed` when a failure, not a catalog update, lost it. */
  protected removePad(pad: Pad, failed = false): void {
    removePad(pad, failed)
  }
}

/** @public Writes to its source pads from anywhere, and emits `nomorepads` once every expected pad exists. */
export abstract class SourceElement extends BaseElement {}

/** @public Takes pads from upstream and writes its output to source pads it creates. */
export abstract class TransformElement extends BaseElement {
  /** Whether to take this upstream source pad. */
  abstract negotiate(pad: Pad): boolean
  /** Called once per item; the pad's next item waits for the returned promise. */
  abstract receive(pad: Pad, item: PadItem): void | Promise<void>
}

/** @public Consumes pads. Listens for `nomorepads` to learn every source has reported, and emits `ready`. */
export abstract class SinkElement extends BaseElement {
  /** Whether to take this upstream source pad. */
  abstract negotiate(pad: Pad): boolean
  /** Called once per item; the pad's next item waits for the returned promise. */
  abstract receive(pad: Pad, item: PadItem): void | Promise<void>
}
