---
'client-js': patch
---

The CMAF muxer releases each sample once its segment is out. mp4box kept every encoded frame twice for the life of the publisher, so a long-running publisher ran out of memory.
