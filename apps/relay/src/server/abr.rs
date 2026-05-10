use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Instant;
use moqtail::model::data::full_track_name::FullTrackName;
use tracing::{info, warn};

use crate::server::client::MOQTClient;
use crate::server::track_manager::TrackManager;

// ─────────────────────────────────────────────────────────────────────────────
// Configuration
// ─────────────────────────────────────────────────────────────────────────────

/// Hard latency budget. A group born at T must finish arriving by T + budget.
const LATENCY_BUDGET_MS: f64 = 3000.0;

/// Tick period for the ABR loop.
const TICK_MS: u64 = 100;

/// Observation window for the app-limited signal.  Sized to one GOP so that
/// transient bursts don't fool us into thinking we're saturating the link.
const WINDOW_DURATION_MS: u64 = 500;
const WINDOW_TICKS: usize = (WINDOW_DURATION_MS / TICK_MS) as usize; // 5

/// After any tier change, hold for one full window before reconsidering.
/// Prevents oscillation and gives the link time to react to the change.
const COOLDOWN_TICKS: u64 = WINDOW_TICKS as u64;

/// Initial blindfold period.  Lets the transport finish slow start and
/// establish a min_rtt baseline before we start interpreting signals.
const WARMUP_TICKS: u64 = 30; // 3 s

/// Congestion thresholds applied to non-app-limited samples.
/// If, during the window, any non-app-limited sample shows queueing delay
/// above this, we treat the window as evidence of real congestion.
const CONGESTION_QUEUEING_DELAY_MS: f64 = 50.0;

/// If sustained loss rate exceeds this, downshift regardless of the
/// app-limited window analysis.
const CONGESTION_LOSS_RATE: f64 = 0.02;

/// Emergency: backlog this high AND climbing → slam to lowest tier.
const EMERGENCY_BACKLOG_LEVEL: u64 = 6;

// ─────────────────────────────────────────────────────────────────────────────
// Per-tick window sample
// ─────────────────────────────────────────────────────────────────────────────

/// One tick's worth of observations.  Stored in a ring buffer covering the
/// last WINDOW_TICKS ticks.  We keep the transport state alongside the
/// app_limited flag so we can analyze "what was the network doing during
/// the moments the link was actually being stressed?"
#[derive(Clone, Copy, Debug)]
struct Sample {
    app_limited: bool,
    queueing_delay_ms: f64,
}

// ─────────────────────────────────────────────────────────────────────────────
// Persistent history (lives across ticks)
// ─────────────────────────────────────────────────────────────────────────────

struct AbrHistory {
    /// Ring buffer of the last WINDOW_TICKS samples.
    window: VecDeque<Sample>,

    /// Loss-rate EWMA, computed from diffed PathStats counters.
    loss_ewma: f64,
    last_lost_packets: u64,
    last_sent_packets: u64,

    /// Ticks since the last tier change (or warmup exit).  Enforces
    /// COOLDOWN_TICKS between decisions.
    ticks_since_change: u64,

    /// True for the first WARMUP_TICKS ticks of the controller's life.
    in_warmup: bool,
    warmup_ticks_elapsed: u64,
}

impl AbrHistory {
    fn new() -> Self {
        Self {
            window: VecDeque::with_capacity(WINDOW_TICKS + 1),
            loss_ewma: 0.0,
            last_lost_packets: 0,
            last_sent_packets: 0,
            ticks_since_change: 0,
            in_warmup: true,
            warmup_ticks_elapsed: 0,
        }
    }

    fn push_sample(&mut self, sample: Sample) {
        if self.window.len() >= WINDOW_TICKS {
            self.window.pop_front();
        }
        self.window.push_back(sample);
    }

    /// True if the window is full AND every sample in it is app_limited.
    /// This is the upshift precondition: we have a full GOP of evidence
    /// that the link is not being stressed.
    fn window_fully_app_limited(&self) -> bool {
        self.window.len() == WINDOW_TICKS
            && self.window.iter().all(|s| s.app_limited)
    }

    /// Among the non-app-limited samples in the window, is any of them
    /// showing transport-level stress (high queueing delay)?  This tells
    /// us whether the moments of real link saturation were healthy bursts
    /// or genuine congestion events.
    fn non_app_limited_shows_congestion(&self) -> bool {
        self.window
            .iter()
            .filter(|s| !s.app_limited)
            .any(|s| s.queueing_delay_ms > CONGESTION_QUEUEING_DELAY_MS)
    }

}

// ─────────────────────────────────────────────────────────────────────────────
// Decision
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Decision {
    Hold,
    Upshift,
    Downshift,
    Emergency,
}

