# ABR Client — Design Spec

**Date:** 2026-03-23
**Status:** Approved
**Scope:** `apps/client-js`

---

## 1. Problem Statement

The client-js player currently switches quality tracks by tearing down and restarting the entire player session. This causes a visible video freeze and a full reconnect cycle (new WebTransport session, new subscriptions, new init segment). There is no mechanism for measuring network conditions or making automatic quality decisions.

This spec covers adding:
- **Seamless track switching** via the MoQ SWITCH message (no reconnect, no freeze)
- **Manual ABR mode**: user selects quality at any time via the dashboard
- **Auto ABR mode**: hybrid EMA bandwidth estimation + BOLA buffer-based scoring drives quality decisions autonomously, non-blocking and parallel to the video pipeline

---

## 2. Goals

1. Replace the full-restart track-change path with a seamless SWITCH-message path using `client.switch()`.
2. Add a `GoodputTracker` to measure per-object delivery bandwidth and maintain an EMA.
3. Add an `AbrController` that runs a non-blocking decision loop using hybrid EMA + BOLA logic.
4. Add an `AbrDashboard` UI component with mode toggle, live stats, configurable thresholds, and a quality history sparkline.
5. Support both manual and automatic ABR modes, switchable at runtime.

---

## 3. Non-Goals

- Audio track ABR (publisher does not send audio)
- Relay or publisher changes (both are already complete)
- Changes to `moqtail-ts` library (all required APIs already exist)
- Multi-variant simultaneous subscription (one video track at a time)
- Persistent threshold storage (thresholds reset on page reload)

---

## 4. Architecture

```
app.tsx
  ├── Player                          (extended)
  │     ├── GoodputTracker            (new — EMA inside Player per stream, internal only)
  │     ├── switchTrack(name)         (new method — SWITCH + init reinject)
  │     └── getMetrics()              (new method — bw + buffer + activeTrack)
  │
  ├── AbrController                   (new: src/lib/abr.ts)
  │     ├── start() / stop()
  │     ├── setMode('auto'|'manual')
  │     ├── setThresholds(config)
  │     ├── manualSwitch(trackName)
  │     └── [setInterval loop @ 250ms — BOLA score + EMA cap → player.switchTrack]
  │
  └── AbrDashboard                    (new component in app.tsx)
        ├── Mode toggle (Manual / Auto)
        ├── Live stats (EMA bandwidth, buffer seconds, BOLA score, active quality)
        ├── Configurable thresholds (sliders: V, gp, bufferMax, emaFactor, emaAlphaFast, emaAlphaSlow)
        └── Quality history sparkline (SVG step chart, last 60 events)
```

**No relay changes.** SWITCH handling and group-boundary alignment are fully implemented via `SwitchContext` in `switch_context.rs`.

**No publisher changes.** All 4 quality variants (360p/480p/720p/1080p) are published continuously.

**No moqtail-ts changes.** `MOQtailClient.switch(SwitchOptions)` already exists and returns `Promise<SubscribeError | { requestId: bigint; stream: ReadableStream<MoqtObject> }>`. The returned stream is the same object as the original subscription stream — objects from the new track flow through it after the relay's group boundary transition.

---

## 5. ABR Algorithm: Hybrid EMA + BOLA

### 5.1 DEWMA (Dual Exponential Weighted Moving Average — Goodput Estimation)

Two EMA trackers run simultaneously on each object delivery measurement, using the min-of-two principle for conservative, safe bandwidth estimation.

**Instantaneous goodput per object:**
```
instantaneous_bps = (bytes * 8 * 1000) / durationMs
```

**Fast EMA** (default `alphaFast = 0.5`): highly reactive, drops immediately on congestion:
```
ema_fast = alphaFast * instantaneous_bps + (1 - alphaFast) * ema_fast
```

**Slow EMA** (default `alphaSlow = 0.1`): stable historical baseline, ignores micro-stutters:
```
ema_slow = alphaSlow * instantaneous_bps + (1 - alphaSlow) * ema_slow
```

**Bandwidth estimate — minimum of both:**
```
BW_est = min(ema_fast, ema_slow)
```

Taking the minimum means the estimate reacts quickly to real degradation (fast EMA drops) while not overreacting to brief spikes (slow EMA stays high). The estimate only rises when *both* EMAs agree that bandwidth has genuinely recovered.

DEWMA is used as a **cap**: no quality tier is selected whose bitrate exceeds `BW_est * emaFactor`.

