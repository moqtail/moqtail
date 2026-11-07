# moqtail-ts

Write plain, explicit code with the smallest structure that keeps each step easy to follow.

## Layout

- File order: license header, module doc (only for a concept or invariant the code can't show), imports, exported types, constants, main class/functions, module helpers, `if (import.meta.vitest) { … }` (test-only imports via `await import(...)`).
- An element folder holds `index.ts` (`defineElement` + element tests), one class file, and self-testing helper modules.

## Imports

- `@/…` across folders, `./…` within one. Never `../`. `@/` first, one line per module.
- `import type` for type-only imports.

## Classes and exports

- Private state is `#field` / `#method`. No TS `private`, no constructor parameter properties only the class reads.
- Order: readonly fields, mutable fields, constructor, getters, public, protected, `#private`.
- Export only what another module imports. `@public` opens the doc comment, only on API reachable from an `index.ts` entry point.

## Functions

- Top-level functions use `function`; arrows for callbacks and local one-liners.
- One nameable step per function. Inline single-use wrappers; split bodies doing several steps, no further.
- No side effects in expressions (`a && f()`, comma operators, `return void x++`, `return logger.warn(...)`). Use an `if` block.
- No bare boolean flags at call sites: use two methods or a string literal param.
- `Promise.withResolvers()`, not hand-rolled deferreds. Name a repeated value once.

## Shared helpers

Grep before writing one. A helper moves to a shared home only when two modules need it.

| Home                      | Holds                                                             |
| ------------------------- | ----------------------------------------------------------------- |
| `@/util`                  | client + player (`Emitter`, `errorMessage`)                       |
| `@/client/util`           | `withTimeout`, `isSessionClosed`                                  |
| `@/player/util/async`     | `backoffDelay`, `sleep`, `within`, `handledResolvers`             |
| `@/player/util/track`     | `trackKey`, `renditionId`, `rendition`, `bitrateOf`               |
| `@/player/util/json`      | `canonicalJson`, `deepEqual`, `parseJsonArray`                    |
| `@/player/element/define` | `definitionOf` (unset during `init`), `checkName`, `hasInterface` |
| `@/player/constants`      | player defaults, subscriber priorities                            |

## Errors and async

- Player: `new PlayerError(code, message, { element, cause })`, always with `element`. Caller mistakes are `TypeError`.
- Client: build an `InternalError`, `await this.disconnect(...)`, rethrow.
- Unknown error to text: `errorMessage(error)`.
- One `AbortController` per owner named `#abort` (`context.signal` in elements). DOM listeners take `{ signal }`.
- Fire-and-forget is `void promise`. A bare `.catch(() => {})` needs a one-line why unless obvious.

## Naming and comments

- One word, one meaning per file; don't reuse a nearby member's name. `previous` = the object before this one. Name fields for what they hold (`#clientOptions`).
- Comments state only the why the code can't show. Cite specs (`MSF 5.2.3`, `CMSF 3.6`, `moqt-22 §3.4`, `SSTS 6.3`). Never reference `PLAN.md`, `plan/` or milestones.

## Tests

- Pure logic in-source; MediaSource and custom elements in `test/browser`.
- Reuse `@/player/testing`: `FakeClient`, `testObject`, `mediaObject`, `gzip` (`testing/client`); `testChain`, `testSource`, `collectingSink`, `flush`, `settle` (`testing/elements`); `timeRanges` (`testing/media`).
- Client tests: `connected(serverOptions, clientOptions)` gives a post-handshake client.
- Few, meaningful tests; names state the behaviour.
