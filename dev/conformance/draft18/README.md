# Conformance fixtures

Language-agnostic JSON fixtures for the MOQT draft this directory is named after. Both
`libs/moqtail-rs` and `libs/moqtail-ts` load these files and assert their constants against
them.

**These fixtures are normative for both stacks.** A codepoint is changed here first, and never
in one language only. Every value is transcribed from the draft text in `docs/`; each file
names the table it comes from and each entry carries its own spec reference, so a fixture can
be diffed against the draft without reading any code.

## Files

| File                       | Source                                                              |
| -------------------------- | ------------------------------------------------------------------- |
| `varint.json`              | §1.4.1 Table 1 and Table 2, plus non-minimal and boundary cases     |
| `message_types.json`       | §10 Table 5 — control message types and the Stream column           |
| `parameter_types.json`     | §15.4 Table 10 (Setup Options), §15.7 Table 13 (Message Parameters) |
| `property_types.json`      | §15.8 Table 14, Table 15 (provisional), and the range policy        |
| `stream_reset_codes.json`  | §3.3.3, §15.10.4 Table 20                                           |
| `request_error_codes.json` | §15.10.2 Table 18                                                   |
| `termination_codes.json`   | §15.10.1 Table 17                                                   |

## Conventions

Codepoints are hex **strings** (`"0x50"`), so a fixture diffs against the draft's own tables
without mental base conversion.

Varint values are decimal **strings**. The largest of them, `18446744073709551615` (2^64-1),
cannot survive `JSON.parse` as a number — it silently rounds. Byte sequences are lowercase hex
strings with no `0x` prefix.

`"reserved": true` marks a codepoint the draft reserves. Parsers must reject it rather than
map it to a name.

## Sections

`entries` is the draft's table. A stack must define exactly these codepoints, with these
values, and must reject the ones marked `reserved`.

`not_in_draft` names codepoints a stack still defines that the draft does not. They are what
makes the check work in both directions: without them a suite could only prove that every
entry in the table exists in the code, never that the code has stopped inventing codepoints of
its own. Each carries a `pending` marker naming the issue that removes it; once that lands,
parsers must reject the value.

`local_extensions` is deliberate, permanent non-conformance — codepoints this project uses
that the draft does not define and that are not going away. They are never asserted against
the draft, and unlike `not_in_draft` they carry no `pending` marker, because nothing is
pending: the divergence is the decision.

## Pending markers

An entry a stack does not implement yet carries a marker naming the issue that will land it:

```json
{
  "name": "EXAMPLE_MESSAGE",
  "value": "0x50",
  "pending": { "rs": 1234, "ts": 5678 }
}
```

That reads: _this entry is normative, but moqtail-rs will not conform until issue 1234, and
moqtail-ts not until issue 5678._ A stack named in `pending` is exempt from the assertion; a
stack not named must match exactly or its suite fails. An entry with no `pending` key binds
both stacks.

Markers are self-cleaning. The tests also assert that a pending entry is still
non-conformant, so when the work lands the suite fails with _"marked pending but now conforms
— delete the marker"_. A marker cannot quietly become a permanent exemption, and what remains
marked is an accurate account of what is left to do.

## Adding or changing a codepoint

1. Change it here, citing the draft.
2. Run both suites. Whichever stack drifted now fails.
3. Fix that stack — or add a `pending` marker naming the issue that will.
