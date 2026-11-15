---
'client-js': patch
'moqtail': patch
---

Implement the ABR controller: `abr()`, which `<moqtail-video>` installs by default, is now `MlAbrController` (replacing `DefaultAbrController`), a small neural network that switches live renditions and probes for room with FETCHes below live priority. Live switching sets now start at their middle bitrate. Also improve stuck playback scenarios by nudging the playback so that it lands inside a buffered region.
