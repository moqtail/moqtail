---
'relay': patch
---

Accept connections whose TLS handshake goes through a HelloRetryRequest, such as picoquic clients offering only an X25519MLKEM768 key share. The relay now reads the negotiated ALPN after the handshake completes instead of closing the connection with no protocol chosen yet.
