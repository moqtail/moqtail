// Copyright 2026 The MOQtail Authors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use it except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! WASD (Within-tier Active Stream Depth): the depth measured at the quality
//! actually being delivered, sampled
//! while the group is still in flight, with a downshift that reaches back into
//! the groups it overtook.
//!
//! One state machine, advertised under three ids — [`WASD_ALGORITHM_ID`]
//! (`wasd`, the classic), [`WASD_V2_ALGORITHM_ID`] (`wasd-v2`) and
//! [`WASD_V3_ALGORITHM_ID`] (`wasd-v3`) — that differ only in
//! [`Reach`]: how far a downshift reaches back into the groups it overtook.
//! Each version is declared, with its own documentation, in its own file:
//! [`classic`], [`v2`], [`v3`]. Everything below is the behaviour all three
//! versions share unless a sentence says otherwise;
//! [`WasdAlgorithmFactory`] documents each reach.
//!
//! Three changes over [`crate::algorithms::asd_v11`], and they belong
//! together because each one is what makes the next one survivable.
//!
//! **Depth at the current tier.** The signal is not the connection's open
//! forwarding streams; it is those of them still carrying the member the last
//! decision chose. With three streams in flight — two at 1080p, one at 720p,
//! and 720p current — the depth is one, not three. Everything else in flight
//! belongs to a quality the algorithm has already given up on, and counting it
//! makes the recovery from a downshift look like the congestion it was meant to
//! fix. What it drains is the signal the decision itself produced — the
//! rejected streams are no longer at the tier being measured. What it cannot
//! drain is the queue a short dip leaves behind: the overlap that queue
//! causes at the next group boundary still reads as congestion at the new
//! tier, which is the retroactive versions' measured shortcoming the
//! README keeps under its own heading.
//!
//! **One clock, two certainties.** Groups are the algorithm's clock but not
//! the path's: bandwidth moves faster than 25 groups a second, so the same
//! depth is read on every sample the host takes — at the group boundary and
//! mid-group alike — through one state-machine method. What the two samples
//! may conclude differs by how much they can see. The boundary's reading is
//! certain: two streams of the delivered tier still open when a group's first
//! Object arrives means the delivery is two groups behind, because the new
//! group's stream has not opened yet — so one congested boundary reading
//! downshifts alone and ends the exploration. The mid-group reading is not:
//! a single deep sample is the normal handoff overlap, the new group's stream
//! open while the previous one's FIN-ACK is still in flight, so the monitor
//! waits for
//! `DOWNSHIFT_SUBGROUP_READS` consecutive congested samples — and a reading
//! that does not fire touches nothing, least of all the clean streak the
//! climb counts on. The streak re-arms when it fires, so one burst cannot
//! walk the ladder tick by tick, and a fired downshift costs a full fresh
//! clean streak before the climb resumes. Upshifts belong to the boundary
//! alone — the moment the relay's forward gate is waiting to record that
//! group's track — so the tier never moves mid-group and never disagrees with
//! what is on the wire.
//!
//! **A downshift that reaches back.** When the tier falls, the groups still in
//! flight at the higher member were overtaken by a decision they are still
//! inside. This is what the versions tune: the classic issues no request at
//! all, v2 names the set with [`Reach::Newest`], and v3 stages the re-send by
//! age. Where a request is made, this relay's mechanism flips those groups to
//! the member asked for, resets the streams still carrying the old one, and
//! marks the groups as re-sent so the streams the subscriber then causes to be
//! opened do not read as new congestion. Without the mark the downshift would
//! manufacture the evidence for the next one.
//!
//! Two parts of the original experiment are deliberately not here, because they
//! are mechanism and this crate has none. The re-sent group is *not* replayed in
//! full from the relay's cache: the overtaken delivery is cancelled and the new
//! member's own objects resume from wherever that group had got to, which means
//! the subscriber receives a group that starts mid-way and may not decode. The
//! replay belongs in the host's forwarding path, where the track cache is, and
//! it has conformance consequences (the `FIRST_OBJECT` flag on a stream that
//! does not start at the subgroup's first object) that are not this algorithm's
//! to settle. The stream-discard timeout is likewise not halved for re-sent
//! groups; it stays the negotiated value for the set, which is the subscriber's
//! permission and not the algorithm's to spend.

