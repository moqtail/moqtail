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
//! Everything below is the behaviour all three versions share unless a
//! sentence says otherwise; [`WasdAlgorithmFactory`] documents each reach.
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
//! fix. It also removes the need for a cooldown: a successful downshift drains
//! the signal by itself, because the streams that were producing it are no
//! longer at the tier being measured.
//!
//! **A monitor inside the group.** Groups are the algorithm's clock but not the
//! path's: bandwidth moves faster than 25 groups a second. The same depth is
//! therefore read again on every periodic sample the host takes mid-group, and
//! a reading that stays at the downshift depth for
//! `DOWNSHIFT_SUBGROUP_READS` consecutive samples in a row downshifts without
//! waiting for the next boundary. Requiring the streak rather than the last
//! reading is what makes this safe: the tail of a healthy group produces a
//! single deep reading, because the next group's stream opens before the
//! previous one has drained, and real congestion produces it until something is
//! done. The streak re-arms when it fires, so one burst cannot walk the ladder
//! tick by tick, and a dip costs a full fresh clean streak before the climb
//! resumes.
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
use crate::algorithms::wasd::state::Monitor;
use crate::algorithms::wasd::state::Step;

#[cfg(test)]
mod tests;

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
pub struct WasdAlgorithmFactory {
  id: u64,
  reach: Reach,
}

impl WasdAlgorithmFactory {
  /// The classic: downshift on depth, retroactive nothing.
  pub const CLASSIC: Self = Self {
    id: WASD_ALGORITHM_ID,
    reach: Reach::Nothing,
  };
  /// The retroactive downshift of the newest overtaken group.
  pub const V2: Self = Self {
    id: WASD_V2_ALGORITHM_ID,
    reach: Reach::Newest,
  };
  /// The retroactive double downshift.
  pub const V3: Self = Self {
    id: WASD_V3_ALGORITHM_ID,
    reach: Reach::AllStaged,
  };
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
}

impl Default for WasdParams {
  fn default() -> Self {
    Self {
      monitor_reads: state::DOWNSHIFT_SUBGROUP_READS,
    }
  }
}

impl WasdParams {
  /// From the operator's `key=value` pairs, with `wasd.reads` the only knob.
  pub fn from_tuning(tuning: &Tuning) -> Self {
    let defaults = Self::default();
    Self {
      monitor_reads: tuning.usize("wasd.reads", defaults.monitor_reads as usize) as u32,
    }
  }
}

/// The tier walk, measured at the tier it is on.
#[derive(Debug)]
pub struct WasdAlgorithm {
  ladder: LadderState,
  reach: Reach,
}

impl WasdAlgorithm {
  /// The classic: a connection at the bottom tier, exploring, and a
  /// downshift that reaches back into nothing.
  pub fn new() -> Self {
    Self {
      ladder: LadderState::new(),
      reach: Reach::Nothing,
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
    }
  }

  /// Any version, with the monitor's patience set by the operator.
  pub fn with_reach_and_params(reach: Reach, params: WasdParams) -> Self {
    Self {
      ladder: LadderState::new_with_reads(params.monitor_reads),
      reach,
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
      DecisionReason::GroupBoundary => {
        // The exploration is walked in whole groups, and it is a group boundary
        // that tells the algorithm the ladder is taller than the path. A deep
        // reading inside a group does not end it, and does not act either: see
        // the periodic arm below.
        if self.ladder.note_congestion(depth) {
          tracing::debug!(
            client_id = input.client_id,
            group_id = input.group_id,
            depth,
            "SSTS wasd: exploration over (congestion)"
          );
        }
        match self.ladder.on_group(depth) {
          Step::Held => {}
          Step::Upshift { from, to } => {
            tracing::debug!(
              client_id = input.client_id,
              group_id = input.group_id,
              from,
              to,
              depth,
              exploring = self.ladder.exploring(),
              "SSTS wasd: UPSHIFT"
            );
            if self.ladder.note_top_of_ladder(max_index, any_active(input)) {
              tracing::debug!(
                client_id = input.client_id,
                group_id = input.group_id,
                tier = to,
                "SSTS wasd: exploration over (top of the ladder)"
              );
            }
          }
          Step::Downshift { from, to } => tracing::debug!(
            client_id = input.client_id,
            group_id = input.group_id,
            from,
            to,
            depth,
            "SSTS wasd: DOWNSHIFT (group boundary)"
          ),
        }
      }
      DecisionReason::PeriodicSample => {
        // The monitor's 100 ms reading of the delivered tier. The algorithm's
        // whole state is this one number, and a tier change is the rare
        // outcome: every reading goes to the log (the run's dashboard is
        // built from these lines), the changes log on top of them with more.
        tracing::debug!(
          client_id = input.client_id,
          group_id = input.group_id,
          tier = self.ladder.tier(),
          depth,
          "SSTS wasd: depth reading"
        );
        match self.ladder.on_tick(depth, any_active(input)) {
          Monitor::Held => {}
          Monitor::Downshift { from, to } => tracing::debug!(
            client_id = input.client_id,
            group_id = input.group_id,
            from,
            to,
            depth,
            "SSTS wasd: DOWNSHIFT (mid-transmission)"
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
        match (self.reach, resend_deadline_ms(set)) {
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

/// A re-sent group gets half the buffer the subscriber negotiated for the set.
///
/// This is part of what choosing this algorithm means, and it is what makes the
/// re-send worth its bytes: the relay is finding out whether the lighter
/// delivery makes a deadline, not whether the heavy one did, and a group that
/// misses a deadline it was already late for is a group the subscriber was going
/// to discard anyway. Half, because a re-send starts behind the live stream it
/// replaced and has no way to catch up; if it cannot make the deadline at half,
/// it could not have made it at full either, and the subscriber is better off
/// with the gap and the next group at the new quality.
///
/// A set that negotiated no buffer has granted no permission to shorten, so
/// `None` leaves the relay's rule of never discarding in charge. The floor of one
/// is an edge rather than a policy: 0 means "never" in this contract, so halving
/// a 1 ms buffer must not turn a very short deadline into an infinite one.
fn resend_deadline_ms(set: &SetSnapshot) -> Option<u64> {
  if set.subscriber_buffer_ms == 0 {
    return None;
  }
  Some((set.subscriber_buffer_ms / 2).max(1))
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
/// it was meant to relieve.
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
