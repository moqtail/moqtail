# MOQtail

## Keep the docs in sync

A change to user-facing behaviour updates the matching page under `docs/` in the same change.

| Change                                         | Update                                                                                                                       |
| ---------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------- |
| Relay CLI flags, config keys, defaults         | `apps/relay/relay.example.yaml` (embedded in the config reference), `docs/relay/configure.md`, `docs/relay/reference/cli.md` |
| relay-api endpoints, features, MCP tools       | `docs/relay/observability.md`, `docs/relay/mcp.md`, `docs/relay/reference/http-api.md`                                       |
| Docker image, certificates, install steps      | `docs/relay/install.md`, `docs/relay/certificates.md`, `docs/relay/tutorial.md`                                              |
| moqtail-ts client API                          | `docs/ts/client/*`                                                                                                           |
| moqtail-ts player, element, plugins, worker    | `docs/ts/player/*`                                                                                                           |
| Package install, entry points, browser support | `docs/ts/install.md`, `docs/ts/index.md`                                                                                     |

The API reference under `docs/ts/api/` is generated from `@public` doc comments, so it follows the code. Prose and examples are updated by hand. Finish with `npm run docs:build`: dead links fail it.

## Docs style

The site follows [Diátaxis](https://diataxis.fr/): every page is one type and serves one goal.

- **Tutorial** (a lesson): the goal, prerequisites, numbered steps that each end in something the reader can see, then "Next steps".
- **How-to** (a recipe): the title is a verb and an object ("Install the relay"). One line on when to use it, the steps, then "More information".
- **Reference** (a dictionary): neutral description. Tables, or a source file embedded with `<<<`. No instructions.
- **Explanation** (a discussion): why and how it works. No steps.

Writing:

- Second person, present tense. Short, plain sentences. No marketing words.
- Sentence-case headings. Name a file in the sentence before its code block: a `[title]` on a fence only shows inside `::: code-group`, where the titles are the tabs for alternatives, or for the files of one example (`index.html` and `src/main.ts`). Use `::: tip` and `::: warning` sparingly.
- Links are relative and extensionless (`/relay/install`). Link API pages under "More information" at the end of a page.
- Every page sets `description` in its frontmatter. It becomes `og:description`.
- Terms: MOQtail (the project), the `moqtail` package, the relay, publisher, subscriber, namespace, track, MSF URL, WebTransport, raw QUIC.
- Examples use the public streams on `relay.moqtail.dev`, or a local relay at `moqt://localhost:4433`.
- Check every flag, default and option against the source, not against a README or an old binary.

## moqtail-ts (libs/moqtail-ts)

Write plain, explicit code with the smallest structure that keeps each step easy to follow.

### Layout

- File order: license header, module doc (only for a concept or invariant the code can't show), imports, exported types, constants, main class/functions, module helpers, `if (import.meta.vitest) { … }` (test-only imports via `await import(...)`).
- An element folder holds `index.ts` (`defineElement` + element tests), one class file, and self-testing helper modules. An element the player loads on first use also has `meta.ts`: every field but `init`, which the lazy built-in in `@/player/elements` and `index.ts` both spread. Its `index.ts` is then a package entry point (`moqtail/player/elements/<name>`), so nothing else may import it statically: the build fails if one does.

### Imports

- `@/…` across folders, `./…` within one. Never `../`. `@/` first, one line per module.
- `import type` for type-only imports.

### Classes and exports

- Private state is `#field` / `#method`. No TS `private`, no constructor parameter properties only the class reads.
- Order: readonly fields, mutable fields, constructor, getters, public, protected, `#private`.
- Export only what another module imports. `@public` opens the doc comment of API listed in the docs API reference (`docs/ts/api/`), and only on API reachable from an `index.ts` entry point.

### Functions

- Top-level functions use `function`; arrows for callbacks and local one-liners.
- One nameable step per function. Inline single-use wrappers; split bodies doing several steps, no further.
- A blank line separates the logical groups of a body, nested blocks included: guards, then setup, then each step, then the return. Keep a declaration next to the guard that checks it. Separate consecutive multi-line local helpers.
- No side effects in expressions (`a && f()`, comma operators, `return void x++`, `return logger.warn(...)`). Use an `if` block.
- No bare boolean flags at call sites: use two methods or a string literal param.
- `Promise.withResolvers()`, not hand-rolled deferreds. Name a repeated value once.

### Shared helpers

Grep before writing one. A helper moves to a shared home only when two modules need it.

| Home                           | Holds                                                                                                                                                                     |
| ------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `@/util`                       | client + player (`Emitter`, `errorMessage`)                                                                                                                               |
| `@/client/util`                | `withTimeout`, `isSessionClosed`, `ObjectStream`                                                                                                                          |
| `@/player/util/async`          | `backoffDelay`, `sleep`, `within`, `handledResolvers`                                                                                                                     |
| `@/player/util/track`          | `trackKey`, `renditionId`, `rendition`, `bitrateOf`                                                                                                                       |
| `@/player/util/json`           | `canonicalJson`, `deepEqual`, `parseJsonArray`                                                                                                                            |
| `@/player/element/source`      | switching set base: `SwitchingSetSource`, `Packaging`, `planSource`, `Delivery`, `Recovery`, `Switcher`; init data: `acceptedInitData`, `objectInitData`, `initOnObjects` |
| `@/player/element/sink`        | media sink base: `MediaSink`, `roleOf`                                                                                                                                    |
| `@/player/element/define`      | `definitionOf` (unset during `init`), `checkName`, `hasInterface`, `lazyElement`                                                                                          |
| `@/player/elements/loc/track`  | `locCapabilities`, `timestampMicros`, `decoderConfigOf`                                                                                                                   |
| `@/player/elements/cmaf/track` | `cmafCapabilities`, `mimeTypeOf`                                                                                                                                          |
| `@/player/constants`           | player defaults, subscriber priorities                                                                                                                                    |

### Errors and async

- Player: `new PlayerError(code, message, { element, cause })`, always with `element`. Caller mistakes are `TypeError`.
- Client: build an `InternalError`, `await this.disconnect(...)`, rethrow.
- Unknown error to text: `errorMessage(error)`.
- One `AbortController` per owner named `#abort` (`context.signal` in elements). DOM listeners take `{ signal }`.
- Fire-and-forget is `void promise`. A bare `.catch(() => {})` needs a one-line why unless obvious.

### Naming and comments

- One word, one meaning per file; don't reuse a nearby member's name. `previous` = the object before this one. Name fields for what they hold (`#clientOptions`).
- Comments state only the why the code can't show. Cite specs (`MSF 5.2.3`, `CMSF 3.6`, `moqt-22 §3.4`, `SSTS 6.3`). Never reference `PLAN.md`, `plan/` or milestones.

### Tests

- Pure logic in-source; MediaSource and custom elements in `test/browser`.
- Reuse `@/player/testing`: `FakeClient`, `testObject`, `mediaObject`, `gzip` (`testing/client`); `testChain`, `testSource`, `collectingSink`, `flush`, `settle` (`testing/elements`); `timeRanges` (`testing/media`).
- Client tests: `connected(serverOptions, clientOptions)` gives a post-handshake client.
- Few, meaningful tests; names state the behaviour.
