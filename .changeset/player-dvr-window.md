---
'moqtail': patch
'client-js': patch
---

Make `<moqtail-video>` follow the catalog's `accessibleWindowTime` (MSF 5.2.46) on live streams. With a window, its `currentTime`, `seekable`, `buffered` and `liveEdgeStart` count from the window's start, so seekbars that start at 0, such as Video.js's, cover only the accessible range, and `targetLiveWindow` is `Infinity`. Without a window there is no DVR: `seekable` is empty, `targetLiveWindow` is `0`, and setting `currentTime` is ignored. The MSE sink bounds the live seekable range by the window even without a media timeline. client-js hides the Video.js time slider while nothing is seekable, leaving a spacer in its place as the live skin does, and its skin styles move to `videojs.css`.
