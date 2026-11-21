---
'moqtail': patch
---

Setting `config.plugins = []` on `<moqtail-video>` installs no plugins. It installed `liveBuffer()` and `abr()`, which are now installed only when `config.plugins` is unset.