use crate::AbrAlgorithm;
use crate::AbrAlgorithmFactory;
use crate::AbrInput;
use crate::DecisionReason;
use crate::Selection;
use crate::SetSnapshot;
use crate::Tuning;
use crate::algorithms::wasd::state::LadderState;
use crate::algorithms::wasd::state::SLOW_START_UPSHIFT_STREAK;
use crate::algorithms::wasd::state::Step;

#[cfg(test)]
mod tests;

pub mod classic;
pub mod v2;
pub mod v3;

mod state;

/// WASD, the within-tier depth read: the first of the three version ids in
/// the range the specification leaves to implementations.
pub const WASD_ALGORITHM_ID: u64 = crate::registry::PRIVATE_ALGORITHM_ID_BASE + 3;

/// WASD v2, the retroactive downshift of the newest overtaken group.
pub const WASD_V2_ALGORITHM_ID: u64 = crate::registry::PRIVATE_ALGORITHM_ID_BASE + 4;

/// WASD v3, the retroactive double downshift: oldest overtaken groups one
/// rung below the new one.
pub const WASD_V3_ALGORITHM_ID: u64 = crate::registry::PRIVATE_ALGORITHM_ID_BASE + 5;

/// How far a downshift reaches back into the groups it overtook — the one
/// thing the three wasd versions differ in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach {
  /// Classic: the decision stands from this group on, and the groups already
  /// in flight finish on the member they were carrying.
  Nothing,
  /// Cancel the newest overtaken group and re-send it on the new rung; let
  /// the older ones finish where they are.
  Newest,
  /// Cancel every overtaken group: the newest re-sent on the new rung, every
  /// older one on the rung below that (the ladder's bottom being the floor).
  AllStaged,
}

impl Reach {
  /// The name this version is advertised and logged under.
  pub fn variant(&self) -> &'static str {
    match self {
      Reach::Nothing => "wasd",
      Reach::Newest => "wasd-v2",
      Reach::AllStaged => "wasd-v3",
    }
  }
}

/// Factory for [`WasdAlgorithm`]: one instance per connection, the version
/// told by the id it is registered under.
///
/// The three versions are the same state machine — the depth read at the
/// delivered tier, the mid-group monitor, the same exploration and cautious
/// streak, the same `wasd.reads` knob — and differ only in how far a
/// downshift reaches back:
///
/// * [`Reach::Nothing`], advertised as `wasd` at
///   [`WASD_ALGORITHM_ID`]: the classic. A downshift is forward-looking
///   only.
/// * [`Reach::Newest`], advertised as `wasd-v2` at [`WASD_V2_ALGORITHM_ID`]:
///   the downshift also cancels the newest group it overtook and has it
///   re-sent at the new rung. 1080p down to 720p at group 4 means group 3,
///   still in flight at 1080p, is cancelled and re-sent at 720p; groups
///   older than that keep their delivery.
/// * [`Reach::AllStaged`], advertised as `wasd-v3` at
///   [`WASD_V3_ALGORITHM_ID`]: the same, one step further back — groups
///   older than the newest are cancelled too and re-sent a rung below the
///   new one (1080p down to 720p: the newest overtaken group goes 720p,
///   everything older goes 480p), because the older a group is, the less of
///   its delivery window is left to a re-send.
///
/// Each version's own declaration, with its id, is in its file:
/// [`classic::FACTORY`], [`v2::FACTORY`], [`v3::FACTORY`].
pub struct WasdAlgorithmFactory {
  id: u64,
  reach: Reach,
}

impl AbrAlgorithmFactory for WasdAlgorithmFactory {
  fn id(&self) -> u64 {
    self.id
  }

  fn name(&self) -> &'static str {
    self.reach.variant()
  }

  fn create(&self, tuning: &Tuning) -> Box<dyn AbrAlgorithm> {
    Box::new(WasdAlgorithm::with_reach_and_params(
      self.reach,
      WasdParams::from_tuning(tuning),
    ))
  }
}