impl std::fmt::Display for Decision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Hold      => write!(f, "HOLD"),
            Self::Upshift   => write!(f, "UPSHIFT"),
            Self::Downshift => write!(f, "DOWNSHIFT"),
            Self::Emergency => write!(f, "EMERGENCY"),
        }
    }
}

/// Core decision logic.  Reads the window and current state, returns what
/// the actuator should do.  Pure function — no side effects.
fn decide(
    history: &AbrHistory,
    group_backlog: u64,
    backlog_rising: bool,
    current_index: usize,
    max_index: usize,
) -> Decision {
    // ── Emergency: catastrophic backlog overrides everything ──────────────
    if group_backlog >= EMERGENCY_BACKLOG_LEVEL && backlog_rising {
        return Decision::Emergency;
    }

    // ── Cooldown gate: don't reconsider tier until window has refilled ────
    // Loss check is intentionally inside the cooldown so a transient burst
    // of loss doesn't cascade across multiple tiers in a single second.
    if history.ticks_since_change < COOLDOWN_TICKS {
        return Decision::Hold;
    }

    // ── Window must be full to make any informed decision ────────────────
    if history.window.len() < WINDOW_TICKS {
        return Decision::Hold;
    }

    // ── Upshift: full window of app-limited → link has spare capacity ─────
    // When app_limited is true the CC signals (loss, queueing) are unreliable
    // because we're not stressing the link, so we upshift unconditionally.
    if history.window_fully_app_limited() {
        if current_index < max_index {
            return Decision::Upshift;
        }
        return Decision::Hold; // already at top
    }

    // ── At this point: has non-app-limited samples, OR has sustained loss ─
    // Non-app-limited = we stressed the link; inspect quality of those moments.
    // Loss = explicit congestion signal even if CC window wasn't the bottleneck.
    let congested = history.non_app_limited_shows_congestion()
        || history.loss_ewma > CONGESTION_LOSS_RATE;

    if congested {
        if current_index > 0 {
            return Decision::Downshift;
        }
        return Decision::Hold; // already at floor
    }

    // Saturation moments were healthy (queueing ≈ 0, no loss).
    // We're at the sweet spot — hold.
    Decision::Hold
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

fn ewma(current: f64, sample: f64, alpha: f64) -> f64 {
    alpha * sample + (1.0 - alpha) * current
}

async fn sorted_tracks(track_manager: &TrackManager) -> Vec<FullTrackName> {
    let mut tracks: Vec<FullTrackName> =
        track_manager.track_aliases.read().await.values().cloned().collect();
    tracks.sort_by_key(|t| {
        let name = t.to_string();
        if      name.contains("360p")  { 0 }
        else if name.contains("480p")  { 1 }
        else if name.contains("720p")  { 2 }
        else if name.contains("1080p") { 3 }
        else { 4 }
    });
    tracks
}

async fn poison_group(group_id: u64, client: &MOQTClient) {
    let mut decisions = client.group_decisions.write().await;
    decisions.insert(group_id, u64::MAX);
    let _ = client.decision_tx.send(group_id);
}

/// Compute discard timeout for the current tick.  RTT-proportional with a
/// floor and a ceiling; tightens when we've recently downshifted (kill
/// laggards faster while the network is recovering).
fn compute_discard_timeout(decision_just_made: Decision, smoothed_rtt_ms: f64) -> u64 {
    let multiplier = match decision_just_made {
        Decision::Emergency => 1.0,
        Decision::Downshift => 1.2,
        _                   => 2.0,
    };
    let ceiling = match decision_just_made {
        Decision::Emergency => 150.0,
        Decision::Downshift => 250.0,
        _                   => 500.0,
    };
    (multiplier * smoothed_rtt_ms).clamp(80.0, ceiling) as u64
}

// ─────────────────────────────────────────────────────────────────────────────
// Main controller task
// ─────────────────────────────────────────────────────────────────────────────

pub(crate) fn start_abr_controller(
    client: Arc<MOQTClient>,
    track_manager: Arc<TrackManager>,
) {
    tokio::spawn(async move {
        let mut abr_rx = client.abr_rx.lock().await.take().expect("ABR started once");

        let mut current_index    = 0usize; // start at 360p; let upshift earn each tier
        let mut history          = AbrHistory::new();
        let mut group_birth_times: HashMap<u64, Instant> = HashMap::new();
        let mut last_decided_group = 0u64;
        let mut last_decision    = Decision::Hold;
        let mut prev_backlog     = 0u64;

        let mut tick = tokio::time::interval(std::time::Duration::from_millis(TICK_MS));

        info!(
            "ABR controller started: app-limited window design, \
             window={WINDOW_TICKS} ticks ({WINDOW_DURATION_MS} ms), \
             warmup={WARMUP_TICKS} ticks."
        );

        loop {
            tokio::select! {
                // ── Branch 1: new group from publisher ──────────────────
                msg = abr_rx.recv() => {
                    let group_id = match msg {
                        Some(id) => id,
                        None => break,
                    };

                    // Panic flare from close_stream's RESET timebomb
                    if group_id == u64::MAX {
                        warn!("ABR: panic flare — forcing emergency response.");
                        if current_index != 0 {
                            current_index = 0;
                            history.ticks_since_change = 0;
                            client.discard_timeout_ms.store(80, Ordering::Relaxed);
                        }
                        continue;
                    }

                    if group_id <= last_decided_group {
                        continue;
                    }
                    last_decided_group = group_id;

                    // Track birth time for deadline enforcement
                    group_birth_times.entry(group_id).or_insert_with(Instant::now);

                    // Apply current quality decision to this group and a few
                    // recent ones (catches any that arrived while we were
                    // computing the decision).
                    let tracks = sorted_tracks(&track_manager).await;
                    if tracks.is_empty() { continue; }

                    let max_index  = tracks.len().saturating_sub(1);
                    let safe_index = current_index.min(max_index);
                    let target_track = &tracks[safe_index];

                    let target_alias = {
                        let aliases = track_manager.track_aliases.read().await;
                        aliases.iter()
                            .find(|(_, name)| *name == target_track)
                            .map(|(alias, _)| *alias)
                    };

                    if let Some(alias) = target_alias {
                        let mut decisions = client.group_decisions.write().await;
                        for g in group_id.saturating_sub(3)..=group_id {
                            decisions.insert(g, alias);
                            let _ = client.decision_tx.send(g);
                        }
                        decisions.retain(|&k, _| k >= group_id.saturating_sub(5));
                    }

                    crate::telemetry::log_abr_decision(group_id, &target_track.to_string());
                }

                // ── Branch 2: 100 ms tick — sample, decide, actuate ─────
                _ = tick.tick() => {
                    if last_decided_group == 0 { continue; }

                    // Read transport signals
                    let smoothed_rtt_ms = client.connection.rtt().as_secs_f64() * 1000.0;
                    let min_rtt_ms      = client.connection.min_rtt().as_secs_f64() * 1000.0;
                    let latest_rtt_ms   = client.connection.latest_rtt().as_secs_f64() * 1000.0;
                    let cwnd_bytes      = client.connection.cwnd();
                    let app_limited     = client.connection.app_limited();
                    let group_backlog   = client.active_send_tasks.load(Ordering::SeqCst) as u64;
                    let lost_packets    = client.connection.lost_packets();
                    let sent_packets    = client.connection.sent_packets();

                    // Update loss EWMA
                    let delta_lost = lost_packets.saturating_sub(history.last_lost_packets);
                    let delta_sent = sent_packets.saturating_sub(history.last_sent_packets);
                    let sample_loss = delta_lost as f64 / (delta_sent as f64).max(1.0);
                    history.loss_ewma = ewma(history.loss_ewma, sample_loss, 0.2);
                    history.last_lost_packets = lost_packets;
                    history.last_sent_packets = sent_packets;

                    // Push this tick's sample into the window
                    let queueing_delay_ms = (smoothed_rtt_ms - min_rtt_ms).max(0.0);
                    history.push_sample(Sample {
                        app_limited,
                        queueing_delay_ms,
                    });

                    // Backlog rate (just whether it's rising — for emergency detection)
                    let backlog_rising = group_backlog > prev_backlog;
                    prev_backlog = group_backlog;

                    // Telemetry: log everything before deciding
                    let discard_ms = client.discard_timeout_ms.load(Ordering::Relaxed);
                    crate::telemetry::log_network_stats(
                        smoothed_rtt_ms as u128, min_rtt_ms as u128, latest_rtt_ms as u128,
                        cwnd_bytes, group_backlog, app_limited, discard_ms,
                    );

                    // Window-fill diagnostic — useful when reading dashboards
                    let window_app_limited_count = history.window
                        .iter().filter(|s| s.app_limited).count();
                    let window_app_limited_frac = if history.window.is_empty() {
                        0.0
                    } else {
                        window_app_limited_count as f64 / history.window.len() as f64
                    };

                    crate::telemetry::log_abr_state(
                        if history.in_warmup { "WARMUP" } else { "ACTIVE" },
                        current_index,
                        queueing_delay_ms,
                        window_app_limited_frac,
                        backlog_rising,
                        history.loss_ewma,
                    );

                    // ── Warmup: count down, then exit ──────────────────
                    if history.in_warmup {
                        history.warmup_ticks_elapsed += 1;
                        if history.warmup_ticks_elapsed >= WARMUP_TICKS {
                            history.in_warmup = false;
                            history.ticks_since_change = 0;
                            history.window.clear(); // start window fresh post-warmup
                            info!("ABR: warmup complete (min_rtt={:.2}ms)", min_rtt_ms);
                        }
                    } else {
                        // ── Active: make a decision ────────────────────
                        let tracks    = sorted_tracks(&track_manager).await;
                        let max_index = tracks.len().saturating_sub(1);

                        let decision = decide(
                            &history,
                            group_backlog,
                            backlog_rising,
                            current_index,
                            max_index,
                        );

                        match decision {
                            Decision::Hold => {
                                history.ticks_since_change += 1;
                            }
                            Decision::Upshift => {
                                let new_index = (current_index + 1).min(max_index);
                                info!(
                                    "ABR: UPSHIFT {} → {} (window 100% app-limited)",
                                    current_index, new_index
                                );
                                current_index = new_index;
                                history.ticks_since_change = 0;
                                history.window.clear(); // need fresh data at new tier
                            }
                            Decision::Downshift => {
                                let new_index = current_index.saturating_sub(1);
                                let reason = if history.loss_ewma > CONGESTION_LOSS_RATE {
                                    "loss"
                                } else {
                                    "congested saturation"
                                };
                                warn!(
                                    "ABR: DOWNSHIFT {} → {} ({})",
                                    current_index, new_index, reason
                                );
                                current_index = new_index;
                                history.ticks_since_change = 0;
                                history.window.clear();
                                // Drop in-flight high-quality groups that
                                // we just decided we can't afford
                                let now = Instant::now();
                                let victims: Vec<u64> = group_birth_times
                                    .iter()
                                    .filter_map(|(&gid, birth)| {
                                        let p = now.duration_since(*birth).as_millis() as f64
                                              / LATENCY_BUDGET_MS;
                                        if p > 0.4 { Some(gid) } else { None }
                                    })
                                    .collect();
                                for gid in victims {
                                    poison_group(gid, &client).await;
                                    group_birth_times.remove(&gid);
                                }
                            }
                            Decision::Emergency => {
                                warn!(
                                    "ABR: EMERGENCY (backlog={} rising) — slam to 0",
                                    group_backlog
                                );
                                current_index = 0;
                                history.ticks_since_change = 0;
                                history.window.clear();
                                let now = Instant::now();
                                let victims: Vec<u64> = group_birth_times
                                    .iter()
                                    .filter_map(|(&gid, birth)| {
                                        let p = now.duration_since(*birth).as_millis() as f64
                                              / LATENCY_BUDGET_MS;
                                        if p > 0.5 { Some(gid) } else { None }
                                    })
                                    .collect();
                                for gid in victims {
                                    poison_group(gid, &client).await;
                                    group_birth_times.remove(&gid);
                                }
                            }
                        }

                        last_decision = decision;
                    }

                    // ── Apply discard timeout (every tick) ──────────────
                    let timeout = compute_discard_timeout(last_decision, smoothed_rtt_ms);
                    client.discard_timeout_ms.store(timeout, Ordering::Relaxed);

                    // ── Deadline-driven poisoning (every tick) ──────────
                    let now = Instant::now();
                    let deadline_victims: Vec<u64> = group_birth_times
                        .iter()
                        .filter_map(|(&gid, birth)| {
                            let elapsed_ms   = now.duration_since(*birth).as_millis() as f64;
                            let remaining_ms = LATENCY_BUDGET_MS - elapsed_ms;
                            if remaining_ms < smoothed_rtt_ms * 1.2 {
                                Some(gid)
                            } else {
                                None
                            }
                        })
                        .collect();

                    for gid in deadline_victims {
                        let elapsed = now.duration_since(group_birth_times[&gid]).as_millis();
                        warn!(
                            "ABR: deadline-poison group {} ({}ms elapsed, budget {}ms)",
                            gid, elapsed, LATENCY_BUDGET_MS as u64
                        );
                        poison_group(gid, &client).await;
                        group_birth_times.remove(&gid);
                    }

                    // ── Reap stale birth-time records ───────────────────
                    // Bound at the budget itself, not 2× — anything past its
                    // budget is either delivered or already dead, not in flight.
                    group_birth_times.retain(|_, birth| {
                        birth.elapsed().as_millis() < LATENCY_BUDGET_MS as u128
                    });
                }
            }
        }
    });
}