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
 * Plugins offer features (switching set sources, controllers, elements) and the setup
 * parameters they need, so a protocol extension or a controller is one entry in `opts.plugins`.
 */

import type { Setup } from '@/model/control/setup'
import { SetupOptions } from '@/model/parameter/setup_option'
import {
  checkName,
  DEFAULT_PRIORITY,
  type ElementDefinition,
  type SourceElementDefinition,
} from '@/player/element/define'
import type { MediaSinkElement } from '@/player/element/interfaces'
import type { MOQtailPlayer } from '@/player/player'
import { canonicalJson } from '@/player/util/json'
import { PlayerError } from './error'

/**
 * @public
 * The player attaches it once a live stream is ready, and detaches it on `destroy()` and when
 * the stream turns on-demand. Times are in seconds.
 */
export interface BufferController {
  attach(player: MOQtailPlayer, sink: MediaSinkElement): void
  detach(): void
  /** Seconds kept behind `currentTime`; replaces the sink's own `backBuffer` while attached. */
  backBuffer?(): number
}

/** @public What a plugin offers. */
export type Feature =
  /** Loaded by the catalog for a media switching set whose first track it accepts, with `{ child }` (`SwitchingSetOptions`). */
  | { type: 'switch-source'; item: SourceElementDefinition }
  | { type: 'buffer-controller'; item: BufferController }
  /** A source, transform, or sink, registered like `registerElement`. */
  | { type: 'element'; item: ElementDefinition }

/** @public Every hook is optional: no `init` means ctx is `undefined`, no `setup` or `negotiated` means true. */
export interface PluginDefinition<Config = void, Ctx = undefined> {
  readonly name: string
  /** Lower wins, like `ElementDefinition.priority`. Default 100. Ties go to the earlier entry in `opts.plugins`. */
  readonly priority?: number
  /** Once per player, in the constructor. The config comes from `opts.plugins`. */
  init?(config: Config): Ctx
  /** Synchronous, in the constructor. Adds setup parameters to its own `clientSetup`; false drops the plugin. */
  setup?(ctx: Ctx, clientSetup: SetupOptions): boolean
  /** After SERVER_SETUP. False means none of its features are used (its parameters were already sent). */
  negotiated?(ctx: Ctx, serverSetup: Setup): boolean
  /** What the plugin offers, once per player after `init`. */
  features?(ctx: Ctx): Feature[]
}

/** @public An entry for `opts.plugins`, from calling what `definePlugin` returns. */
export interface Plugin {
  readonly definition: PluginDefinition<any, any>
  readonly config: unknown
}

/** A plugin of one player, after `init` and `setup`. */
export interface PluginInstance {
  readonly name: string
  readonly priority: number
  readonly ctx: unknown
  readonly definition: PluginDefinition<any, any>
  readonly setupOptions: SetupOptions
  readonly features: readonly Feature[]
}

/** The features one player uses. */
export interface Features {
  bufferController?: BufferController
  /** In plugin order, before the built-in sources. */
  elements: ElementDefinition[]
  switchSources: SourceElementDefinition[]
}

/**
 * @public
 * Checks a plugin definition and returns a factory: calling it with the config gives an entry
 * for `opts.plugins`. A plugin whose config is all optional can be called with no arguments.
 */
export function definePlugin<Config = void, Ctx = undefined>(
  definition: PluginDefinition<Config, Ctx>,
): (...config: [Config] extends [void] ? [] : {} extends Config ? [config?: Config] : [config: Config]) => Plugin {
  checkName(`Plugin "${String(definition.name)}"`, definition.name)
  return (...config) => ({ definition, config: config[0] })
}

/**
 * Runs `init` then `setup` for each plugin, in order, and drops those whose `setup` refused.
 * @throws TypeError for two plugins with the same name.
 */
export function initPlugins(plugins: readonly Plugin[]): PluginInstance[] {
  const names = new Set<string>()
  for (const { definition } of plugins) {
    if (names.has(definition.name)) throw new TypeError(`Plugin "${definition.name}" is listed twice`)
    names.add(definition.name)
  }
  const instances: PluginInstance[] = []
  for (const { definition, config } of plugins) {
    const ctx = definition.init?.(config)
    const setupOptions = new SetupOptions()
    if (definition.setup && !definition.setup(ctx, setupOptions)) continue
    instances.push({
      name: definition.name,
      priority: definition.priority ?? DEFAULT_PRIORITY,
      ctx,
      definition,
      setupOptions,
      features: definition.features?.(ctx) ?? [],
    })
  }
  return instances
}

/** The plugins whose `negotiated` accepted `serverSetup`, by priority (low first), then in list order. */
export function activePlugins(instances: readonly PluginInstance[], serverSetup: Setup): PluginInstance[] {
  return instances
    .filter((p) => p.definition.negotiated?.(p.ctx, serverSetup) ?? true)
    .sort((a, b) => a.priority - b.priority) // stable, so ties keep list order
}

/** The buffer controller comes from the first plugin offering one; elements and switch sources from all of them, in order. */
export function resolveFeatures(active: readonly PluginInstance[]): Features {
  const features = active.flatMap((p) => p.features)
  const items = <T extends Feature['type']>(type: T) =>
    features.flatMap((f) => (f.type === type ? [f.item as Extract<Feature, { type: T }>['item']] : []))
  const [bufferController] = items('buffer-controller')
  return {
    ...(bufferController && { bufferController }),
    elements: items('element'),
    switchSources: items('switch-source'),
  }
}

