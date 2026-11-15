# ABR controller

`abr()` is the player's default ABR plugin. It picks the video rendition for live streams. Every
tick (500 ms by default) a small neural network chooses one of four actions: **Down**, **Stay**,
**Up** or **Probe**. Hand-written rules decide which of those actions are allowed on that tick.

```ts
import { MOQtailPlayer, abr, liveBuffer } from 'moqtail/player'

new MOQtailPlayer({ plugins: [liveBuffer(), abr({ onDecision: (d) => console.log(d.action, d.observation) })] })
```

`abr({ tickMs, model, onDecision })`: `tickMs` is the decision interval, which the bundled model
was trained at 500 ms. `model` is a `ModelBlob` that replaces the bundled one. `onDecision` gets
every `Decision`, with the observation, the features and the action.

The player attaches the controller only when it has something to decide: a live stream whose video
source doesn't switch by itself (an `ssts()` source does) and has no rendition picked manually. It
also does nothing while the viewer has paused, or when fewer than two renditions are selected or a
bitrate is unknown.

## A tick

```text
data events, player state, sink buffer
        │
   Signals (signals.ts)         windowed stats: throughput, arrival rate, stalls, groups
        │
   Observation ──► features()   13 inputs, all ratios or clipped durations (features.ts)
        │                │
 legalActions()       Policy    int4 MLP, logits over the actions (policy.ts, model.ts)
        └──────┬─────────┘
          sample a legal action
               │
   switch() / Prober.start()
```

1. **Observe.** `Signals` follows the player's data events and keeps the last few seconds of
   arrivals: live video bytes, objects against the frame rate (`arrivalRate`, about 1 while the
   link keeps up), stalls, group starts and the last complete groups. The controller adds the
   forward buffer, the playback rate, the ladder (the selected renditions by bitrate) and what the
   prober knows.
2. **Featurize.** `features()` turns the observation into ratios that don't depend on the ladder's
   bitrates or the link's absolute speed, so one model works for any ladder.
3. **Restrict.** `legalActions()` gates the model, because every switch costs the viewer:
   - only **Stay** while a switch settles (the old track's streams haven't ended, or the new track
     hasn't arrived) and for 1 s after;
   - **Down** only on trouble: a stall, frames arriving under 0.8 of their rate, or under 0.5 s of
     buffer. It goes straight to the highest rendition within 0.8 of measured throughput;
   - **Up** only when the median of the last 3 probe estimates, all still fresh, has room for the
     next rendition with a 0.9 margin. It goes straight to the highest rendition that fits;
   - **Probe** only below the top, with no probe in flight, while not starved, once the backoff has
     passed (5, 10 then 15 s after probes that found no room, reset after 30 calm seconds).
4. **Decide.** `Policy.choose()` samples a legal action by the model's odds, rather than always
   taking the likeliest one: the model learned some choices, such as an occasional probe on a calm
   link, only as odds. For the first 2 s of live video the controller stays put unless playback
   stalls, because startup reads as congestion.

## Probing

Live delivery is paced by the publisher, so it never shows capacity above the current bitrate.
To measure headroom, a probe FETCHes objects of a recently received group again, at priority 200
(below live media), so the relay sends them only when the live stream leaves the link idle. The
probe asks for about one second of the bitrate gap to the next rendition. Its estimate counts every
byte that arrived from the request until its last object, live bytes included. The probe ends when
all of it has arrived, when it falls too far behind the pace that would show room, or after 2 s.

Estimates age only while the controller is free to act, and stay evidence for 10 s. A stall or
starvation while the estimate is above the playing bitrate contradicts it, and the estimates are
dropped.

## The model

`policy.ts` decodes a few hundred bytes of base64: a header that carries `FEATURES_VERSION` and a
hash of the feature names, then int4 weights with f16 scales per layer, optionally deflate-raw
compressed. A model trained on another feature list is rejected at load. If the model doesn't
load, ABR stays idle. Changing `features()` or the feature list means training a new model and
bumping `FEATURES_VERSION`. `model.ts` is generated.