/// How patient the mid-transmission monitor is.
#[derive(Debug, Clone, Copy)]
pub struct WasdParams {
  /// Consecutive mid-group readings at the downshift depth before the monitor
  /// acts. One is a reaction to a single sample; the default asks for the pattern.
  pub monitor_reads: u32,
  /// Probing risk tolerance in `(0.0, 1.0]`: the price of a climb in clean
  /// groups per unit of relative bitrate step. Higher climbs more eagerly;
  /// lower demands more evidence before stepping across a large jump.
  pub rho: f64,
  /// Operator's floor for `U_min`, in clean groups, used in place of the
  /// derived `ceil(B / G)` when set. `None` keeps the derived horizon; a
  /// subscriber that signals `U_min` in `algorithm_params` outranks either.
  pub u_min: Option<u64>,
}

impl WasdParams {
  /// The tolerance the algorithm was written with: a step is worth one clean
  /// group per unit of relative jump, and the buffer floor decides the rest.
  pub const DEFAULT_RHO: f64 = 1.0;
}

impl Default for WasdParams {
  fn default() -> Self {
    Self {
      monitor_reads: state::DOWNSHIFT_SUBGROUP_READS,
      rho: Self::DEFAULT_RHO,
      u_min: None,
    }
  }
}

impl WasdParams {
  /// From the operator's `key=value` pairs: `wasd.reads` for the monitor's
  /// patience, `wasd.rho` for the probing risk tolerance, and `wasd.umin`
  /// (a group count, `0` meaning unset) for a floor to use in place of the
  /// derived `ceil(B / G)`.
  pub fn from_tuning(tuning: &Tuning) -> Self {
    let defaults = Self::default();
    Self {
      monitor_reads: tuning.usize("wasd.reads", defaults.monitor_reads as usize) as u32,
      rho: tuning.f64("wasd.rho", defaults.rho),
      u_min: match tuning.usize("wasd.umin", 0) {
        0 => None,
        v => Some(v as u64),
      },
    }
  }
}

/// The tier walk, measured at the tier it is on.
#[derive(Debug)]
pub struct WasdAlgorithm {
  ladder: LadderState,
  reach: Reach,
  /// The probing risk tolerance this instance climbs on.
  rho: f64,
  /// The operator's `U_min` floor, used when the subscriber signals none.
  u_min: Option<u64>,
}

impl WasdAlgorithm {
  /// The classic: a connection at the bottom tier, exploring, and a
  /// downshift that reaches back into nothing.
  pub fn new() -> Self {
    Self {
      ladder: LadderState::new(),
      reach: Reach::Nothing,
      rho: WasdParams::DEFAULT_RHO,
      u_min: None,
    }
  }

  /// The classic, with the monitor's patience set by the operator.
  pub fn with_params(params: WasdParams) -> Self {
    Self::with_reach_and_params(Reach::Nothing, params)
  }

  /// v2: the downshift cancels the newest overtaken group and has it re-sent
  /// at the new rung.
  pub fn v2() -> Self {
    Self::with_reach(Reach::Newest)
  }

  /// v3: the downshift reaches every overtaken group, staging the older ones
  /// a rung below the new one.
  pub fn v3() -> Self {
    Self::with_reach(Reach::AllStaged)
  }

  /// The classic walk with a chosen reach.
  pub fn with_reach(reach: Reach) -> Self {
    Self {
      ladder: LadderState::new(),
      reach,
      rho: WasdParams::DEFAULT_RHO,
      u_min: None,
    }
  }

  /// Any version, with the monitor's patience, the probing risk tolerance and
  /// the `U_min` floor set by the operator.
  pub fn with_reach_and_params(reach: Reach, params: WasdParams) -> Self {
    Self {
      ladder: LadderState::new_with_reads(params.monitor_reads),
      reach,
      rho: params.rho,
      u_min: params.u_min,
    }
  }

  /// The version this instance runs, which is also its advertised name.
  pub fn reach(&self) -> Reach {
    self.reach
  }

  /// Whether the connection is still in its exploration phase.
  pub fn exploring(&self) -> bool {
    self.ladder.exploring()
  }
}

impl Default for WasdAlgorithm {
  fn default() -> Self {
    Self::new()
  }
}