/**
 * `client.setupOptions` followed by each plugin's. A plugin setting a parameter type to
 * another value than an earlier entry throws `UNSUPPORTED_ORIGIN`.
 */
export function mergeSetupOptions(
  base: SetupOptions | undefined,
  plugins: readonly PluginInstance[],
): SetupOptions | undefined {
  if (!plugins.some((p) => p.setupOptions.build().length > 0)) return base
  const merged = new SetupOptions()
  const owners = new Map<bigint, { owner: string; value: string }>()
  for (const pair of base?.build() ?? []) {
    merged.addRaw(pair)
    if (!owners.has(pair.typeValue))
      owners.set(pair.typeValue, { owner: 'client.setupOptions', value: canonicalJson(pair.value) })
  }
  for (const plugin of plugins) {
    for (const pair of plugin.setupOptions.build()) {
      const value = canonicalJson(pair.value)
      const existing = owners.get(pair.typeValue)
      if (existing && existing.value !== value) {
        throw new PlayerError(
          'UNSUPPORTED_ORIGIN',
          `${existing.owner} and plugin "${plugin.name}" set setup parameter 0x${pair.typeValue.toString(16)} to different values`,
        )
      }
      if (existing) continue
      owners.set(pair.typeValue, { owner: `plugin "${plugin.name}"`, value })
      merged.addRaw(pair)
    }
  }
  return merged
}

if (import.meta.vitest) {
  const { describe, test, expect, expectTypeOf } = import.meta.vitest
  const { Setup } = await import('@/model/control/setup')
  const { testSource } = await import('@/player/testing/elements')
  const buffer = (name: string): BufferController => ({ attach() {}, detach() {}, backBuffer: () => name.length })

  describe('plugins', () => {
    test('the factory infers its config: optional when all of it is, none without init', () => {
      const optional = definePlugin({ name: 'a', init: (c: { n?: number } = {}) => c })
      const required = definePlugin({ name: 'b', init: (c: { n: number }) => c })
      const none = definePlugin({ name: 'c' })
      expectTypeOf(none).parameters.toEqualTypeOf<[]>()
      expect([optional().config, optional({ n: 1 }).config, required({ n: 2 }).config]).toEqual([
        undefined,
        { n: 1 },
        { n: 2 },
      ])
      // @ts-expect-error the config is required
      required()
      // @ts-expect-error a config of the wrong type
      optional({ n: '1' })
      expect(() => definePlugin({ name: 'a b' })).toThrow(TypeError)
    })

    test('init and setup run per player; a refused setup drops the plugin, a repeated name throws', () => {
      let inits = 0
      const counted = definePlugin({ name: 'counted', init: () => ({ n: ++inits }) })
      const refused = definePlugin({
        name: 'refused',
        setup: (_, s) => {
          s.addMaxRequestUpdates(1)
          return false
        },
      })
      const [first] = initPlugins([counted(), refused()])
      const second = initPlugins([counted()])
      expect([first!.ctx, second[0]!.ctx, initPlugins([refused()])]).toEqual([{ n: 1 }, { n: 2 }, []])
      expect(() => initPlugins([counted(), counted()])).toThrow(TypeError)
    })

    test('active plugins go by priority then list order; controllers come from the first, sources from all', () => {
      const offer = (name: string, priority: number, accepted = true) =>
        definePlugin({
          name,
          priority,
          negotiated: () => accepted,
          features: () => [
            { type: 'buffer-controller', item: buffer(name) },
            { type: 'element', item: testSource({ name }) },
            { type: 'switch-source', item: testSource({ name: `${name}-switch` }) },
          ],
        })()
      const plugins = initPlugins([
        offer('late', Infinity),
        offer('tie-1', 100),
        offer('tie-2', 100),
        offer('no', 1, false),
      ])
      const features = resolveFeatures(activePlugins(plugins, new Setup([])))
      expect(features.bufferController?.backBuffer?.()).toBe('tie-1'.length)
      expect(features.elements.map((e) => e.name)).toEqual(['tie-1', 'tie-2', 'late'])
      expect(features.switchSources.map((e) => e.name)).toEqual(['tie-1-switch', 'tie-2-switch', 'late-switch'])
      expect(resolveFeatures([])).toEqual({ elements: [], switchSources: [] })
    })

    test('setup parameters merge after client.setupOptions; a conflict names both owners', () => {
      const algorithms = (name: string, ...ids: number[]) =>
        definePlugin({
          name,
          setup: (_, s) => {
            s.addSstsAlgorithms(ids)
            return true
          },
        })()
      const base = new SetupOptions().addMaxRequestUpdates(4)
      const merged = mergeSetupOptions(base, initPlugins([algorithms('a', 1), algorithms('b', 1)]))
      expect(merged?.build()).toHaveLength(2)
      expect(mergeSetupOptions(base, initPlugins([definePlugin({ name: 'none' })()]))).toBe(base)
      expect(() => mergeSetupOptions(undefined, initPlugins([algorithms('a', 1), algorithms('b', 2)]))).toThrow(
        'plugin "a" and plugin "b"',
      )
    })
  })
}
