---
'relay': patch
---

Accept X25519MLKEM768 key shares. The relay's TLS now runs on aws-lc-rs, so clients that offer only the post-quantum hybrid group, such as picoquic by default, complete the handshake instead of receiving a HelloRetryRequest.
