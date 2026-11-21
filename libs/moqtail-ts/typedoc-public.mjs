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

// TypeDoc can only exclude by tag (`@internal`), so this keeps an export only when it is tagged `@public`.
// TypeDoc turns `@public` into the Public flag (on the function for a signature's comment) and drops the tag.
import { Converter, ReflectionKind, makeRecursiveVisitor } from 'typedoc'

/** @param {import('typedoc').Application} app */
export function load(app) {
  app.converter.on(Converter.EVENT_RESOLVE_BEGIN, (context) => {
    for (const module of context.project.getReflectionsByKind(ReflectionKind.Module)) {
      for (const child of [...(module.children ?? [])]) {
        if (!isPublic(child)) context.project.removeReflection(child)
      }

      // An entry point globbed in without public exports, such as an element the player loads itself
      if (!module.children?.length) context.project.removeReflection(module)
    }
  })
  app.converter.on(Converter.EVENT_RESOLVE_END, (context) => warnHiddenReferences(context.project, app.logger))
}

/** @param {import('typedoc').DeclarationReflection} reflection */
function isPublic(reflection) {
  const target = reflection.isReference() ? reflection.getTargetReflectionDeep() : reflection
  return target.flags.isPublic
}

// TypeDoc's own notExported check skips removed symbols, so a public signature using a non-public type would render it as plain text.
/**
 * @param {import('typedoc').ProjectReflection} project
 * @param {import('typedoc').Logger} logger
 */
function warnHiddenReferences(project, logger) {
  const warned = new Set()
  for (const owner of Object.values(project.reflections)) {
    const visitor = makeRecursiveVisitor({
      reference(type) {
        const key = `${type.name} ${owner.getFriendlyFullName()}`
        if (type.reflection || !project.symbolIdHasBeenRemoved(type.symbolId) || warned.has(key)) return
        warned.add(key)
        logger.warn(`${type.name} is used by ${owner.getFriendlyFullName()} but is not @public`)
      },
    })
    for (const type of [owner.type, owner.default, ...(owner.extendedTypes ?? []), ...(owner.implementedTypes ?? [])]) {
      type?.visit(visitor)
    }
  }
}
