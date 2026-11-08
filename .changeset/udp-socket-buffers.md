---
'relay': minor
---

ask for 8 MiB UDP socket buffers by default (`--socket-buffer-kib`, 0 keeps the kernel default), so a fan-out burst no longer overflows the socket
