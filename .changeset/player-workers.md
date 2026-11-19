---
'moqtail': minor
---

The player runs `webcodecs` decoding and `playout` canvas rendering in a worker of its own where the browser allows it, linking them inside the worker so decoded frames stay off the main thread. `preferWorker: false` keeps them on the main thread. `serveWorker` from `moqtail/player/worker`, with the `worker` player option, runs your own elements' engines in that worker: an element's `threads` probe gives it `context.worker`. `ElementContext.fail` reports an element's fatal error from outside `receive()`.
