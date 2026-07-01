---
'moqtail': patch
---

Fix `Header.deserialize` rejecting subgroup headers with default publisher priority. The dispatch previously used an explicit switch covering only `0x10–0x15` and `0x18–0x1D`, missing the `0x30–0x35` and `0x38–0x3D` ranges (DEFAULT_PRIORITY bit set). Replaced with bitmask detection matching the Rust validator.
