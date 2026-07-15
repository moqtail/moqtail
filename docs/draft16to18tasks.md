# Execution Plan: moqtail draft-16 → draft-18

The **execution plan** for migrating this repo from draft-ietf-moq-transport-16 to -18:
ordered, independently-verifiable tasks, each sized to become one GitHub issue / one PR.

Self-contained — it supersedes an earlier set of working notes (a spec diff and a code
audit) that are not part of the repo. Where a finding below contradicts those notes, §0
records the correction and the evidence, so nothing depends on reading them.

Every task cites the **exact changelog bullet(s)** it implements, quoted verbatim with a
line number into `docs/draft-ietf-moq-transport-18.txt`, in this form:

> 📎 **A.2 : 7346** — "New variable-length integer encoding (#1016)"

`A.1` = _Since draft-ietf-moq-transport-17_ (line 7202). `A.2` = _Since
draft-ietf-moq-transport-16_ (line 7324). §9 gives the reverse map — every bullet in both
sections → its task — so coverage can be checked in either direction.

| Prefix | Component               | Path               |
| ------ | ----------------------- | ------------------ |
| `C-`   | Common (cross-language) | `dev/conformance/` |
| `RS-`  | moqtail-rs              | `libs/moqtail-rs`  |
| `RL-`  | relay                   | `apps/relay`       |
| `CL-`  | client                  | `apps/client`      |
| `TS-`  | moqtail-ts              | `libs/moqtail-ts`  |
| `JS-`  | client-js               | `apps/client-js`   |
| `MT-`  | meet                    | `apps/meet`        |

Order: **C → RS → RL → CL → (Rust interop gate) → TS → JS → MT → (JS interop gate)**.

