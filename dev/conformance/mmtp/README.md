# MMTP receiver vectors

`receiver-v1.json` is the shared contract for the MMTP receiver path. Every
implementation of that path loads this file; none generates it.

Each case lists the payloads of one MoQ track whose catalog `packaging` is
`mmtp`, one MMTP packet per object, fed in order to a fresh receiver. `mfus`
lists every MFU the receiver must emit, in order (`data` is the reassembled MFU
payload); `rejected` counts the objects it must drop.

## Provenance

| Case                             | Source                                                                                                                                                                                                                                                                                                                                                                      |
| -------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `ffmpeg-a747b97f-partial-run`    | Real producer bytes: FFmpeg `a747b97f` `moq_mmt` muxer, `testsrc2`, captured 2026-08-19. First, middle and last fragment (`fragment_counter` 37, 18, 0) of a 38-fragment run, so no MFU completes.                                                                                                                                                                          |
| `two-byte-source-fec-payload-id` | Captured packet carrying a 2-byte Source FEC Payload ID where ISO/IEC 23008-1 C.5.2 specifies 4.                                                                                                                                                                                                                                                                            |
| every other case                 | Hand-encoded from ISO/IEC 23008-1 9.2: 12-byte MMTP header, MPU payload header, and a 14-byte timed MFU data-unit header. Fragmented runs use consecutive `packet_sequence_number`s with a counting-down `fragment_counter`. `complete-mfu-source-fec-payload-id` sets FEC_type=1 and carries a 4-byte Source FEC Payload ID after `length`, which the receiver must strip. |

The hand-encoded fragmented runs carry the data-unit header on the first
fragment (FI=1) only, as the FFmpeg producer does: its captured middle fragment
is exactly 14 bytes shorter than its first. These vectors therefore pin that
producer convention; they do not independently establish it against the
ISO/IEC 23008-1 MPU-payload syntax, because the producer, libmmt and these
vectors all share it.

The negative cases each break one rule: a header cut short, an empty object,
the wrong Source FEC Payload ID width, a complete MFU (FI=0) with a non-zero
`fragment_counter`, a `payload_length` running past the packet, and a second
first fragment (FI=1) for a run that is already open.