**DEWMA cold start behaviour:** After a track switch both EMAs are reset to 0. While `BW_est === 0`, the DEWMA cap blocks all quality tiers. During this warm-up phase, the controller stays on the current track — BOLA scores are computed but no switch is triggered until at least one sample has been recorded. This is the safe default: avoid switching blind.

### 5.2 BOLA (Buffer-Based Quality Selection)

BOLA (Buffer Occupancy based Lyapunov Algorithm, Spiteri et al.) selects quality based on current buffer level, not bandwidth prediction. It is the algorithm deployed in dash.js and used in production at Akamai, BBC, and CBS.

**Utility function** (log-based, min quality has utility 1):
```
utility_m = log(bitrate_m / bitrate_min) + 1
```

**Control parameter V** (derived from `bufferMax`, `segmentDurationS`, and utility range):
```
V = (bufferMax - segmentDurationS) / (utility_max + bolaGp)
```
where `gp` (gamma × p) is the rebuffering penalty parameter and `segmentDurationS` is the GOP/segment duration in seconds (hardcoded to `1.0` to match the publisher's 1-second GOPs; exposed as `segmentDurationS` in `AbrThresholds` so it can be updated if the publisher changes).

**BOLA score per quality tier m:**
```
score_m = (V * (utility_m + bolaGp) - bufferLevel) / bitrate_m
```

**Selection rule:** choose the highest quality tier `m` where:
- `score_m > 0` (BOLA says the buffer level supports this quality)
- `bitrate_m ≤ BW_est * emaFactor` (DEWMA cap — skip this check when `BW_est === 0`, stay on current tier instead)

If no tier satisfies both conditions, stay on the current tier (no switch).

### 5.3 Decision Loop

Runs every `pollIntervalMs` (default 250ms). `#switching` is set to `true` **synchronously** before any `await` inside `switchTrack()`, so two rapid loop ticks cannot both pass the guard:

```
if #switching: return   ← guard check (synchronous)

metrics = player.getMetrics()   // bandwidthBps, fastEmaBps, slowEmaBps, bufferSeconds, activeTrack

// Emergency: buffer critically low — drop to lowest tier immediately
if metrics.bufferSeconds < emergencyBufferFloor:
  #switching = true                              ← set synchronously
  player.switchTrack(lowestTier)                 ← fire-and-forget (no await)
  return

// DEWMA cold start: skip quality selection until first sample arrives
if metrics.bandwidthBps === 0: return

// BOLA score all tiers, apply DEWMA cap (BW_est = min(fast, slow))
best = highest tier where score_m > 0 AND bitrate_m ≤ metrics.bandwidthBps * emaFactor

if best !== currentTier:
  #switching = true                              ← set synchronously
  player.switchTrack(best)                       ← fire-and-forget (no await)
```

`#switching` is cleared by the `onTrackSwitched` callback, which `Player.switchTrack()` calls on completion (success or failure).

---

## 6. New Files

### `src/lib/goodput.ts` — `GoodputTracker` (internal to Player)

`GoodputTracker` is an internal implementation detail of `Player`. It is **not** re-exported from `src/lib/index.ts`.

Implements DEWMA: two simultaneous EMA trackers, returning `min(ema_fast, ema_slow)` as the bandwidth estimate.

```ts
export class GoodputTracker {
  constructor(alphaFast: number = 0.5, alphaSlow: number = 0.1)

  // Called inside WritableStream.write with bytes received and ms elapsed
  // Updates both fast and slow EMAs
  recordObject(bytes: number, durationMs: number): void

  // Returns min(ema_fast, ema_slow) in bps; 0 until first sample recorded
  getBandwidthBps(): number

  // Exposed for dashboard display
  getFastEmaBps(): number
  getSlowEmaBps(): number

  // Resets both EMAs to 0 (called on track switch)
  reset(): void

  // Update alpha values without recreating the tracker
  setAlphas(alphaFast: number, alphaSlow: number): void
}
```

---

### `src/lib/abr.ts` — `AbrController`

#### Threshold configuration

```ts
export interface AbrThresholds {
  // BOLA V parameter (buffer target sensitivity)
  // Higher = more aggressive quality seeking
  // Recomputed automatically when bufferMax, bolaGp, or segmentDurationS changes
  bolaV: number                  // default: auto-computed

  // BOLA gamma*p rebuffering penalty
  // Higher = more conservative, favours lower quality to protect buffer
  bolaGp: number                 // default: 1.0

  // Maximum buffer target in seconds (BOLA normalisation)
  bufferMax: number              // default: 5.0

  // GOP / segment duration in seconds — must match publisher's GOP (currently 1s)
  // Used in bolaV computation: V = (bufferMax - segmentDurationS) / (utility_max + bolaGp)
  segmentDurationS: number       // default: 1.0

  // Emergency floor: drop to lowest tier immediately if buffer drops below this
  emergencyBufferFloor: number   // default: 0.5

  // EMA safety factor: cap selected quality at emaBandwidth * emaFactor
  // Prevents selecting quality that would immediately drain the buffer
  emaFactor: number              // default: 0.85

  // DEWMA fast EMA alpha (forwarded to GoodputTracker.setAlphas())
  // Higher = more reactive to sudden drops (default 0.5)
  emaAlphaFast: number           // default: 0.5

  // DEWMA slow EMA alpha (forwarded to GoodputTracker.setAlphas())
  // Lower = more stable historical baseline (default 0.1)
  emaAlphaSlow: number           // default: 0.1

  // Decision loop interval in ms
  pollIntervalMs: number         // default: 250
}
```

`setThresholds(partial)` merges into the current config. If `bufferMax`, `bolaGp`, or `segmentDurationS` changes, `bolaV` is recomputed from the formula. If `emaAlphaFast` or `emaAlphaSlow` changes, `player`'s internal `GoodputTracker.setAlphas()` is called via a new `Player.setEmaAlphas(fast, slow)` method (see Section 7).

#### Switch history

```ts
export type SwitchReason = 'auto-upgrade' | 'auto-downgrade' | 'auto-emergency' | 'manual'

export interface SwitchEvent {
  ts: number               // performance.now() timestamp
  fromTrack: string
  toTrack: string
  reason: SwitchReason
  bufferAtSwitch: number   // buffer seconds at time of decision
  emaBwAtSwitch: number    // EMA bandwidth bps at time of decision
}
```

Ring buffer of last 60 events.

#### Metrics snapshot

```ts
export interface AbrMetrics {
  bandwidthBps: number         // min(ema_fast, ema_slow) — used for DEWMA cap
  fastEmaBps: number           // fast EMA value (for dashboard display)
  slowEmaBps: number           // slow EMA value (for dashboard display)
  bufferSeconds: number
  activeTrack: string | null
  mode: 'auto' | 'manual'
  bolaScores: Record<string, number>   // per-track BOLA score for dashboard display
  history: SwitchEvent[]
}
```

#### Public API

```ts
export class AbrController {
  constructor(
    player: Player,
    tracks: Track[],                              // from catalog, sorted by bitrate asc
    onMetricsUpdate: (m: AbrMetrics) => void,
  )

  start(): void
  stop(): void
  setMode(mode: 'auto' | 'manual'): void
  setThresholds(t: Partial<AbrThresholds>): void
  manualSwitch(trackName: string): void
  getHistory(): SwitchEvent[]
}
```

---

## 7. Modified Files

### `src/lib/player.ts`

#### `MOQStreamStruct` additions

Two new fields on `MOQStreamStruct`:

```ts
tracker: GoodputTracker           // EMA measurement for this stream
lastGroupId: bigint               // last seen object.location.group, initialised to -1n
pendingSwitch: {
  trackName: string
  initData: ArrayBuffer
  mimeType: string
} | null
```

#### GoodputTracker integration

One `GoodputTracker` per `MOQStreamStruct`. Inside the existing `WritableStream.write` handler, wrap the existing append sequence:

```ts
const t0 = performance.now()
// ... existing appendBuffer + waitForBufferUpdate retry loop ...
struct.tracker.recordObject(object.payload.byteLength, performance.now() - t0)
struct.lastGroupId = object.location.group   // update last seen group
```

Purely additive — no change to existing append logic.

#### `switchTrack(trackName: string): Promise<void>`

New public method. `#switching` is set at the **call site** (in `AbrController`) synchronously before firing this method. The method itself must also do the following atomically:

1. Find the current video stream struct in `#streams` (by `catalog.getRole(s.trackName) === 'video'`)
2. Build `FullTrackName` for `trackName`
3. Fetch init data: `this.catalog.getInitData(trackName)`
4. Fetch MIME string: `const role = this.catalog.getRole(trackName); const codec = this.catalog.getCodecString(trackName); const mimeType = \`${role}/mp4; codecs="${codec}"\``
5. Call `const result = await this.client.switch({ fullTrackName, subscriptionRequestId: struct.requestId })`
6. **If result is `SubscribeError`**: log the error, call `this.#options.onTrackSwitched?.(struct.trackName)` with the *current* track name (signals failure, releases `#switching`), return early
7. **On success**: update `struct.requestId = result.requestId` (the new request ID returned by `client.switch()`), set `struct.pendingSwitch = { trackName, initData, mimeType }`, reset `struct.tracker`
8. Call `this.#options.onTrackSwitched?.(trackName)`

#### Init segment re-injection (inside `WritableStream.write`)

When `struct.pendingSwitch` is set and the incoming object's group ID has advanced (`object.location.group > struct.lastGroupId`), this signals the relay has completed the group-boundary transition. Execute before appending the object payload:

```ts
// Guard: must not call changeType while SourceBuffer is updating
if (sourceBuffer.updating) await waitForBufferUpdate(sourceBuffer)

const { initData, mimeType } = struct.pendingSwitch
sourceBuffer.changeType(mimeType)           // reconfigure for new resolution/codec
sourceBuffer.appendBuffer(initData)
await waitForBufferUpdate(sourceBuffer)
struct.trackName = struct.pendingSwitch.trackName   // read before nulling
struct.pendingSwitch = null
```

Then fall through to the existing payload append. `SourceBuffer.changeType()` is the MSE API for reconfiguring a source buffer for a new codec/resolution without recreating it, and is supported in all modern browsers.

#### `getMetrics()`

```ts
getMetrics(): { bandwidthBps: number; fastEmaBps: number; slowEmaBps: number; bufferSeconds: number; activeTrack: string | null } {
  const videoStruct = this.#streams.find(s => this.catalog?.getRole(s.trackName) === 'video')
  const buffered = this.#element?.buffered
  const bufferSeconds =
    buffered && buffered.length > 0 && this.#element
      ? Math.max(0, buffered.end(buffered.length - 1) - this.#element.currentTime)
      : 0
  return {
    bandwidthBps: videoStruct?.tracker.getBandwidthBps() ?? 0,
    fastEmaBps: videoStruct?.tracker.getFastEmaBps() ?? 0,
    slowEmaBps: videoStruct?.tracker.getSlowEmaBps() ?? 0,
    bufferSeconds,
    activeTrack: videoStruct?.trackName ?? null,
  }
}
```

`catalog` may be null between `dispose()` and the next `initialize()`. The optional-chain `this.catalog?.getRole(...)` handles this safely — `videoStruct` will be undefined and all metrics return their zero defaults.

#### `setEmaAlphas(alphaFast: number, alphaSlow: number): void`

New public method. Called by `AbrController.setThresholds()` when `emaAlphaFast` or `emaAlphaSlow` changes:

```ts
setEmaAlphas(alphaFast: number, alphaSlow: number): void {
  const videoStruct = this.#streams.find(s => this.catalog?.getRole(s.trackName) === 'video')
  videoStruct?.tracker.setAlphas(alphaFast, alphaSlow)
}
```

#### `PlayerOptions` addition

```ts
onTrackSwitched?: (trackName: string) => void
```

---

### `src/app.tsx`

#### State additions

```ts
const [abrMode, setAbrMode] = useState<'auto' | 'manual'>('manual')
const [abrMetrics, setAbrMetrics] = useState<AbrMetrics | null>(null)
const abrRef = useRef<AbrController | null>(null)
```

#### Lifecycle

- `AbrController` created after `player.startMedia()` succeeds, with `setAbrMetrics` as `onMetricsUpdate`
- `abrRef.current.start()` called immediately after creation
- `abrRef.current.stop()` called in `disposePlayer` before player dispose

#### `handleTrackChange` replacement

In **manual mode**: track row clicks call `abrRef.current?.manualSwitch(trackName)` — no full restart.

In **auto mode**: track rows are read-only (disabled checkboxes). Mode toggle switches between the two at runtime.

The existing full-restart `startPlayback` path is retained only for initial connection and explicit "Connect" button clicks.

---

## 8. `AbrDashboard` Component

Rendered below the track list in the sidebar when `hasTracks && abrMetrics !== null`:

```
┌─────────────────────────────────┐
│ ABR  [Manual ●────────── Auto]  │  ← segmented toggle
├─────────────────────────────────┤
│ BW (est)    1.8 Mbps  ████░░░   │  ← DEWMA min(fast,slow) vs current tier bitrate
│  fast EMA   2.1 Mbps            │  ← fast EMA raw value
│  slow EMA   1.6 Mbps            │  ← slow EMA raw value
│ Buffer      2.4 s     ████████  │  ← buffer seconds + fill bar (0–bufferMax)
│ Quality     720p                │  ← active track label
├─────────────────────────────────┤
│ Thresholds  ▾ (collapsible)     │
│  BOLA V     2.1  [──●────────]  │  ← range 0.5–5.0
│  BOLA gp    1.0  [●──────────]  │  ← range 0.5–5.0
│  Buf Max    5.0s [──────────●]  │  ← range 2.0–10.0
│  BW Cap     0.85 [────────●──]  │  ← emaFactor range 0.5–1.0
│  α fast     0.5  [────●──────]  │  ← range 0.1–0.9
│  α slow     0.1  [●──────────]  │  ← range 0.01–0.3
├─────────────────────────────────┤
│ BOLA Scores                     │
│  360p  +2.1  480p  +0.8         │
│  720p  -0.3  1080p -1.4         │  ← live per-tier scores; positive = buffer supports tier
├─────────────────────────────────┤
│ History                         │
│  ▁▁▂▃▃▂▁▁▂▃▄▄▃▃▄▄▃▂▁▁▂▃▄      │  ← SVG step chart
└─────────────────────────────────┘
```

**Sparkline:** inline SVG, no external library. Y = quality tier index (0–3), X = last 60 switch events newest-right. Rendered as a `<polyline>` step path.

**BOLA scores panel:** shows live `score_m` for each tier. Positive = buffer level supports this quality; negative = buffer too low. Gives the user direct visibility into why BOLA is or is not selecting each tier.

**Threshold sliders:** `<input type="range">`. `onChange` calls `abrRef.current.setThresholds(...)`. Changes to `bolaGp`, `bufferMax`, or `segmentDurationS` trigger automatic recomputation of `bolaV` inside `AbrController`. Changes to `emaAlphaFast` or `emaAlphaSlow` call `player.setEmaAlphas(fast, slow)`.

---

## 9. Data Flow

```
[relay] ──objects──► Player.WritableStream.write
                          │
                          ├── struct.lastGroupId = object.location.group
                          ├── GoodputTracker.recordObject(bytes, ms)   ← updates fast+slow EMA, ~1µs
                          ├── if pendingSwitch && object.location.group > lastGroupId:
                          │       await waitForBufferUpdate if updating
                          │       sourceBuffer.changeType(newMimeType)
                          │       sourceBuffer.appendBuffer(initData)
                          │       pendingSwitch = null
                          └── sourceBuffer.appendBuffer(payload)

[AbrController setInterval @ 250ms]   ← entirely outside media pipeline
  if #switching: skip
  player.getMetrics() → { bandwidthBps, fastEmaBps, slowEmaBps, bufferSeconds, activeTrack }  // BW_est = min(fast,slow)
    → compute BOLA scores for all tiers
    → apply DEWMA cap: BW_est = min(fast, slow); skip if BW_est === 0
    → if best tier ≠ current tier:
        #switching = true   ← synchronous
        player.switchTrack(bestTier)   ← fire-and-forget
             │
             ├── client.switch({ fullTrackName, subscriptionRequestId })
             │        └── [relay] group boundary → new track objects begin flowing
             ├── on SubscribeError: log + call onTrackSwitched(currentTrack) → #switching = false
             └── on success: update struct.requestId, set pendingSwitch, reset tracker (both EMAs)
                      └── onTrackSwitched(newTrackName) → #switching = false
```

---

## 10. File Change Summary

| File | Change |
|---|---|
| `apps/client-js/src/lib/goodput.ts` | New — `GoodputTracker` (EMA); internal only, not re-exported from index |
| `apps/client-js/src/lib/abr.ts` | New — `AbrController` (BOLA + EMA cap, decision loop, history) |
| `apps/client-js/src/lib/player.ts` | Add `GoodputTracker`, `MOQStreamStruct` fields, `switchTrack()`, `getMetrics()`, `setEmaAlphas()`, init reinject |
| `apps/client-js/src/lib/index.ts` | Add re-export for `AbrController` only (`GoodputTracker` stays internal) |
| `apps/client-js/src/app.tsx` | Add `AbrController` wiring, `AbrDashboard` component, replace track-change handler |

---

## 11. Open Questions / Future Work

- **Switch cooldown**: a time-based cooldown (no upgrade within Xs of a downgrade) can be added as an additional threshold to prevent thrashing on marginal bandwidth.
- **BOLA-FINITE vs BOLA-BASIC**: this spec implements BOLA-BASIC. BOLA-FINITE (handles finite-length videos) is not needed for live streaming.
- **Audio ABR**: not in scope. When audio is added, the same `switchTrack` + `AbrController` pattern extends naturally.
- **EMA persistence across switches**: currently reset on every switch. Could be preserved with a decay factor — the benefit is shorter warm-up after a switch; the risk is carrying stale data from the old track's bandwidth conditions.
- **Pre-buffering next tier**: subscribing to the target tier briefly before switching (to pre-warm the buffer) is a possible future optimisation. Currently only one video track is active at a time.