All tasks are filed as GitHub issues under the
[**draft-18** milestone](https://github.com/moqtail/moqtail/milestone/3), with dependencies
expressed as native GitHub `blocked_by` relationships rather than prose — so the milestone
view shows what is actually startable, and this table is only for mapping a task ID back to
its issue.

**Startable today (no blockers):** [#222](https://github.com/moqtail/moqtail/issues/222) `RS-2a`,
[#223](https://github.com/moqtail/moqtail/issues/223) `C-1`,
[#224](https://github.com/moqtail/moqtail/issues/224) `RS-21`,
[#275](https://github.com/moqtail/moqtail/issues/275) `TS-17`.

<details>
<summary><b>Task → issue map</b> (58 issues)</summary>

| Task           | Issue                                                 | Title                                                                               |
| -------------- | ----------------------------------------------------- | ----------------------------------------------------------------------------------- |
| **Common**     |                                                       |                                                                                     |
| `C-1`          | [#223](https://github.com/moqtail/moqtail/issues/223) | Lock draft-18 constants and varint vectors into shared conformance fixtures         |
| **moqtail-rs** |                                                       |                                                                                     |
| `RS-2a`        | [#222](https://github.com/moqtail/moqtail/issues/222) | Bump ALPN to moqt-18 — wire format is draft-18 but still advertises moqt-16         |
| `RS-2`         | [#225](https://github.com/moqtail/moqtail/issues/225) | Update the control message type table to draft-18 (Table 5)                         |
| `RS-3`         | [#227](https://github.com/moqtail/moqtail/issues/227) | Collapse CLIENT_SETUP/SERVER_SETUP into a single SETUP message                      |
| `RS-4`         | [#232](https://github.com/moqtail/moqtail/issues/232) | Change the control stream from one bidi stream to a pair of uni streams             |
| `RS-5`         | [#235](https://github.com/moqtail/moqtail/issues/235) | Move all seven request types onto their own bidirectional request streams           |
| `RS-6`         | [#236](https://github.com/moqtail/moqtail/issues/236) | Remove MAX_REQUEST_ID and REQUESTS_BLOCKED                                          |
| `RS-6b`        | [#241](https://github.com/moqtail/moqtail/issues/241) | Remove the cancel/teardown message family                                           |
| `RS-7`         | [#237](https://github.com/moqtail/moqtail/issues/237) | Fold PUBLISH_OK into REQUEST_OK and add Track Properties to REQUEST_OK              |
| `RS-7b`        | [#242](https://github.com/moqtail/moqtail/issues/242) | Remove Request ID from response messages                                            |
| `RS-8`         | [#226](https://github.com/moqtail/moqtail/issues/226) | Rename Extension Headers to Properties                                              |
| `RS-9`         | [#228](https://github.com/moqtail/moqtail/issues/228) | Split DELIVERY_TIMEOUT, add RENDEZVOUS_TIMEOUT/FILL_TIMEOUT, align property numbers |
| `RS-10`        | [#238](https://github.com/moqtail/moqtail/issues/238) | Add stream reset error codes and build the reset path                               |
| `RS-11`        | [#243](https://github.com/moqtail/moqtail/issues/243) | Split SUBSCRIBE_NAMESPACE into SUBSCRIBE_NAMESPACE and SUBSCRIBE_TRACKS             |
| `RS-12`        | [#247](https://github.com/moqtail/moqtail/issues/247) | Allow zero-element track namespaces (bound 1-32 → 0-32)                             |
| `RS-13`        | [#244](https://github.com/moqtail/moqtail/issues/244) | GOAWAY: add Request ID and Timeout, per-request migration, REDIRECT                 |
| `RS-14`        | [#229](https://github.com/moqtail/moqtail/issues/229) | Add the FIRST_OBJECT bit to SUBGROUP_HEADER                                         |
| `RS-15`        | [#230](https://github.com/moqtail/moqtail/issues/230) | Delta-encode Group/Object ID in FETCH responses; close session on wrap              |
| `RS-16`        | [#248](https://github.com/moqtail/moqtail/issues/248) | Add the PUBLISH_BLOCKED message                                                     |
| `RS-17`        | [#252](https://github.com/moqtail/moqtail/issues/252) | Deferred data-plane additions (padding, subgroup reopen, EndGroup delta, .session)  |
| `RS-18`        | [#234](https://github.com/moqtail/moqtail/issues/234) | Add GREASE support and reserve the application-specific Property ranges             |
| `RS-19`        | [#239](https://github.com/moqtail/moqtail/issues/239) | Remove TRACK_STATUS from REQUEST_UPDATE                                             |
| `RS-20`        | [#240](https://github.com/moqtail/moqtail/issues/240) | Make the auth token cache safe across multiple request streams                      |
| `RS-21`        | [#224](https://github.com/moqtail/moqtail/issues/224) | Document `Switch = 0x22` as a non-conformant local extension                        |
| **relay**      |                                                       |                                                                                     |
| `RL-1`         | [#231](https://github.com/moqtail/moqtail/issues/231) | Enforce Mandatory Track Properties (0x4000-0x7FFF) at the relay                     |
| `RL-2`         | [#253](https://github.com/moqtail/moqtail/issues/253) | Handle SUBSCRIBE_TRACKS and emit PUBLISH_BLOCKED                                    |
| `RL-3`         | [#249](https://github.com/moqtail/moqtail/issues/249) | SUBSCRIBE takes precedence over SUBSCRIBE_NAMESPACE; exclude own tracks             |
| `RL-4`         | [#245](https://github.com/moqtail/moqtail/issues/245) | Truthful LARGEST_OBJECT; tolerate unknown error codes                               |
| `RL-5`         | [#250](https://github.com/moqtail/moqtail/issues/250) | Emit stream reset codes on abort paths; GOAWAY per-request migration                |
| `RL-6`         | [#246](https://github.com/moqtail/moqtail/issues/246) | Coalesce REQUEST_UPDATE processing; EXPIRES update mechanism                        |
| `RL-7`         | [#251](https://github.com/moqtail/moqtail/issues/251) | FETCH semantics (empty track, End Location, joining fetch)                          |
| **client**     |                                                       |                                                                                     |
| `CL-1`         | [#233](https://github.com/moqtail/moqtail/issues/233) | Make `moqt://` the unified scheme on the client, including WebTransport             |
| `CL-2`         | [#254](https://github.com/moqtail/moqtail/issues/254) | Rust interop gate: client to relay end-to-end on draft-18                           |
| **moqtail-ts** |                                                       |                                                                                     |
| `TS-2`         | [#255](https://github.com/moqtail/moqtail/issues/255) | Update the control message type table to draft-18                                   |
| `TS-3`         | [#256](https://github.com/moqtail/moqtail/issues/256) | Collapse CLIENT_SETUP/SERVER_SETUP into SETUP; add missing setup options            |
| `TS-4`         | [#257](https://github.com/moqtail/moqtail/issues/257) | Control stream from one bidi to a pair of uni streams                               |
| `TS-5`         | [#258](https://github.com/moqtail/moqtail/issues/258) | Move all seven request types onto their own bidi request streams                    |
| `TS-6`         | [#259](https://github.com/moqtail/moqtail/issues/259) | Remove MAX_REQUEST_ID and REQUESTS_BLOCKED                                          |
| `TS-6b`        | [#261](https://github.com/moqtail/moqtail/issues/261) | Remove the cancel/teardown message family                                           |
| `TS-7`         | [#262](https://github.com/moqtail/moqtail/issues/262) | Fold PUBLISH_OK into REQUEST_OK; add Track Properties to REQUEST_OK                 |
| `TS-7b`        | [#263](https://github.com/moqtail/moqtail/issues/263) | Remove Request ID from response messages                                            |
| `TS-8`         | [#264](https://github.com/moqtail/moqtail/issues/264) | Rename Extension Headers to Properties                                              |
| `TS-9`         | [#265](https://github.com/moqtail/moqtail/issues/265) | Split DELIVERY_TIMEOUT, add RENDEZVOUS_TIMEOUT/FILL_TIMEOUT, align property numbers |
| `TS-10`        | [#260](https://github.com/moqtail/moqtail/issues/260) | Add stream reset error codes and build the abort path                               |
| `TS-11`        | [#266](https://github.com/moqtail/moqtail/issues/266) | Split SUBSCRIBE_NAMESPACE into SUBSCRIBE_NAMESPACE and SUBSCRIBE_TRACKS             |
| `TS-12`        | [#267](https://github.com/moqtail/moqtail/issues/267) | Allow zero-element track namespaces                                                 |
| `TS-13`        | [#268](https://github.com/moqtail/moqtail/issues/268) | GOAWAY Request ID and Timeout, per-request migration, REDIRECT                      |
| `TS-14`        | [#269](https://github.com/moqtail/moqtail/issues/269) | Add the FIRST_OBJECT bit to SUBGROUP_HEADER                                         |
| `TS-15`        | [#270](https://github.com/moqtail/moqtail/issues/270) | Delta-decode Group/Object ID in FETCH responses                                     |
| `TS-16`        | [#271](https://github.com/moqtail/moqtail/issues/271) | Add the PUBLISH_BLOCKED message                                                     |
| `TS-17`        | [#275](https://github.com/moqtail/moqtail/issues/275) | Fix TerminationCode.tryFrom throwing on five valid enum values                      |
| `TS-18`        | [#272](https://github.com/moqtail/moqtail/issues/272) | Add GREASE support and the application-specific Property ranges                     |
| `TS-19`        | [#273](https://github.com/moqtail/moqtail/issues/273) | Remove TRACK_STATUS from REQUEST_UPDATE                                             |
| `TS-20`        | [#274](https://github.com/moqtail/moqtail/issues/274) | Make the auth token cache safe across multiple request streams                      |
| **client-js**  |                                                       |                                                                                     |
| `JS-1`         | [#276](https://github.com/moqtail/moqtail/issues/276) | Absorb the moqtail-ts API break                                                     |
| `JS-2`         | [#278](https://github.com/moqtail/moqtail/issues/278) | Make `moqt://` the real transport scheme + JS interop gate                          |
| **meet**       |                                                       |                                                                                     |
| `MT-1`         | [#277](https://github.com/moqtail/moqtail/issues/277) | Absorb the moqtail-ts API break                                                     |
| `MT-2`         | [#279](https://github.com/moqtail/moqtail/issues/279) | Adopt `moqt://`                                                                     |

</details>

---

## 0. Status: verified against the tree at commit `4dd968c`

Re-verified by reading the code and running both suites — `cargo test -p moqtail`
**247 passed**, `npm test` **310 passed (67 files)**.

### ✅ RS-1 and TS-1 are both DONE — including moqtail-ts

**The brief for this pass said the varint landed in Rust "not moqtail-ts." That is not what
commit `4dd968c` did — it updated both.** Verified:

- `libs/moqtail-rs/src/model/common/varint.rs:38` — `first.leading_ones() + 1`.
- `libs/moqtail-ts/src/model/common/byte_buffer.ts:80-113` — `countLeadingOnes(first)`,
  `length = leadingOnes + 1`, `firstByteValueBits()`.
- Immediately before that commit, `byte_buffer.ts` was still QUIC-style
  (`git show 4dd968c^:…/byte_buffer.ts` → `const prefix = first >> 6`, switch on 1/2/4/8).

Both are correct and cover the draft-18 §1.4.1 Table 2 vectors, including the non-minimal
case (`varint.rs:290` `decodes_non_minimal_encodings_across_lengths` with `[0x80,0x25] → 37`;
`byte_buffer.ts:668` `decodes non-minimal encodings`, same vector). Both encode
`u64::MAX` as the 9-byte `0xff…ff` form. Rust dropped the now-impossible `VarIntOverflow`
error variant (`model/error/parse.rs`); TS keeps `VarIntOverflowError` against
`MAX_VARINT_VALUE = 2n ** 64n - 1n` — correct asymmetry, since `bigint` is unbounded and
`u64` is not.

**So there is no varint task left in either language.** The one gap is that the two test
suites hardcode the same vectors independently rather than sharing a fixture — that is C-1
below, now reduced to locking in what already exists.

### ⚠️ Finding 1 — the wire is draft-18 but still advertises `moqt-16`

This is the most urgent item on the list, and it is a **regression introduced by the varint
commit**, not a migration step.

The varint change is wire-breaking, but the version string was not bumped with it. All three
sites still say `moqt-16`:

- `libs/moqtail-rs/src/model/control/constant.rs:19` — `SUPPORTED_VERSIONS = "moqt-16"`
- `libs/moqtail-ts/src/model/control/constant.ts:21` — `SUPPORTED_VERSIONS = ['moqt-16']`
- `apps/client/src/connection.rs:34` — `CLIENT_SUPPORTED_VERSIONS = "moqt-16"`

ALPN is what makes a flag day safe: mismatched peers should fail at the TLS handshake instead
of exchanging bytes. Right now that safety net is defeated — this build **claims** `moqt-16`,
negotiates successfully against a real draft-16 peer, and then produces garbage on the first
varint. The live case is concrete: `apps/client-js` defaults to `https://relay.moqtail.dev`
(`src/app.tsx:76,92`, `lib/player.ts:57`, `presets.json:4`). If that relay runs pre-`4dd968c`
code, a locally-built client-js now negotiates `moqt-16` with it and fails in a way that
looks like corruption rather than a version mismatch.

**Action: RS-2a below — bump all three to `moqt-18` now, ahead of everything else.** It is a
three-line change and it restores the clean-failure property for the rest of the migration.

### ⚠️ Finding 2 — five cancel/teardown messages must be deleted; my previous plan missed them

> 📎 **A.2 : 7341** — "Move requests to bidirectional streams; remove cancel messages (#1389)"

The previous revision covered "move requests to bidi streams" (RS-5) but silently dropped
"remove cancel messages." Draft-18 §3.3.2 replaces them with QUIC stream teardown: _"Implementations
SHOULD cancel requests by abruptly terminating any directions of a stream that are still open
by resetting or sending STOP_SENDING."_ Confirmed absent from draft-18 (0 occurrences each):

| Message                  | Rust `constant.rs` | Status in draft-18                          |
| ------------------------ | ------------------ | ------------------------------------------- |
| `Unsubscribe`            | `0x0A`             | gone — reset the SUBSCRIBE request stream   |
| `FetchCancel`            | `0x17`             | gone — reset the FETCH request stream       |
| `UnsubscribeNamespace`   | `0x14`             | gone — close the SUBSCRIBE_NAMESPACE stream |
| `PublishNamespaceCancel` | `0x0C`             | gone                                        |
| `PublishNamespaceDone`   | `0x09`             | gone (`NAMESPACE_DONE 0x0E` survives)       |

This is new task **RS-6b**. Both languages carry all five.

### ⚠️ Finding 3 — `SUBSCRIBE_NAMESPACE` must be renumbered `0x11` → `0x50`

Both languages have `SubscribeNamespace = 0x11` (`constant.rs:44`, `constant.ts:55`).
Draft-18 Table 5 assigns `0x50`, with `SUBSCRIBE_TRACKS = 0x51` alongside it. The previous
plan named `0x50` when describing the split but never flagged that the existing codepoint
changes — an easy way to ship a silent interop break. Folded into RS-2.

### ⚠️ Finding 4 — Request ID is removed from _responses_

> 📎 **A.1 : 7213** — "Remove Required Request ID (#1615)"

Not "Request ID is removed." Draft-18 §10.1: _"Only request messages include a Request ID;
response messages do not, since they are sent on the same stream."_ `SUBSCRIBE` still carries
`Request ID (vi64)`; `REQUEST_OK`, `SUBSCRIBE_OK`, `FETCH_OK` and `REQUEST_ERROR` no longer
do. All four carry `request_id` today (`subscribe_ok.rs:29`, `fetch_ok.rs:30`,
`request_ok.rs:26`, `publish_ok.rs:26`). New task **RS-7b**.

### ✏️ Corrections to the previous revision of this plan

1. **"Property side is already correct — only the Parameter side needs work"** (a claim carried
   over from the earlier code audit) is **wrong**. `TrackExtensionType`
   (`model/extension_header/constant.rs:55-63`) is missing `SUBGROUP_DELIVERY_TIMEOUT 0x06`
   entirely, and needs `DeliveryTimeout → ObjectDeliveryTimeout` plus
   `ImmutableExtensions → ImmutableProperties`. Both sides need work. See RS-9.
2. **"Drop the dead `ReservedClientSetupV10=0x40`/`ReservedServerSetupV10=0x41` slots"**
   (previous TS-2) is **wrong and would have been a bug**. Draft-18 Table 5 lists `0x01`,
   `0x40`, `0x41`, `0x20`, `0x21` as RESERVED. TS is _more_ correct than Rust here — it already
   has the first three. Keep them; add `0x20`/`0x21`; Rust should gain all five.
3. **"Add Track Properties to REQUEST_OK, SUBSCRIBE_OK and FETCH_OK"** overstated the work.
   `SubscribeOk` (`:32`) and `FetchOk` (`:34`) already carry `track_extensions` — after the
   RS-8 rename they are done. Only `RequestOk` genuinely lacks the field.
4. **Acceptance commands said `cargo test -p moqtail-rs`.** The crate is named `moqtail`;
   `-p moqtail-rs` errors out. Fixed throughout.
5. **`Switch = 0x22` was never mentioned.** See Finding 5.

### ⚠️ Finding 5 — `Switch = 0x22` is a moqtail extension with no home in draft-18

`ControlMessageType::Switch = 0x22` (`constant.rs:57`, `constant.ts:57`,
`model/control/switch.rs`) is this project's track-switching research extension — it is not in
the spec (0 occurrences of `SWITCH`). Two things make this worth a deliberate decision rather
than a silent carry-forward:

- Draft-18 has **no IANA registry for control message types** at all (§15 registers URI
  schemes, Setup Options, Message Parameters, Properties, auth token types — not message
  types). Table 5 is a closed space, so there is no sanctioned range to squat in. The
  application-specific ranges at **A.2 : 7397** (`0x78-0x7F`, `0x3800-0x3FFF` — see RS-18) are
  for **Property** types, not message types, so they do not authorise `0x22`.
- `0x22` is unassigned in Table 5 today, so nothing breaks now, but it is live in the
  neighbouring registries (`GROUP_ORDER` parameter, `DEFAULT_PUBLISHER_GROUP_ORDER` property),
  which makes it a plausible future assignment.

**Decided: keep `0x22` and document it as non-conformant — docs only, no code change (RS-21).**
There is no `SWITCH_FROM` in draft-18; it is a moqtail idea, so it is out of scope for a
draft-18 upgrade and any redesign is a separate effort. Note for whoever takes that on: making
it a Message Parameter would be _worse_ than `0x22`, since §10.2 makes unknown Message Parameters
a session-closing `PROTOCOL_VIOLATION` — a replacement must be negotiated via a Setup Option
(§3.2) or be a Property. Draft-18's own answer is neither: plain SUBSCRIBE at a lower priority
(line 1832). See RS-21.

---

## 1. Migration strategy

**Decided: development happens on `main`.** No integration branch, no dual-stacking. `main` is
the draft-18 branch as of `4dd968c`, and every task below lands there directly. This is a
research relay pre-1.0, so the cost of `main` being transiently non-interoperable with draft-16
peers is accepted deliberately.

That choice makes **RS-2a the first task and a genuine release blocker**, not a tidy-up. With
no branch isolating the work, `main` is the thing people build and deploy — and today it
advertises `moqt-16` while speaking draft-18 varints (Finding 1). Until the ALPN string is
bumped, every `main` build negotiates successfully with a draft-16 peer and then produces
garbage. Bumping it converts that silent corruption into a clean handshake refusal, which is
the property the rest of the migration leans on.

**Consequence to plan around:** between RS-2a and JS-2, `main` cannot talk to the deployed
`relay.moqtail.dev` unless that relay is also rebuilt from `main`. Redeploy the relay from
`main` at RS-2a, and treat CL-2 / JS-2 as the points where interop is re-established.

**Ordering still holds.** Rust first (RS → RL → CL → CL-2 gate), then JS (TS → JS/MT → JS-2
gate). The gate structure matters _more_ without a branch, since it is the only thing
distinguishing "`main` is mid-migration" from "`main` is broken."

**Workspace constraint.** `apps/relay`, `apps/client` and `libs/moqtail-rs` are one Cargo
workspace; `apps/client-js`/`apps/meet` resolve `moqtail` → `libs/moqtail-ts` through npm
workspaces (note: neither app declares the dep in its `package.json` — it resolves via the
workspace root). A library task that changes a public API cannot merge without updating its
consumers in the same PR. Tasks are categorised by _where the substantive work is_; each is
still responsible for `cargo build --workspace` / `npm run build` staying green.

---

## 2. Phase C — Common

### C-1 — Lock the varint vectors into a shared fixture; add the constants tables

- **Deliverable:** `dev/conformance/draft18/` — `varint.json` (the Table 2 vectors + the
  non-minimal cases both suites already test, now defined once), `message_types.json`
  (Table 5), `parameter_types.json` (§15.7), `property_types.json` (§15.8),
  `stream_reset_codes.json` (§3.3.3), `request_error_codes.json`, `termination_codes.json`.
- **Why:** the two stacks have already drifted once (Rust's `subscribe_options`, TS's missing
  `AUTHORITY`/`MOQT_IMPLEMENTATION`), and the varint vectors are now duplicated by hand in
  `varint.rs:143-156` and `byte_buffer.ts:576-582`. Codepoints change here first, never in one
  language only.
- **Acceptance:** both suites load the fixtures; the existing (passing) varint tests are
  rewritten to read `varint.json`; a test asserts each language's enums against the JSON so
  drift fails CI.
- **Depends on:** nothing. **Size:** S.

---

## 3. Phase RS — moqtail-rs

### ~~RS-1 — varint~~ ✅ DONE (`4dd968c`)

> 📎 **A.2 : 7346** — "New variable-length integer encoding (#1016)"
> 📎 **A.1 : 7294** — "Allow 7-byte varint and non-minimal encodings (#1595)"

Both bullets are satisfied in `libs/moqtail-rs/src/model/common/varint.rs`. Only C-1's fixture
extraction remains.

### RS-2a — 🔥 Bump ALPN to `moqt-18` (do this first)

- **Files:** `libs/moqtail-rs/src/model/control/constant.rs:19`;
  `apps/client/src/connection.rs:34`; `libs/moqtail-ts/src/model/control/constant.ts:21`
- **Why:** Finding 1 — the wire format is already draft-18; the advertised version is not.
- **Acceptance:** relay and client negotiate `moqt-18`; a `moqt-16` peer is **refused at the
  handshake** rather than failing on a parse. Add a test asserting the advertised ALPN matches
  the wire version the build implements.
- **Depends on:** nothing. **Size:** XS. Crosses into TS by design — this one is not worth
  splitting by language.

### RS-2 — Message type table → draft-18 (Table 5)

> 📎 **A.1 : 7320** — "Add stream type column to message type table (#1555)"
> 📎 **A.1 : 7231** — "Remove PUBLISH_OK message type, make it a REQUEST_OK alias (#1611)"

- **Files:** `libs/moqtail-rs/src/model/control/constant.rs:22-48` (enum + `TryFrom`)
- **Change:**
  - add `Setup = 0x2F00`; remove `ClientSetup 0x20` / `ServerSetup 0x21`
  - **renumber `SubscribeNamespace 0x11 → 0x50`** (Finding 3); add `SubscribeTracks = 0x51`
  - add `PublishBlocked = 0xF`
  - remove `MaxRequestId 0x15`, `RequestsBlocked 0x1A` (RS-6)
  - remove `PublishOk 0x1E` as a wire type (RS-7)
  - **keep `SubscribeOk 0x04` and `FetchOk 0x18`** — they remain distinct messages with their
    own bodies (§10.8, §10.13); only `PUBLISH_OK` becomes an alias of `REQUEST_OK 0x7`
  - add RESERVED markers for `0x01`, `0x40`, `0x41`, `0x20`, `0x21` (Correction 2)
  - record the Stream column (Control / Request / Request+First) — RS-5 enforces it
- **Acceptance:** enum matches `message_types.json`; a fixture-driven test fails on drift;
  `TryFrom` rejects every RESERVED codepoint.
- **Depends on:** C-1. **Size:** S. Lands with RS-3/RS-6/RS-6b/RS-7 or the tree won't compile.

### RS-3 — Collapse CLIENT_SETUP/SERVER_SETUP into SETUP

> 📎 **A.2 : 7330** — "Collapse CLIENT_SETUP and SERVER_SETUP into a single SETUP message (#1510)"
> 📎 **A.2 : 7425** — "Rename Setup Parameters to Setup Options (#1461)"
> 📎 **A.1 : 7314** — "Add IANA registry for Setup Options (#1564)"
> 📎 **A.2 : 7429** — "Add security/privacy considerations for MOQT_IMPLEMENTATION (#1511)"

- **Files:** `model/control/{client_setup.rs,server_setup.rs}` → `setup.rs`;
  `apps/relay/src/server/session.rs:833-851`; `apps/client/src/connection.rs:43-73`
- **Change:** one `Setup { setup_options: Vec<KeyValuePair> }` at `0x2F00`, sent by both peers.
  Rename `SetupParameter` → `SetupOption` throughout.
  **Smaller than it looks:** both structs already carry only parameters, and version
  negotiation already lives in ALPN (draft-18 §3.1: SETUP has no version fields). There is no
  negotiation logic to build — this is a merge plus a rename.
  Enforce client-only options: `AUTHORITY` and `PATH` MUST NOT arrive from a server, and
  `AUTHORITY` MUST NOT be used over WebTransport (§10.3.1.1) → `INVALID_AUTHORITY` /
  `INVALID_PATH`.
- **Acceptance:** `Setup` round-trips; one rejection test per client-only-option violation;
  `cargo build --workspace` green.
- **Depends on:** RS-2, RS-2a. **Size:** M.

### RS-4 — Control stream: one bidi → a pair of uni streams

> 📎 **A.2 : 7328** — "Change control stream from bidi to a pair of uni streams (#1510)"

- **Files:** `libs/moqtail-rs/src/transport/control_stream_handler.rs:27-180`;
  `apps/relay/src/server/session.rs:152-153,200-296`; `apps/client/src/connection.rs:67`
- **Change:** split `ControlStreamHandler` into independent send-half and recv-half; each peer
  opens its own uni stream (`open_uni`/`accept_uni`) and writes `SETUP` first.
- **Acceptance:** Rust client ↔ Rust relay handshake over two uni streams; SETUP asserted as
  first message each direction; any other first message → `PROTOCOL_VIOLATION`.
- **Depends on:** RS-3. **Size:** L. Spills into relay + client by necessity.

### RS-5 — Move the seven request types onto their own bidi request streams

> 📎 **A.2 : 7341** — "Move requests to bidirectional streams; remove cancel messages (#1389)" _(first half; second half is RS-6b)_
> 📎 **A.2 : 7371** — "Enforce REQUEST_OK/ERROR as first message on the response stream (#1499)"

- **Files:** `apps/relay/src/server/session.rs:402-519`;
  `apps/relay/src/server/message_handlers/mod.rs:39-216`
- **Change:** generalize the pattern **already working** for `SUBSCRIBE_NAMESPACE`
  (`handle_request_stream` / `dispatch_request_stream_message`) to `SUBSCRIBE`, `PUBLISH`,
  `FETCH`, `PUBLISH_NAMESPACE`, `TRACK_STATUS`, `SUBSCRIBE_TRACKS`. Delete those branches from
  the control-stream loop. Enforce `First`-marked types open a stream, and
  `REQUEST_OK`/`REQUEST_ERROR` is the first response message.
- **Acceptance:** all 7 round-trip on their own streams; a stream opening with a non-`First`
  type is reset; the control loop handles only `SETUP` and `GOAWAY`.
- **Depends on:** RS-4. **Size:** L — low design risk, proven template in-tree.

### RS-6 — Remove MAX_REQUEST_ID / REQUESTS_BLOCKED

> 📎 **A.2 : 7344** — "Remove MAX_REQUEST_ID/REQUESTS_BLOCKED (#1471)"

- **Files:** `model/control/{max_request_id.rs,requests_blocked.rs}`;
  `apps/relay/src/server/message_handlers/max_request_id_handler.rs`; gating at
  `message_handlers/mod.rs:57-66` and `session.rs:426-441`; `model/parameter/constant.rs`
  (`MaxRequestId = 0x02` setup option)
- **Acceptance:** no references remain; relay still bounds concurrency via QUIC stream limits
  (assert the limit is applied to the endpoint).
- **Depends on:** RS-5 — removing this earlier leaves the relay with no request flow control at
  all. **Size:** M.

### RS-6b — 🆕 Remove the cancel/teardown message family

> 📎 **A.2 : 7341** — "Move requests to bidirectional streams; remove cancel messages (#1389)" _(second half)_

- **Files:** `model/control/constant.rs` + the per-message modules for `Unsubscribe 0x0A`,
  `FetchCancel 0x17`, `UnsubscribeNamespace 0x14`, `PublishNamespaceCancel 0x0C`,
  `PublishNamespaceDone 0x09`; their relay handlers
- **Change:** delete all five (Finding 2). Replace each call site with QUIC stream teardown per
  §3.3.2 — reset / `STOP_SENDING` with a relevant RS-10 code. `NAMESPACE_DONE 0x0E` stays.
- **Acceptance:** none of the five codepoints parse; unsubscribing resets the SUBSCRIBE request
  stream and the publisher observes `CANCELLED 0x1`; cancelling a fetch resets the FETCH stream.
- **Depends on:** RS-5, RS-10 (needs the reset codes). **Size:** M.

### RS-7 — PUBLISH_OK → REQUEST_OK alias; Track Properties on REQUEST_OK

> 📎 **A.1 : 7231** — "Remove PUBLISH_OK message type, make it a REQUEST_OK alias (#1611)"
> 📎 **A.1 : 7236** — "Add Track Properties to REQUEST_OK (#1576)"
> 📎 **A.1 : 7312** — "Define textual aliases for REQUEST_OK by request type (#1610)"

- **Files:** `model/control/{publish_ok.rs,request_ok.rs}`; `message_handlers/mod.rs:100-104`
- **Change:** delete `PublishOk`; `PUBLISH` is answered by `RequestOk (0x7)`. Add the trailing
  `Track Properties` field to `RequestOk` — **only `RequestOk` needs it**; `SubscribeOk:32` and
  `FetchOk:34` already carry `track_extensions` (Correction 3). Per §10.5 Track Properties are
  populated **only** in `TRACK_STATUS_OK`; non-empty in `PUBLISH_OK`, `REQUEST_UPDATE_OK`,
  `SUBSCRIBE_NAMESPACE_OK` or `PUBLISH_NAMESPACE_OK` → `PROTOCOL_VIOLATION`. Keep per-type
  textual aliases for logging only.
- **Acceptance:** `PUBLISH` → `REQUEST_OK` round-trip; `SubscribeOk`/`FetchOk` still at
  `0x4`/`0x18` with their own bodies; the `PROTOCOL_VIOLATION` case is tested.
- **Depends on:** RS-5. **Size:** M.

### RS-7b — 🆕 Remove Request ID from response messages

> 📎 **A.1 : 7213** — "Remove Required Request ID (#1615)"

- **Files:** `model/control/{request_ok.rs:26,subscribe_ok.rs:29,fetch_ok.rs:30,request_error.rs}`
- **Change:** drop `request_id` from all four responses (Finding 4). The request stream
  identifies the request. `Request ID` stays in request messages (`SUBSCRIBE` etc.) and keeps
  its even/odd client/server parity rule (§10.1).
- **Acceptance:** responses round-trip without `request_id`; correlation is by stream, proven by
  two concurrent SUBSCRIBEs resolving to the right subscriptions; `INVALID_REQUEST_ID` still
  fires for a bad-parity Request ID on a _request_.
- **Depends on:** RS-5, RS-7. **Size:** M.

### RS-8 — Rename Extension Headers → Properties

> 📎 **A.2 : 7427** — "Rename Extension Headers to Properties (#1504)"
> 📎 **A.2 : 7416** — "Properties can appear in mutable list or inside Immutable Properties (#1442)"

- **Files:** `model/extension_header/` → `model/property/`; `TrackExtension`→`TrackProperty`,
  `ObjectExtension`→`ObjectProperty`, `TrackExtensionType`→`TrackPropertyType`,
  `ImmutableExtensions`→`ImmutableProperties`; call sites in `subscribe_ok.rs`, `fetch_ok.rs`,
  `publish.rs`, `data/{object,subgroup_object,fetch_object,datagram}.rs` (~15 files).
  `LOCHeaderExtensionId` (`constant.rs:21`) → `LOCPropertyId`.
- **Acceptance:** `cargo build --workspace` green; test count unchanged; no `extension_header`
  identifiers remain.
- **Depends on:** C-1. **Size:** S. **Land immediately after RS-2a** — it is a repo-wide rename
  and will conflict with everything scheduled around it.

### RS-9 — Timeout split, RENDEZVOUS_TIMEOUT, FILL_TIMEOUT, property renumbering

> 📎 **A.1 : 7285** — "Split DELIVERY_TIMEOUT into two types of timeout (#1605)"
> 📎 **A.1 : 7287** — "FILL_TIMEOUT parameter (#1490)"
> 📎 **A.2 : 7353** — "Add RENDEZVOUS_TIMEOUT parameter for SUBSCRIBE (#1447)"
> 📎 **A.2 : 7377** — "Allow DELIVERY_TIMEOUT value of 0 to mean no timeout (#1450)"
> 📎 **A.2 : 7401** — "Copy DELIVERY_TIMEOUT min requirement from parameter to property (#1427)"

- **Files:** `model/parameter/constant.rs` (`MessageParameterType`);
  `model/extension_header/constant.rs:55-63` (`TrackExtensionType`)
- **Change — Parameters** (vs §15.7; current enum has 9 of 13):
  | Type | Name | Status |
  |---|---|---|
  | `0x02` | `OBJECT_DELIVERY_TIMEOUT` | rename from `DeliveryTimeout` |
  | `0x03` | `AUTHORIZATION_TOKEN` | ✓ present |
  | `0x04` | `RENDEZVOUS_TIMEOUT` | **missing — add** |
  | `0x06` | `SUBGROUP_DELIVERY_TIMEOUT` | **missing — add** |
  | `0x08` `0x09` `0x10` `0x20` `0x21` `0x22` `0x32` | EXPIRES, LARGEST_OBJECT, FORWARD, SUBSCRIBER_PRIORITY, SUBSCRIPTION_FILTER, GROUP_ORDER, NEW_GROUP_REQUEST | ✓ present |
  | `0x0A` | `FILL_TIMEOUT` | **missing — add** (FETCH-only) |
  | `0x34` | `TRACK_NAMESPACE_PREFIX` | **missing — add** |
- **Change — Properties** (vs §15.8): **contrary to the earlier audit, this side is not already
  correct.** Add `SUBGROUP_DELIVERY_TIMEOUT 0x06`; rename `DeliveryTimeout → ObjectDeliveryTimeout`
  and `ImmutableExtensions → ImmutableProperties`. `MaxCacheDuration 0x04`,
  `DefaultPublisherPriority 0x0E`, `DefaultPublisherGroupOrder 0x22`, `DynamicGroups 0x30`,
  `PriorGroupIdGap 0x3C`, `PriorObjectIdGap 0x3E` are already right.
- **Acceptance:** both enums match `parameter_types.json` / `property_types.json`;
  `FILL_TIMEOUT` on a non-FETCH request rejected; `DELIVERY_TIMEOUT = 0` means no timeout; the
  property-side minimum matches the parameter-side minimum.
- **Depends on:** RS-8. **Size:** M.

### RS-10 — Stream reset error codes (generalized to all request streams)

> 📎 **A.1 : 7233** — "Generalize stream reset codes to all request streams, add new codes, align with PUBLISH_DONE (#1606)"
> 📎 **A.2 : 7360** — "Add GOING_AWAY to REQUEST_ERROR codes (#1434)"
> 📎 **A.2 : 7362** — "Add EXCESSIVE_LOAD error code (#1479)"
> 📎 **A.2 : 7367** — "Add TOO_FAR_BEHIND stream reset code (#1445)"
> 📎 **A.2 : 7364** — "Add NAMESPACE_TOO_LARGE error and stream reset for large namespaces (#1496)"

- **Files:** new `model/error/stream_reset.rs`; `transport/data_stream_handler.rs`;
  `model/control/constant.rs` (`RequestErrorCode`)

⚠️ **These are two separate registries and they disagree on the same names.** Do not merge them
into one enum — `GOING_AWAY` is `0x4` as a stream reset code and `0x6` as a request error code;
`EXPIRED_AUTH_TOKEN` is `0x7` vs `0x5`. Keep two types.

- **Change (a) — new Stream Reset codes** (§3.3.3), applicable to **any** request stream:
  `INTERNAL_ERROR 0x0`, `CANCELLED 0x1`, `DELIVERY_TIMEOUT 0x2`, `SESSION_CLOSED 0x3`,
  `GOING_AWAY 0x4`, `TOO_FAR_BEHIND 0x5`, `UNKNOWN_OBJECT_STATUS 0x6`, `EXPIRED_AUTH_TOKEN 0x7`,
  `EXCESSIVE_LOAD 0x9`, `MALFORMED_TRACK 0x12`. Retire the draft-16-only names
  (`GENERIC_ERROR`, `DEPENDS_ON_CANCELED_GROUP`, `TRACK_ENDED`, `SUBSCRIBER_PRIORITY_OUTDATED`).
- **Change (b) — extend `RequestErrorCode`** (`model/control/constant.rs`, §15 table). Verified
  present: `0x0`-`0x5`, `0x10`, `0x11`, `0x12`, `0x19`, `0x20`, `0x30`, `0x32`. **Missing —
  add five:**
  | Code | Name | Bullet |
  |---|---|---|
  | `0x6` | `GOING_AWAY` | A.2 : 7360 (#1434) |
  | `0x9` | `EXCESSIVE_LOAD` | A.2 : 7362 (#1479) |
  | `0x31` | `NAMESPACE_TOO_LARGE` | A.2 : 7364 (#1496) |
  | `0x33` | `UNSUPPORTED_EXTENSION` | A.1 : 7238 (#1509) |
  | `0x34` | `REDIRECT` | A.1 : 7215 (#1534) → RS-13 |
  plus `0x7f * N + 0x9D` reserved for greasing (RS-18).
- **Change (c) — build the reset path itself. No spike needed; the question is answered:**
  **there are no stream-reset call sites to wire codes into, because neither implementation ever
  resets a stream with an error code.** Verified:
  - **Rust:** `grep -rn "\.reset(\|stop_sending"` across `libs/moqtail-rs`, `apps/relay`,
    `apps/client` → **0 matches**. Every teardown is a graceful `.finish()` (FIN):
    `data_stream_handler.rs:257`, `apps/relay/src/server/client.rs:351`,
    `subscription.rs:854,1126`, `subscription_manager.rs:170`,
    `message_handlers/fetch_handler.rs:500,531,543`.
  - **TypeScript:** the only two `.abort()` calls are
    `client/publication/fetch.ts:111,124` and both pass a **string**
    (`'Fetch cancelled during publish'`), not a numeric code. Elsewhere it is
    `reader.cancel(<string>)` / `writer.close()` — e.g. `client.ts:1966`,
    `data_stream.ts:175,258`, `request_stream.ts:60`.

  So this task is not "add codes to existing resets" — it is **build the error-carrying
  teardown path that does not exist yet**, in both languages: a `reset_stream(code)` /
  `abort(code)` wrapper over quinn/wtransport and WebTransport respectively, plus the
  receive-side mapping from a QUIC reset code back to a typed error. That is why the estimate
  is L, not M — but the uncertainty is gone.

- **Blocks RS-6b.** Draft-18 cancels requests _by resetting the stream with a code_ (§3.3.2).
  Today the codebase cancels with dedicated messages and only ever finishes gracefully. The
  cancel messages cannot be deleted until this path exists.
- **Acceptance:** both enums match their fixtures; a test asserts `GOING_AWAY` is `0x4` as a
  reset code and `0x6` as a request error (guards the trap above); a real abort path (delivery
  timeout) resets with `0x2` **and the peer reads back the numeric code** — the round-trip is
  the point, since string reasons are what exists today; an over-long namespace → `0x31`.
- **Depends on:** RS-5. **Size:** L.

### RS-11 — Split SUBSCRIBE_NAMESPACE / SUBSCRIBE_TRACKS

> 📎 **A.1 : 7210** — "Split SUBSCRIBE_NAMESPACE into SUBSCRIBE_NAMESPACE and SUBSCRIBE_TRACKS (#1542)"
> 📎 **A.1 : 7254** — "Clarify SUBSCRIBE_NAMESPACE stream closure semantics (#1541)"
> 📎 **A.2 : 7436** — "Fix \"SUBSCRIBE_NAMESPACE with short prefixes\" (#1502)"

- **Files:** `model/control/subscribe_namespace.rs:29,37,43,79-87`; new `subscribe_tracks.rs`;
  `model/control/constant.rs:203` (`PrefixOverlap = 0x30`)
- **Change:** `SUBSCRIBE_NAMESPACE (0x50)` becomes pure namespace-advertisement discovery. New
  `SUBSCRIBE_TRACKS (0x51)` requests `PUBLISH` for all tracks under a matching prefix, present
  and future. **Independent overlap spaces** (§10.19): the two types may share a prefix without
  `PREFIX_OVERLAP`; two of the same type may not.
  **Rust-specific:** delete `subscribe_options: u64` (`:29`, values `PublishOnly 0x00` /
  `NamespaceOnly 0x01` / `Both 0x02`, fixtures at `:118,147,182`). It merges the two semantics
  into one message — the opposite of draft-18's direction — and TS never had it. This is where
  that Rust/TS divergence is reconciled.
- **Acceptance:** both messages round-trip at `0x50`/`0x51`; overlapping `SUBSCRIBE_NAMESPACE` +
  `SUBSCRIBE_TRACKS` on one prefix both succeed; two overlapping `SUBSCRIBE_TRACKS` →
  `PREFIX_OVERLAP`; no `subscribe_options` remains.
- **Depends on:** RS-5, RS-7. **Size:** L.

### RS-12 — Namespace bounds: 0–32 fields; encoding constraints

> 📎 **A.2 : 7379** — "Allow zero-element namespaces (#1472)"
> 📎 **A.2 : 7388** — "Constrain encoding/parsing of track namespace and names (#1512)"

- **Files:** `model/data/full_track_name.rs:56,97` (`ns_count == 0` rejection, both sites);
  `model/control/subscribe_namespace.rs:79-87`
- **Change:** drop the `== 0` rejection. Fix the pre-existing inconsistency where
  `subscribe_namespace.rs` checks only the upper bound while `full_track_name.rs` checks both.
- **Acceptance:** zero-element namespace round-trips; 33 fields still rejected (now with
  `NAMESPACE_TOO_LARGE`, RS-10); both validators agree.
- **Depends on:** RS-10, RS-11. **Size:** XS.

### RS-13 — GOAWAY: Request ID, per-request migration, REDIRECT

> 📎 **A.1 : 7229** — "Add Request ID to GOAWAY (#1559)"
> 📎 **A.1 : 7218** — "Allow GOAWAY on request streams to migrate individual requests (#1617)"
> 📎 **A.1 : 7215** — "Add REDIRECT for request errors and established subscriptions (#1534)"
> 📎 **A.2 : 7358** — "Add Timeout field to GOAWAY message (#1497)"

- **Files:** `model/control/goaway.rs:22-24`; new redirect struct (§10.6.1)
- **Change:** the struct is `GoAway { new_session_uri: Option<String> }` — **verified: both the
  `Timeout` field (#1497) and `Request ID` (#1559) are missing, so this task adds two fields, not
  one.** `request_id` is present only on the control stream ("the smallest peer Request ID that
  was not or might not have been processed prior to sending the GOAWAY"). Allow `GOAWAY` on a
  single request stream to migrate that request. Add the `Redirect` structure to `REQUEST_ERROR`
  (code `0x34`, RS-10b). `GOAWAY 0x10` becomes Control **and** Request per Table 5.
- **Acceptance:** round-trips with and without `request_id`; a `GOAWAY` on one request stream
  migrates only that request and leaves the session up; a redirect drives a retry at the new URI.
- **Depends on:** RS-5, RS-10 (`GOING_AWAY`). **Size:** M.

### RS-14 — FIRST_OBJECT bit on SUBGROUP_HEADER

> 📎 **A.1 : 7276** — "Add FIRST_OBJECT bit to SUBGROUP_HEADER type (#1618)"
> 📎 **A.2 : 7413** — "Clarify language for malformed tracks in a subgroup with END_OF_GROUP (#1464)"

- **Files:** `model/data/constant.rs:43-53` (`EXTENSIONS 0x01`, `END_OF_GROUP 0x08`,
  `REQUIRED_BIT 0x10`, `DEFAULT_PRIORITY 0x20`, `INVALID_BITS_MASK 0xC0` at `:53`);
  `subgroup_header.rs:33-106`
- **Change:** add `FIRST_OBJECT = 0x40`; narrow `INVALID_BITS_MASK` `0xC0 → 0x80` (`0x40` is
  rejected outright today at `:59`). Rename `EXTENSIONS` → `PROPERTIES` (§11: the `0x01` bit is
  now the Properties bit) per RS-8. Valid form is `0b0XX1XXXX` (`0x10-0x1F`, `0x30-0x3F`,
  `0x50-0x5F`, `0x70-0x7F`); `SUBGROUP_ID_MODE == 0b11` reserved → `0x16,0x17,0x1E,0x1F,0x36,
0x37,0x3E,0x3F,0x56,0x57,0x5E,0x5F,0x76,0x77,0x7E,0x7F` MUST be `PROTOCOL_VIOLATION`. Set the
  bit on the original publisher's first-object-in-subgroup path.
- **Acceptance:** table-driven test over all 256 type bytes classifying valid/invalid;
  publisher sets `FIRST_OBJECT` on a new subgroup, not on a reopened one.
- **Depends on:** RS-8. **Size:** M. Parallelisable with the session work.

### RS-15 — Delta-encode Group/Object ID in FETCH responses; close on wrap

> 📎 **A.1 : 7273** — "Make Object ID and Group ID delta encoded in Fetch responses (#1586)"
> 📎 **A.1 : 7298** — "Close session when delta encoding wraps (#1560)"

- **Files:** `model/data/fetch_object.rs`
- **Change:** Object ID = `previous + delta + 1` within a subgroup stream (delta alone for the
  first); same for Group ID in fetch responses. Close the session if a delta would exceed
  `2^64 - 1`.
- **Acceptance:** multi-object fetch round-trips; a wrapping delta closes the session.
- **Depends on:** RS-8. **Size:** M.

### RS-16 — PUBLISH_BLOCKED

> 📎 **A.2 : 7355** — "Add PUBLISH_BLOCKED message for SUBSCRIBE_NAMESPACE flow control (#1452)"

- **Files:** new `model/control/publish_blocked.rs` (`0xF`)
- **Change:** net-new. Sent on the `SUBSCRIBE_TRACKS` response stream when the publisher is
  blocked by the peer's bidi stream limit; MUST NOT send `PUBLISH` for that track afterwards
  until unblocked (§10.20).
- **Acceptance:** round-trips; a relay at its stream limit emits it instead of stalling.
- **Depends on:** RS-11. **Size:** S.

### RS-17 — Deferred data-plane additions

> 📎 **A.2 : 7399** — "Make EndGroup in Subscription Filters a delta (#1470)"
> 📎 **A.1 : 7291** — "Allow publisher to reopen subgroup after REQUEST_UPDATE forward 0->1 (#1583)"
> 📎 **A.1 : 7296** — "Padding streams and datagrams (#1475)"
> 📎 **A.1 : 7242** — "Add Session-Level Tracks reserved namespace (#1562)"
> 📎 **A.2 : 7409** — "Clarify datagram status and properties cases (#1444)"

- **Change:** additive; none block interop on the core request/response paths. `.session`
  needs RS-12's zero-element namespaces.
- **Depends on:** RS-12. **Size:** M. **Defer past CL-2** unless a demo needs one.

### RS-18 — 🆕 GREASE

> 📎 **A.2 : 7350** — "Add GREASE for Setup Options, Properties, and error code registries (#1460)"
> 📎 **A.2 : 7397** — "Reserve Property type ranges for application-specific use (#1473)"
> 📎 **A.1 : 7322** — "Fix grease examples to match 0x7f multiplier (#1569)"

- **Change:** absent from both codebases (0 matches for `grease`). Reserve `0x7f * N + 0x9D`
  (§14) in the Property registry; ignore-unknown behaviour for greased Setup Options,
  Properties and error codes; reserve the application-specific Property ranges below.
- **Property registry ranges — use §15.8 (line 6693-6720), not §2.5:**
  | Range | Policy |
  |---|---|
  | `0x00`–`0x77` | Standards Action or IESG Approval (1-byte) |
  | **`0x78`–`0x7F`** | **application-specific, no registration permitted** (1-byte) |
  | `0x80`–`0x37FF` | Specification Required (2-byte) |
  | `0x3800`–`0x3FFF` | application-specific, no registration permitted (2-byte) |
  | `0x4000`–`0x7FFF` | Mandatory Track Properties (RL-1); Track scope only |
  | `0x8000`+ | First Come First Served |

  ⚠️ §2.5 (line 1188) contradicts this, giving the 1-byte range as `0x38-0x3F`. **§15.8 is the
  correct one**, and the reason is worth knowing: draft-18's new varint (RS-1) puts **7 usable
  bits in a 1-byte encoding**, so the 1-byte space is `0x00-0x7F` and the top 8 code points are
  `0x78-0x7F`. `0x38-0x3F` is the top 8 of a **6-bit** space (`0x00-0x3F`) — i.e. the _old_ QUIC
  varint's 1-byte range. §2.5 is stale text that was never updated when #1016 changed the varint
  encoding. §15.8's whole table is internally consistent with the new scheme (2-byte starts at
  `0x80`, ends at `0x3FFF` = 14 bits), which confirms it. **File upstream against
  `moq-wg/moq-transport`: §2.5's application-specific range predates #1016.**

- **Acceptance:** a greased Setup Option / Property / error code is ignored, not fatal; a peer
  emitting grease values interops; range constants match `property_types.json`, which encodes
  the §15.8 table above.
- **Depends on:** RS-9. **Size:** S.

### RS-19 — 🆕 Remove TRACK_STATUS from REQUEST_UPDATE

> 📎 **A.2 : 7383** — "Remove TRACK_STATUS from REQUEST_UPDATE (#1436)"
> 📎 **A.2 : 7369** — "Add REQUEST_UPDATE to list of REQUEST_ERROR causes (#1466)"
> 📎 **A.1 : 7251** — "Clarify REQUEST_UPDATE failure behavior for all request types (#1539)"

- **Files:** `model/control/request_update.rs`; relay update handling
- **Acceptance:** `REQUEST_UPDATE` on a `TRACK_STATUS` request is rejected with `REQUEST_ERROR`;
  update failures are per-request-type per §10.9.
- **Depends on:** RS-5. **Size:** S.

### RS-20 — 🆕 Auth token cache across multiple streams

> 📎 **A.2 : 7385** — "Define how to use auth token cache safely with multiple streams (#1430)"

- **Change:** with requests on independent streams (RS-5), the auth-token alias cache is shared
  across streams with no ordering guarantee between them. Define/enforce safe use per §10.2.2 —
  the relevant termination codes (`AUTH_TOKEN_CACHE_OVERFLOW 0x13`,
  `DUPLICATE_AUTH_TOKEN_ALIAS 0x14`, `UNKNOWN_AUTH_TOKEN_ALIAS 0x17`,
  `EXPIRED_AUTH_TOKEN 0x18`) already exist in `model/error/termination.rs:19-41`.
- **Acceptance:** an alias registered on one request stream resolves on another; cache overflow
  → `0x13`; duplicate alias → `0x14`.
- **Depends on:** RS-5. **Size:** M. **Direct consequence of RS-5 — do not skip.**

### RS-21 — Document `Switch = 0x22` as a non-conformant local extension

**Decided: docs only. `Switch = 0x22` stays and keeps working; no replacement in this upgrade.**
There is no `SWITCH_FROM` in draft-18 (0 occurrences of `SWITCH`) — it is a moqtail idea, not a
spec feature, so it is out of scope here. Any redesign is a separate effort after JS-2.

- **Files:** `docs/` only. **No code change.** `model/control/switch.rs`, `constant.rs:57` and
  `constant.ts:57` are untouched and `Switch` keeps its current behaviour through the migration.
- **Change:** record in `docs/` that `Switch = 0x22` is a moqtail-local control message, not part
  of draft-18, and specifically that:
  - Draft-18 has **no IANA registry for control message types at all** (§15 registers URI
    schemes, Setup Options, Message Parameters, Properties and auth token types — not message
    types). Table 5 is a closed, spec-defined space, so there is no application-specific or
    provisional range to move into. This is _why_ it stays non-conformant rather than being
    relocated to a "correct" codepoint — no such codepoint exists.
  - `0x22` is unassigned in Table 5 today so nothing breaks now, but it is live in the
    neighbouring registries (`GROUP_ORDER` parameter, `DEFAULT_PUBLISHER_GROUP_ORDER` property),
    which makes it a plausible future assignment and therefore a known collision risk.
  - A conformant draft-18 peer receiving `0x22` closes the session (§10: "An endpoint that
    receives an unknown message type MUST close the session"). So the extension is only usable
    moqtail-to-moqtail — an accepted limitation, recorded rather than fixed.
- **Constraint worth capturing for whoever picks up the redesign:** the obvious "make it a
  parameter instead" move is a trap. The three registries treat unknown values completely
  differently — unknown **Setup Options** are ignored (§10.3), unknown **Properties** are skipped
  via their length field (§15.8), but unknown **Message Parameters** are a session-closing
  `PROTOCOL_VIOLATION` (§10.2: _"there is no need for a mechanism to skip unknown parameters"_).
  A bare parameter would be **worse** than `0x22`. Any future replacement must either be
  negotiated via a Setup Option (§3.2 Extension Negotiation) or be a Property in the
  application-specific range. Draft-18's own answer to the problem is neither: line 1832 says
  _"Subscribers wanting to switch to an alternate representation of a Track can subscribe to it
  at a lower priority"_ — plain SUBSCRIBE plus priority, no message at all.
- **Acceptance:** `docs/` states the above; `Switch` still round-trips and switching still works
  moqtail-to-moqtail after CL-2; no code changed.
- **Depends on:** nothing. **Size:** XS. Can land immediately.

---

## 4. Phase RL — relay

Most relay work is pulled in by RS-3..RS-7 out of compile necessity. These carry _behaviour_
the library cannot.

### RL-1 — Mandatory Track Properties (`0x4000–0x7FFF`)

> 📎 **A.1 : 7238** — "Add support for mandatory-to-understand track extensions (#1509)"
> 📎 **A.2 : 7419** — "Clarify immutable property preservation requirements (#1441)"

- **Change:** unknown ordinary properties are cached and forwarded unmodified; an unknown
  property in `0x4000–0x7FFF` MUST be rejected, not blind-forwarded (§2.5). New logic, not part
  of the RS-8 rename. Immutable properties preserved exactly across the relay.
- **Acceptance:** unknown property outside the range forwarded verbatim; inside the range
  rejected; both asserted at the subscriber; immutable properties byte-identical end to end.
- **Depends on:** RS-8. **Size:** M.

### RL-2 — SUBSCRIBE_TRACKS handling + PUBLISH_BLOCKED emission

> 📎 **A.1 : 7210** — "Split SUBSCRIBE_NAMESPACE into SUBSCRIBE_NAMESPACE and SUBSCRIBE_TRACKS (#1542)"
> 📎 **A.2 : 7355** — "Add PUBLISH_BLOCKED message for SUBSCRIBE_NAMESPACE flow control (#1452)"

- **Acceptance:** a subscriber issuing `SUBSCRIBE_TRACKS` for a prefix receives `PUBLISH` for
  every existing matching track and for one published afterwards; `PUBLISH_BLOCKED` when out of
  streams.
- **Depends on:** RS-11, RS-16. **Size:** L.

### RL-3 — SUBSCRIBE precedence; exclude own tracks

> 📎 **A.1 : 7246** — "SUBSCRIBE takes precedence over SUBSCRIBE_NAMESPACE at relay (#1533)"
> 📎 **A.1 : 7240** — "Exclude your own tracks from SUBSCRIBE_NAMESPACE (#1596)"

- **Acceptance:** a relay holding both an explicit `SUBSCRIBE` and a matching
  `SUBSCRIBE_NAMESPACE` serves the `SUBSCRIBE` path; a subscriber never receives its own
  published tracks back.
- **Depends on:** RS-11. **Size:** M.

### RL-4 — Truthful LARGEST_OBJECT; tolerate unknown errors

> 📎 **A.1 : 7289** — "Forbid relays from lying about LARGEST_OBJECT (#1621)"
> 📎 **A.1 : 7249** — "Don't close the Session for unknown errors (#1561)"
> 📎 **A.1 : 7302** — "Clarify Object existence and cross-source contradictions (#1566)"

- **Acceptance:** a relay forwarding a partially-cached track reports a truthful largest
  location; an unknown `REQUEST_ERROR` code leaves the session open.
- **Depends on:** RS-7. **Size:** M.

### RL-5 — Reset codes on abort paths; GOAWAY per-request migration

> 📎 **A.1 : 7233** — "Generalize stream reset codes to all request streams, add new codes, align with PUBLISH_DONE (#1606)"
> 📎 **A.1 : 7218** — "Allow GOAWAY on request streams to migrate individual requests (#1617)"

- **Acceptance:** load shedding resets with `EXCESSIVE_LOAD 0x9`; a slow subscriber gets
  `TOO_FAR_BEHIND 0x5`; draining sends `GOAWAY` and rejects new requests with `GOING_AWAY`.
- **Depends on:** RS-10, RS-13. **Size:** M.

### RL-6 — 🆕 REQUEST_UPDATE coalescing and EXPIRES

> 📎 **A.1 : 7244** — "Allow coalescing REQUEST_UPDATE processing (#1540)"
> 📎 **A.2 : 7381** — "Clarify EXPIRES parameter update mechanism (#1448)"

- **Change:** a relay MAY coalesce several queued `REQUEST_UPDATE`s for one request and process
  only the latest. `EXPIRES` updates follow §10.2.10.
- **Acceptance:** three rapid updates produce at most one applied state change, matching the
  last; `EXPIRES` refresh extends a subscription without re-subscribing.
- **Depends on:** RS-19. **Size:** M.

### RL-7 — 🆕 FETCH semantics

> 📎 **A.1 : 7256** — "FETCH to a track with no objects returns INVALID_RANGE (#1537)"
> 📎 **A.1 : 7258** — "Clarify FETCH_OK End Location semantics (#1536)"
> 📎 **A.1 : 7262-7268** — "Clarify Joining Fetch behavior:" — "Joining FETCH is unaffected by forward changing to 0 (#1620)" / "Joining Fetch forward state mismatch is a request error (#1609)" / "Clarify Joining Fetch ordering with Forward State transitions (#1577)"
> 📎 **A.2 : 7374** — "Allow joining FETCH for PUBLISH and REQUEST_UPDATE with forward=1 (#1335)"
> 📎 **A.2 : 7406** — "Clarify prior Object semantics after End of Range indicators in FETCH (#1513)"
> 📎 **A.2 : 7411** — "Clarify Stream Count includes empty subgroups (#1449)"

- **Change:** FETCH on an empty track → `INVALID_RANGE`; `FETCH_OK` End Location clamped to
  `{Largest.Group, Largest.Object + 1}` when the request overruns published data;
  `End Location < Start Location` → `PROTOCOL_VIOLATION`; joining-fetch forward-state rules;
  Stream Count includes empty subgroups.
- **Acceptance:** one test per bullet above.
- **Depends on:** RS-7b, RS-15. **Size:** L. Largest single cluster of unmapped bullets — worth
  its own epic.

---

## 5. Phase CL — client + the Rust interop gate

### CL-1 — `moqt://` as the unified scheme

> 📎 **A.1 : 7206** — "Unified moqt:// URI scheme for QUIC and WebTransport (#1486)"
> 📎 **A.1 : 7208** — "Add fragment identifier support for moqt URIs (#1571)"

- **Files:** `apps/client/src/connection.rs:45,59,108-160,217-227`
- **Change:** native QUIC already parses `moqt://` (`strip_prefix("moqt://")`,
  `split_authority_and_path`); the WebTransport path still takes a plain `https://` URL. Unify:
  `moqt://` is the single input scheme, authority/path feed the `AUTHORITY`/`PATH` setup
  options, transport per §3.1 (offer MOQT ALPNs + `h3`; `h3` → WebTransport). Add fragments
  (`moqt://host/app#<type>:<value>`).
- **Acceptance:** one `moqt://` URL connects over both native QUIC and WebTransport;
  `AUTHORITY` is not sent over WebTransport.
- **Depends on:** RS-3. **Size:** M.

### CL-2 — 🚦 Rust interop gate

- **Change:** no features. CI integration test: Rust client publishes to the Rust relay, a
  second Rust client subscribes and receives objects — two uni control streams, per-request
  bidi streams, `moqt-18` ALPN, draft-18 varint.
- **Acceptance:** green on `main` over **both** native QUIC and WebTransport;
  `cargo test --workspace` green; manual: `dev/source.mp4` plays through the relay via
  `apps/client`.
- **Depends on:** all RS-_, RL-_, CL-1. **Size:** M.
- **This gate is the point of the Rust-first ordering.** Once green, the relay is a known-good
  draft-18 peer, so every JS failure below is a JS bug rather than an ambiguity about who is wrong.

---

## 6. Phase TS — moqtail-ts

Mirrors the RS tasks; changelog citations are identical to the Rust twin. Each TS task's
acceptance includes **interop against the CL-2 relay**, not just unit tests — that is what the
Rust-first order buys.

| Task          | Mirrors | Notes                                                                                                                                                                                                                                                                                                                                    |
| ------------- | ------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| ~~**TS-1**~~  | RS-1    | ✅ **DONE** in `4dd968c` — `byte_buffer.ts:80-113`. Not outstanding.                                                                                                                                                                                                                                                                     |
| **TS-2**      | RS-2    | `model/control/constant.ts:42-58`. Renumber `SubscribeNamespace 0x11 → 0x50`; add `Setup 0x2F00`, `SubscribeTracks 0x51`, `PublishBlocked 0xF`. **Keep** `ReservedSetupV00 0x01`, `ReservedClientSetupV10 0x40`, `ReservedServerSetupV10 0x41` (Correction 2) and add `0x20`/`0x21` as reserved.                                         |
| **TS-3**      | RS-3    | `model/control/{client_setup.ts,server_setup.ts}` → `setup.ts`; handshake `client/client.ts:479-485`. **Also add the missing `AUTHORITY (0x05)` and `MOQT_IMPLEMENTATION (0x07)` setup options** — a TS-only gap (`model/parameter/constant.ts:19-24`); Rust has both.                                                                   |
| **TS-4**      | RS-4    | `client/control_stream.ts:33-221`, `client/client.ts:472` → `createUnidirectionalStream()` + `incomingUnidirectionalStreams`.                                                                                                                                                                                                            |
| **TS-5**      | RS-5    | Generalize the existing `RequestStream` (`client/request_stream.ts:27-124`, already used for `SUBSCRIBE_NAMESPACE` at `client.ts:1743-1745`) to the other six. Strip them from `#handleIncomingControlMessages` (`:1817-1832`); call sites `:1001,1084,1169,1224,1389,1450,1491,1524,1629,1683,1721,1808`.                               |
| **TS-6**      | RS-6    | Delete `model/control/{max_request_id.ts,requests_blocked.ts}`, `client/handler/{max_request_id.ts,requests_blocked.ts}`, registrations `handler.ts:22-23,77,81`, `model/parameter/setup/max_request_id.ts`.                                                                                                                             |
| **TS-6b**     | RS-6b   | Delete `Unsubscribe 0x0a`, `FetchCancel 0x17`, `UnsubscribeNamespace 0x14`, `PublishNamespaceCancel 0x0c`, `PublishNamespaceDone 0x09` (`constant.ts:53-56`) + handlers.                                                                                                                                                                 |
| **TS-7**      | RS-7    | `model/control/{publish_ok.ts,request_ok.ts}`. Keep `SubscribeOk 0x4`/`FetchOk 0x18`.                                                                                                                                                                                                                                                    |
| **TS-7b**     | RS-7b   | Remove `requestId` from `RequestOk`/`SubscribeOk`/`FetchOk`/`RequestError`.                                                                                                                                                                                                                                                              |
| **TS-8**      | RS-8    | `model/extension_header/` → `model/property/` (8 files incl. `audio_level.ts`, `capture_time_stamp.ts`, `video_config.ts`, `video_frame_marking.ts`). **Public API break** — feeds `client-js`/`meet`; JS-1/MT-1 absorb it.                                                                                                              |
| **TS-9**      | RS-9    | `model/parameter/constant.ts:41-51` **and** `model/extension_header/constant.ts:26-35` — same both-sides correction as RS-9.                                                                                                                                                                                                             |
| **TS-10**     | RS-10   | Two enums (reset codes + `RequestErrorCode`) **and** the reset path itself. Today the only `.abort()` calls are `client/publication/fetch.ts:111,124` and both pass a **string**, not a code; `reader.cancel(<string>)` elsewhere. Needs `abort(code)` plumbing over WebTransport + receive-side `streamErrorCode` mapping. **Size:** L. |
| **TS-11**     | RS-11   | `model/control/subscribe_namespace.ts` + new `subscribe_tracks.ts`. TS has **no** `subscribe_options`, so purely additive here; the divergence is resolved Rust-side.                                                                                                                                                                    |
| **TS-12**     | RS-12   | `model/data/full_track_name.ts:21,84,120`. TS has no `== 0` guard — add the zero-length regression test rather than removing a check.                                                                                                                                                                                                    |
| **TS-13**     | RS-13   | `model/control/goaway.ts:21-87`.                                                                                                                                                                                                                                                                                                         |
| **TS-14**     | RS-14   | `model/data/constant.ts:200-311`; `INVALID_BITS_MASK 0xc0` at `:244`; `subgroup_header.ts:22-86`.                                                                                                                                                                                                                                        |
| **TS-15**     | RS-15   | Fetch object delta decode.                                                                                                                                                                                                                                                                                                               |
| **TS-16**     | RS-16   | New `publish_blocked.ts`.                                                                                                                                                                                                                                                                                                                |
| **TS-18**     | RS-18   | GREASE.                                                                                                                                                                                                                                                                                                                                  |
| **TS-19**     | RS-19   | `REQUEST_UPDATE` / `TRACK_STATUS`.                                                                                                                                                                                                                                                                                                       |
| **TS-20**     | RS-20   | Auth token cache across streams.                                                                                                                                                                                                                                                                                                         |
| ~~**TS-21**~~ | RS-21   | No TS task — RS-21 is docs-only and `Switch = 0x22` (`constant.ts:57`) is unchanged.                                                                                                                                                                                                                                                     |

### TS-17 — Fix `TerminationCode.tryFrom` (pre-existing bug, not migration)

- **Files:** `libs/moqtail-ts/src/model/error/constant.ts:50-86`
- **Change:** the `switch` throws on the valid enum values `MALFORMED_AUTH_TOKEN`,
  `UNKNOWN_AUTH_TOKEN_ALIAS`, `EXPIRED_AUTH_TOKEN`, `INVALID_AUTHORITY`, `MALFORMED_AUTHORITY` —
  they fall to `default: throw`. The enum already has the full draft-18 set (`:19-41`), as does
  Rust (`model/error/termination.rs:19-41`), so **no termination-code work is needed for the
  migration** beyond this bug.
- **Acceptance:** `tryFrom` round-trips every member.
- **Depends on:** nothing — **ship to `main` now**, independent of draft-18. **Size:** XS.

---

## 7. Phase JS — client-js

### JS-1 — Absorb the moqtail-ts API break

- **Files:** `apps/client-js/src/lib/{publisher.ts,player.ts,utils.ts}`, `src/app.tsx`, `src/types.ts`
- **Acceptance:** `npm --prefix apps/client-js run build` green; typecheck clean.
- **Depends on:** TS-2..TS-21. **Size:** S.

### JS-2 — 🚦 `moqt://` as the real transport scheme + JS interop gate

> 📎 **A.1 : 7206** — "Unified moqt:// URI scheme for QUIC and WebTransport (#1486)"
> 📎 **A.1 : 7208** — "Add fragment identifier support for moqt URIs (#1571)"

- **Files:** `apps/client-js/src/lib/msf-url.ts:21,66,95,101,117`; `src/app.tsx:76,92`;
  `src/lib/player.ts:57`; `src/presets.json:4`
- **Change:** today `moqt://` is **cosmetic only** — `msf-url.ts` builds
  `moqt://host/path#msf:...` share links and converts them back to `https://` at `:95` before
  connecting; the real connection is always `https://relay.moqtail.dev` via
  `new WebTransport(url)` (`libs/moqtail-ts/src/client/client.ts:458`). Make `moqt://` the
  scheme actually connected with, mirroring CL-1, keeping `#msf:` working on draft-18 fragments.
- **Acceptance:** browser client-js publishes to and subscribes from the **CL-2 draft-18 relay**,
  video renders end to end; existing `moqt://…#msf:` links still resolve. This gate is where
  `main` is interoperable again for JS clients.
- **Depends on:** JS-1, CL-2, TS-3. **Size:** M.

---

## 8. Phase MT — meet

### MT-1 — Absorb the moqtail-ts API break

- **Acceptance:** `npm --prefix apps/meet run build` green.
- **Depends on:** TS-2..TS-21. **Size:** S.

### MT-2 — Adopt `moqt://`

> 📎 **A.1 : 7206** — "Unified moqt:// URI scheme for QUIC and WebTransport (#1486)"

- **Change:** `apps/meet/src` has **no** `moqt://` or scheme handling at all — unlike client-js
  there is no cosmetic layer to convert, so this is net-new.
- **Acceptance:** meet joins a room through the draft-18 relay; multi-party A/V works; both
  participants negotiate `moqt-18`.
- **Depends on:** MT-1, JS-2. **Size:** M.

---

## 9. Reverse coverage map — every changelog bullet → task

Read this the other way round to confirm nothing is dropped. Line numbers index
`docs/draft-ietf-moq-transport-18.txt`.

### A.1 — Since draft-ietf-moq-transport-17 (line 7202)

| Line | Bullet                                                                          | Task                                       |
| ---- | ------------------------------------------------------------------------------- | ------------------------------------------ |
| 7206 | Unified moqt:// URI scheme for QUIC and WebTransport (#1486)                    | CL-1, JS-2, MT-2                           |
| 7208 | Add fragment identifier support for moqt URIs (#1571)                           | CL-1, JS-2                                 |
| 7210 | Split SUBSCRIBE_NAMESPACE into SUBSCRIBE_NAMESPACE and SUBSCRIBE_TRACKS (#1542) | RS-11, TS-11, RL-2                         |
| 7213 | Remove Required Request ID (#1615)                                              | **RS-7b**, TS-7b                           |
| 7215 | Add REDIRECT for request errors and established subscriptions (#1534)           | RS-13, TS-13                               |
| 7218 | Allow GOAWAY on request streams to migrate individual requests (#1617)          | RS-13, RL-5                                |
| 7229 | Add Request ID to GOAWAY (#1559)                                                | RS-13, TS-13                               |
| 7231 | Remove PUBLISH_OK message type, make it a REQUEST_OK alias (#1611)              | RS-2, RS-7                                 |
| 7233 | Generalize stream reset codes to all request streams… (#1606)                   | RS-10, RL-5                                |
| 7236 | Add Track Properties to REQUEST_OK (#1576)                                      | RS-7                                       |
| 7238 | Add support for mandatory-to-understand track extensions (#1509)                | RL-1, RS-10 (`UNSUPPORTED_EXTENSION 0x33`) |
| 7240 | Exclude your own tracks from SUBSCRIBE_NAMESPACE (#1596)                        | RL-3                                       |
| 7242 | Add Session-Level Tracks reserved namespace (#1562)                             | RS-17                                      |
| 7244 | Allow coalescing REQUEST_UPDATE processing (#1540)                              | **RL-6**                                   |
| 7246 | SUBSCRIBE takes precedence over SUBSCRIBE_NAMESPACE at relay (#1533)            | RL-3                                       |
| 7249 | Don't close the Session for unknown errors (#1561)                              | RL-4                                       |
| 7251 | Clarify REQUEST_UPDATE failure behavior for all request types (#1539)           | RS-19                                      |
| 7254 | Clarify SUBSCRIBE_NAMESPACE stream closure semantics (#1541)                    | RS-11                                      |
| 7256 | FETCH to a track with no objects returns INVALID_RANGE (#1537)                  | **RL-7**                                   |
| 7258 | Clarify FETCH_OK End Location semantics (#1536)                                 | **RL-7**                                   |
| 7260 | Clarify definition of scope (#1629)                                             | — editorial, no action                     |
| 7262 | Clarify Joining Fetch behavior:                                                 | **RL-7**                                   |
| 7264 | ↳ Joining FETCH is unaffected by forward changing to 0 (#1620)                  | **RL-7**                                   |
| 7266 | ↳ Joining Fetch forward state mismatch is a request error (#1609)               | **RL-7**                                   |
| 7268 | ↳ Clarify Joining Fetch ordering with Forward State transitions (#1577)         | **RL-7**                                   |
| 7273 | Make Object ID and Group ID delta encoded in Fetch responses (#1586)            | RS-15, TS-15                               |
| 7276 | Add FIRST_OBJECT bit to SUBGROUP_HEADER type (#1618)                            | RS-14, TS-14                               |
| 7285 | Split DELIVERY_TIMEOUT into two types of timeout (#1605)                        | RS-9, TS-9                                 |
| 7287 | FILL_TIMEOUT parameter (#1490)                                                  | RS-9, TS-9                                 |
| 7289 | Forbid relays from lying about LARGEST_OBJECT (#1621)                           | RL-4                                       |
| 7291 | Allow publisher to reopen subgroup after REQUEST_UPDATE forward 0->1 (#1583)    | RS-17                                      |
| 7294 | Allow 7-byte varint and non-minimal encodings (#1595)                           | ✅ **RS-1 + TS-1 DONE**                    |
| 7296 | Padding streams and datagrams (#1475)                                           | RS-17                                      |
| 7298 | Close session when delta encoding wraps (#1560)                                 | RS-15                                      |
| 7302 | Clarify Object existence and cross-source contradictions (#1566)                | RL-4                                       |
| 7304 | Clarify immutable track properties (#1535)                                      | RL-1                                       |
| 7306 | Improve Startup Latency and 0-RTT guidance (#1544)                              | — editorial                                |
| 7308 | Improve Security Considerations section (#1625)                                 | — editorial                                |
| 7310 | Rewrite abstract and introduction (#1556)                                       | — editorial                                |
| 7312 | Define textual aliases for REQUEST_OK by request type (#1610)                   | RS-7                                       |
| 7314 | Add IANA registry for Setup Options (#1564)                                     | RS-3                                       |
| 7316 | Add provisional registry for LOC properties (#1624)                             | RS-8 (`LOCHeaderExtensionId`)              |
| 7318 | Update MOQ Properties registration policies (#1525)                             | — editorial                                |
| 7320 | Add stream type column to message type table (#1555)                            | RS-2                                       |
| 7322 | Fix grease examples to match 0x7f multiplier (#1569)                            | RS-18                                      |

### A.2 — Since draft-ietf-moq-transport-16 (line 7324)

| Line | Bullet                                                                        | Task                                                                                           |
| ---- | ----------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------- |
| 7328 | Change control stream from bidi to a pair of uni streams (#1510)              | RS-4, TS-4                                                                                     |
| 7330 | Collapse CLIENT_SETUP and SERVER_SETUP into a single SETUP message (#1510)    | RS-3, TS-3                                                                                     |
| 7341 | Move requests to bidirectional streams; remove cancel messages (#1389)        | RS-5 **+ RS-6b**                                                                               |
| 7344 | Remove MAX_REQUEST_ID/REQUESTS_BLOCKED (#1471)                                | RS-6, TS-6                                                                                     |
| 7346 | New variable-length integer encoding (#1016)                                  | ✅ **RS-1 + TS-1 DONE**                                                                        |
| 7348 | Encode Message Parameters as Type-Value pairs (#1462)                         | ✅ already draft-18-shaped — `model/common/pair.rs:65-91` delta-encodes Type; verify under C-1 |
| 7350 | Add GREASE for Setup Options, Properties, and error code registries (#1460)   | **RS-18**                                                                                      |
| 7353 | Add RENDEZVOUS_TIMEOUT parameter for SUBSCRIBE (#1447)                        | RS-9 (`0x04`, absent)                                                                          |
| 7355 | Add PUBLISH_BLOCKED message for SUBSCRIBE_NAMESPACE flow control (#1452)      | RS-16, RL-2                                                                                    |
| 7358 | Add Timeout field to GOAWAY message (#1497)                                   | RS-13 — **verified missing**, `goaway.rs:22-24`                                                |
| 7360 | Add GOING_AWAY to REQUEST_ERROR codes (#1434)                                 | RS-10                                                                                          |
| 7362 | Add EXCESSIVE_LOAD error code (#1479)                                         | RS-10                                                                                          |
| 7364 | Add NAMESPACE_TOO_LARGE error and stream reset for large namespaces (#1496)   | RS-10 (`0x31`), RS-12                                                                          |
| 7367 | Add TOO_FAR_BEHIND stream reset code (#1445)                                  | RS-10, RL-5                                                                                    |
| 7369 | Add REQUEST_UPDATE to list of REQUEST_ERROR causes (#1466)                    | RS-19                                                                                          |
| 7371 | Enforce REQUEST_OK/ERROR as first message on the response stream (#1499)      | RS-5                                                                                           |
| 7374 | Allow joining FETCH for PUBLISH and REQUEST_UPDATE with forward=1 (#1335)     | **RL-7**                                                                                       |
| 7377 | Allow DELIVERY_TIMEOUT value of 0 to mean no timeout (#1450)                  | RS-9                                                                                           |
| 7379 | Allow zero-element namespaces (#1472)                                         | RS-12, TS-12                                                                                   |
| 7381 | Clarify EXPIRES parameter update mechanism (#1448)                            | **RL-6**                                                                                       |
| 7383 | Remove TRACK_STATUS from REQUEST_UPDATE (#1436)                               | **RS-19**                                                                                      |
| 7385 | Define how to use auth token cache safely with multiple streams (#1430)       | **RS-20**                                                                                      |
| 7388 | Constrain encoding/parsing of track namespace and names (#1512)               | RS-12                                                                                          |
| 7397 | Reserve Property type ranges for application-specific use (#1473)             | RS-18 (`0x78-0x7F`, `0x3800-0x3FFF` per §15.8)                                                 |
| 7399 | Make EndGroup in Subscription Filters a delta (#1470)                         | RS-17                                                                                          |
| 7401 | Copy DELIVERY_TIMEOUT min requirement from parameter to property (#1427)      | RS-9                                                                                           |
| 7406 | Clarify prior Object semantics after End of Range indicators in FETCH (#1513) | **RL-7**                                                                                       |
| 7409 | Clarify datagram status and properties cases (#1444)                          | RS-17                                                                                          |
| 7411 | Clarify Stream Count includes empty subgroups (#1449)                         | **RL-7**                                                                                       |
| 7413 | Clarify language for malformed tracks in a subgroup with END_OF_GROUP (#1464) | RS-14                                                                                          |
| 7416 | Properties can appear in mutable list or inside Immutable Properties (#1442)  | RS-8                                                                                           |
| 7419 | Clarify immutable property preservation requirements (#1441)                  | RL-1                                                                                           |
| 7421 | Clarification for Track Alias uniqueness (#1418)                              | RL-2 (alias uniqueness per session)                                                            |
| 7425 | Rename Setup Parameters to Setup Options (#1461)                              | RS-3, TS-3                                                                                     |
| 7427 | Rename Extension Headers to Properties (#1504)                                | RS-8, TS-8                                                                                     |
| 7429 | Add security/privacy considerations for MOQT_IMPLEMENTATION (#1511)           | RS-3 (TS gap)                                                                                  |
| 7432 | Add editorial text on bandwidth probing techniques (#1477)                    | — editorial (relates to RS-17 padding)                                                         |
| 7434 | Explain idle connection handling (#1443)                                      | — editorial                                                                                    |
| 7436 | Fix "SUBSCRIBE_NAMESPACE with short prefixes" (#1502)                         | RS-11                                                                                          |
| 7438 | Add generative AI disclosure per IRTF guidelines                              | — no action                                                                                    |

---

## 10. Sequencing

All of this lands on `main`.

```
RS-2a 🔥 (ALPN moqt-18 — do first, restores clean failure)
C-1
  └─> RS-8 (rename — land early, conflicts with everything)
        └─> RS-9 ──> RS-18 ;  RS-14, RS-15
  └─> RS-2 ──> RS-3 ──> RS-4 ──> RS-5 ─┬─> RS-6
                                        ├─> RS-7 ──> RS-7b
                                        ├─> RS-10 (L: builds the reset path) ─┬─> RS-6b
                                        │                                     └─> RS-13
                                        ├─> RS-19 ──> RL-6
                                        └─> RS-20
                          RS-7 + RS-10 ──> RS-11 ─┬─> RS-12
                                                   ├─> RS-16 ──> RL-2
                                                   ├─> RS-17 (defer)
                                                   └─> RL-3
                                  RS-8 ──> RL-1 ;  RS-7 ──> RL-4
                        RS-10 + RS-13 ──> RL-5 ;  RS-7b + RS-15 ──> RL-7
                                  RS-3 ──> CL-1
  all RS + RL + CL-1 ──────────────────> CL-2  🚦 Rust interop gate
                              CL-2 ─────> TS-2 … TS-21
                              TS-* ─────> JS-1 ──> JS-2  🚦 JS interop gate — main interoperable again
                              TS-* ─────> MT-1 ──> MT-2

independent, land any time: TS-17, RS-21 (docs only)
```

**Critical path:** `RS-2a → C-1 → RS-2 → RS-3 → RS-4 → RS-5 → RS-11 → CL-2 → TS-2 → … → JS-2`.

RS-10 is the one that grew: it is now L (it builds the reset path from nothing) and it gates
RS-6b, so it is worth starting early in the RS-5 fan-out rather than last.

RS-1/TS-1 are off the critical path — already done.

## 11. Decisions taken

No open questions block the work. For the record:

- **Development on `main`**, no integration branch, no relay dual-stacking (§1). Deployment is
  out of scope for this plan.
- **`Switch = 0x22` stays**, documented as non-conformant, docs-only (RS-21). No `SWITCH_FROM`
  in this upgrade — it is not a draft-18 feature. Redesign is a separate effort after JS-2.
- **Property application-specific range is `0x78-0x7F` / `0x3800-0x3FFF`** per §15.8. §2.5's
  `0x38-0x3F` is stale text predating the #1016 varint change (RS-18).
- **RS-10 needs no spike** — there are no stream-reset call sites in either language; the task
  builds that path from nothing, which is why it is L and why it gates RS-6b.
- **`apps/moqtail-pub`** stays untracked and out of scope.
- **Paper experiments discarded** — no constraint on wire-format changes from that direction.

One item has external lead time and is worth starting early rather than at its slot:

- **Upstream spec bug** (RS-18) — §2.5 (line 1188) gives the 1-byte application-specific
  Property range as `0x38-0x3F`, the top 8 of the _old_ 6-bit varint space; §15.8 (line 6705)
  correctly gives `0x78-0x7F` for the new 7-bit 1-byte encoding. Worth filing against
  `moq-wg/moq-transport`. Blocks nothing here — the plan follows §15.8.
