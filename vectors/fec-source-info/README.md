# FEC Source Info parity vectors

Cross-implementation fixtures for the MoQT object extension header that carries
the FEC source payload ID (BLO-40702, draft-ramadan-moq-fec).

MoQT's key-value parity rule makes an **even** extension type varint-valued and
an **odd** type length-prefixed-byte-valued. FEC Source Info is byte-valued, so
its type must be odd. It is **`0x11`**. The previous value `0x10` is even and is
not accepted anywhere — there is no compatibility shim.

## Value layout

```
SS_ID (32 bits, big endian)
```

The value is **exactly 4 bytes**; any other length is an error. SS_ID is the
source symbol's sequence number -- the same value and the same definition the
Source FEC Payload ID trailer carries on the MMTP path, so one identifier covers
both paths.

The field is fixed width on purpose, for two reasons. MoQT permits non-minimal
varint encodings, so a varint here would vary the extension's own length without
varying its meaning. And the varint encoding is a property of the session, which
MoQT redefined at draft-17 (#1016), so a varint would decode differently on two
drafts.

Format version 1 carried `SBN(32) + ESI(32) + optional ObjectLength(32)` at 8 or
12 bytes. Both widths are gone and both must now be **refused**: SBN/ESI
contradicted the draft's own block model (Z = 1, so SBN is always 0, and blocks
are delimited only by repair objects), and the object length travelled inside
the object it described, so it was absent in the recovery case it existed for.
8 and 12 are in `invalidValueLengths` for exactly that reason.

`cases.json` carries the **value** bytes only, not the `type || length || value`
framing. That framing is the session's business, and pinning it here would
re-introduce exactly the draft-coupling this change removes. Each implementation
composes the value through its own key-value encoder.

## Consumers

| Implementation                                 | Test                                                               |
| ---------------------------------------------- | ------------------------------------------------------------------ |
| libmmt TypeScript (`@blockcast/mmt-container`) | `packages/container/src/__tests__/fec-source-info-vectors.test.ts` |
| libmmt Rust (`mmt-core`)                       | `mmt-core/tests/fec_source_info_vectors.rs`                        |
| moqtail-ts, libmoq, upstream `js/net`          | interop harness, BLO-40677                                         |

## Authoring rule

These bytes are **hand-authored from the specification**. Never regenerate them
by serializing through an implementation under test — that proves
self-consistency, not wire compatibility (see the repo `CLAUDE.md`,
"Conformance Fixtures Are The Contract"). `byte-boundary`, `two-byte-boundary`
and `high-bit-set` exist to fail an implementation that kept a varint, or that
sign-extends the top bit.
