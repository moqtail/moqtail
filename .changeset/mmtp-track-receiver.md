---
'moqtail': minor
---

Add `moqtail/mmtp`, an MMTP receiver for tracks whose CMSF catalog packaging is `mmtp` (one MMTP packet per MoQ object). `MmtpTrackReceiver` reassembles MFUs clock-free from a subscription's object stream, delegating packet parsing and reassembly to the optional peer dependency `@blockcast/mmt-container`. The catalog now accepts `packaging: 'mmtp'` (with a required `codec` and positive integer `timescale`) and exposes `CMSFCatalog.isMMTP()`.