impl AbrAlgorithm for WasdAlgorithm {
  fn decide(&mut self, input: &AbrInput) -> Selection {
    // The reading is taken at the tier the last decision landed on, which is
    // what makes a downshift self-clearing.
    let depth = depth_at_current_tier(input, self.ladder.tier());
    let before = self.ladder.tier();
    let max_index = input
      .sets
      .iter()
      .map(|set| set.members.len())
      .max()
      .unwrap_or(1)
      .saturating_sub(1);

    match input.reason {
      DecisionReason::GroupBoundary | DecisionReason::PeriodicSample => {
        // The cautious streak the next step would need: the subscriber's own
        // clock (buffer over cadence) floored against the step's relative
        // bitrate jump, priced by the probing risk tolerance. Computed inside
        // the arm because a stream timeout is not a reading and never needs it.
        let upshift_streak = upshift_streak_needed(input, before, self.rho, self.u_min);
        // Whether the climb was still exploring when this reading arrived:
        // that is what decides the streak an upshift here actually waited, and
        // the log reports the waited number, not the computed U(k).
        let exploring_before = self.ladder.exploring();

        // One reading, one method: what a sample may conclude is the state
        // machine's business. The boundary's reading is certain — two streams
        // of the delivered tier still open when a group's first Object
        // arrives is two groups behind — and acts alone; the mid-group
        // reading needs the monitor's streak, because the handoff overlap
        // produces its depth all by itself. Upshifts belong to the boundary
        // alone: the moment the relay's forward gate is waiting to record
        // that group's track. The algorithm's whole state is this one number,
        // and a tier change is the rare outcome: every reading goes to the
        // log (the run's dashboard is built from these lines), the changes
        // log on top of them with more.
        tracing::debug!(
          client_id = input.client_id,
          group_id = input.group_id,
          tier = self.ladder.tier(),
          depth,
          "SSTS wasd: depth reading"
        );
        match self.ladder.on_sample(
          depth,
          input.group_id,
          any_active(input),
          max_index,
          upshift_streak,
        ) {
          Step::Held => {}
          Step::Upshift { from, to } => tracing::debug!(
            client_id = input.client_id,
            group_id = input.group_id,
            from,
            to,
            depth,
            upshift_streak = if exploring_before {
              SLOW_START_UPSHIFT_STREAK
            } else {
              upshift_streak.max(1)
            },
            exploring = self.ladder.exploring(),
            "SSTS wasd: UPSHIFT"
          ),
          Step::Downshift { from, to } => tracing::debug!(
            client_id = input.client_id,
            group_id = input.group_id,
            from,
            to,
            depth,
            "SSTS wasd: DOWNSHIFT"
          ),
        }
      }
      // A reset stream is a consequence of a decision this algorithm already
      // made, not an independent reading of the path, and wasd is the
      // algorithm that says so: the depth already reflects the stream leaving.
      // Acting on the reset as well would count the same event twice.
      DecisionReason::StreamTimeout => {}
    }

    self.ladder.clamp_to(max_index);
    let after = self.ladder.tier();
    let mut selection = crate::algorithms::serve_tier(input, after);

    // The groups still in flight above the new rung were overtaken by this
    // decision. Naming the sets is all the algorithm can do; the host works out
    // which groups of that set are affected and what its own state allows.
    // How far the request reaches is the one thing the versions differ in:
    // the classic names nothing, v2 names the set with the newest-overtaken
    // flag, and v3 stages the re-send by age — the newest overtaken group
    // keeps the rung this decision landed on, every older one goes one rung
    // below it, because the older a group is, the less of its delivery window
    // the re-send has to finish inside.
    if after < before {
      for set in input
        .sets
        .iter()
        .filter(|set| set.active && !set.members.is_empty())
      {
        match (self.reach, resend_deadline_ms(input, set, after)) {
          (Reach::Nothing, _) => {}
          (Reach::Newest, Some(deadline)) => {
            selection.supersede_newest_with_deadline(set.id, deadline)
          }
          (Reach::Newest, None) => selection.supersede_newest(set.id),
          (Reach::AllStaged, Some(deadline)) => {
            selection.supersede_age_staged_with_deadline(set.id, deadline)
          }
          (Reach::AllStaged, None) => selection.supersede_age_staged(set.id),
        }
      }
    }

    selection
  }
}

/// The number of consecutive uncongested groups `U(k)` required to step from
/// `current_tier` (`k`) to `current_tier + 1`, taken as the maximum over the
/// connection's active sets (one tier serves them all, so the climb answers
/// to the most demanding ladder):
///
/// ```text
/// U_min = signalled count, else wasd.umin, else ceil(B / G), else UPSHIFT_GOP_STREAK
/// U(k)  = max(U_min, ceil(((R_{k+1} - R_k) / R_k) / rho))
/// ```
///
/// `U_min` is the minimum upshift stability threshold, in clean groups. A
/// subscriber may signal it explicitly in `algorithm_params` (slot 0 of the
/// two-slot layout, in units of media groups); when it signals none, the
/// operator's `wasd.umin` floor answers, and without one the recommended
/// default derives it as one buffer-drainage horizon, `ceil(B / G)`, from the
/// subscriber's own clocks — `B` the negotiated buffer
/// (`subscriber_buffer_ms`) and `G` the observed group cadence
/// (`group_span_ms`) — so the floor scales with the client's latency budget
/// and the media's GoP instead of a constant.
/// The step term prices the jump itself: a ladder whose next rung is 3x the
/// current one is a 3x bigger bet than one 20% up, and `rho` is how much risk
/// to bet per clean group — the subscriber's signalled percentage
/// (`1..=100`, trailing slot) where it signalled one, else the operator's.
///
/// Falls back to `UPSHIFT_GOP_STREAK` for `U_min` when neither the subscriber
/// nor the operator spoke and the buffer or the cadence is unknown (0): an
/// unbuffered connection granted no window to reason about, and a set whose
/// first group has not closed yet has no cadence.
fn upshift_streak_needed(
  input: &AbrInput,
  current_tier: usize,
  default_rho: f64,
  default_u_min: Option<u64>,
) -> u64 {
  use crate::algorithms::asd::state::UPSHIFT_GOP_STREAK;

  // wasd serves one tier across every set on the connection, so the climb must
  // be as cautious as the most demanding ladder it carries: take the largest
  // U(k) any active set prices for this step. A connection with no active set
  // has nothing to price and falls back to the written default.
  input
    .sets
    .iter()
    .filter(|set| set.active && !set.members.is_empty())
    .map(|set| set_upshift_streak(set, current_tier, default_rho, default_u_min))
    .max()
    .unwrap_or(UPSHIFT_GOP_STREAK)
}

/// The `U(k)` one set prices for the step out of `current_tier`: its own
/// floor — the count it signalled, else the operator's, else its own
/// buffer-drainage horizon `ceil(B / G)` — against its own relative bitrate
/// jump, under its own signalled risk tolerance where it signalled one, else
/// the operator's.
fn set_upshift_streak(
  set: &SetSnapshot,
  current_tier: usize,
  default_rho: f64,
  default_u_min: Option<u64>,
) -> u64 {
  use crate::algorithms::asd::state::UPSHIFT_GOP_STREAK;

  // U_min: the count the subscriber signalled for its own set wins, then the
  // operator's floor, then the recommended buffer-drainage horizon
  // ceil(B / G), then the written default when neither clock is known.
  let u_min = signaled_u_min(set).or(default_u_min).unwrap_or_else(|| {
    if set.subscriber_buffer_ms > 0 && set.group_span_ms > 0 {
      set.subscriber_buffer_ms.div_ceil(set.group_span_ms)
    } else {
      UPSHIFT_GOP_STREAK
    }
  });

  // rho: the subscriber's own signalled percentage wins over the operator's.
  let rho = set
    .algorithm_params
    .last()
    .copied()
    .filter(|&v| (1..=100).contains(&v))
    .map(|pct| (pct as f64 / 100.0).clamp(0.01, 1.0))
    .unwrap_or_else(|| default_rho.clamp(0.01, 1.0));

  // The relative bitrate step of the rung this connection stands on.
  let max_rung = set.members.len().saturating_sub(1);
  let k = current_tier.min(max_rung);
  let next = (k + 1).min(max_rung);

  if next <= k {
    // The top rung: nothing to price, the floor alone governs.
    return u_min.max(1);
  }

  let r_k = set.members[k].0 as f64;
  let r_next = set.members[next].0 as f64;

  let step_term = if r_k > 0.0 && r_next > r_k {
    (((r_next - r_k) / r_k) / rho).ceil() as u64
  } else {
    0
  };

  u_min.max(step_term).max(1)
}

/// The minimum upshift stability threshold the subscriber signalled for its
/// own set, in units of media groups: slot 0 of `algorithm_params` when the
/// subscriber sent the two-slot layout and the slot is non-zero. A one-slot
/// vec is the older shape that signals `rho` alone, and a zero means "not
/// signalled" - the derived `ceil(B / G)` default stands.
fn signaled_u_min(set: &SetSnapshot) -> Option<u64> {
  if set.algorithm_params.len() >= 2 {
    set.algorithm_params.first().copied().filter(|&v| v > 0)
  } else {
    None
  }
}

/// A re-sent group inherits the remaining buffer budget of the overtaken
/// stream it replaces (`subscriber_buffer_ms - age_ms`).
///
/// This is part of what choosing this algorithm means, and it is what makes the
/// re-send worth its bytes: the relay is finding out whether the lighter
/// delivery makes a deadline, not whether the heavy one did. The budget is not
/// invented at the decision but inherited: the overtaken stream has already
/// spent part of the subscriber's window on the wire, and what it has left is
/// exactly what its replacement has left too. Half the buffer was a guess at
/// that number; this is the measurement.
///
/// The newest overtaken group donates its remaining time, because it is the
/// group the re-send replaces in the delivery order — older overtaken groups
/// have less of the window left, and v3's staging already prices them one rung
/// lower for it. A re-send of a group whose stream had barely started inherits
/// nearly the whole buffer, which is right: it has nearly the whole window. A
/// stream that is itself a re-send donates too: its age runs from the moment
/// the repair started, which is exactly what a second repair of the same group
/// has left of the same window.
///
/// A set that negotiated no buffer (`subscriber_buffer_ms == 0`) has granted
/// no permission to discard media, so `None` leaves the relay's rule of never
/// discarding in charge. The floor of 1 ms ensures that an overtaken stream
/// whose age already meets or exceeds the buffer expires immediately rather
/// than falling through to 0 (which means "never" in the relay contract).
fn resend_deadline_ms(input: &AbrInput, set: &SetSnapshot, new_tier: usize) -> Option<u64> {
  if set.subscriber_buffer_ms == 0 {
    return None;
  }

  let rung = new_tier.min(set.members.len().saturating_sub(1));
  let newest_overtaken_age_ms = input
    .streams
    .iter()
    .filter(|s| {
      s.set_id == set.id
        && set
          .members
          .iter()
          .skip(rung + 1)
          .any(|&(_, track_id)| track_id == s.relay_track_id)
    })
    .max_by_key(|s| s.group_id)
    .map(|s| s.age_ms)
    .unwrap_or(0);

  Some(
    set
      .subscriber_buffer_ms
      .saturating_sub(newest_overtaken_age_ms)
      .max(1),
  )
}

/// Whether there is anything for a mid-group decision to act on: a connection
/// whose sets are idle, or whose ladders are still being filled, has no delivery
/// to overtake.
fn any_active(input: &AbrInput) -> bool {
  input
    .sets
    .iter()
    .any(|set| set.active && !set.members.is_empty())
}

/// The congestion signal: live forwarding streams carrying the member each
/// active set holds at `tier`, re-sends excluded.
///
/// A re-sent stream is the relay's own reaction to the previous downshift. That
/// is the whole reason the depth is measured at one rung: if the streams the
/// downshift itself caused counted, every downshift would report the congestion
/// it was meant to relieve. Measured, not theorised: with the re-sends in the
/// depth, a 500 ms dip on `PROFILE_JITTER` cost three rungs (3 to 0) instead of
/// one, each repair overlapping the next group manufacturing the evidence for
/// the next step down.
fn depth_at_current_tier(input: &AbrInput, tier: usize) -> u64 {
  input
    .sets
    .iter()
    .filter(|set| set.active && !set.members.is_empty())
    .map(|set| {
      let rung = tier.min(set.members.len() - 1);
      input.open_streams_at_track(set.id, set.members[rung].1)
    })
    .sum()
}
