// Copyright 2026 The MOQtail Authors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Sender-side track switching (SSTS) for moqtail.
//!
//! SSTS lets a relay serve one logical track out of a *switching set*: several
//! encodings of the same content, ordered by throughput, of which the relay
//! forwards at most one per media group. Which one is a bandwidth allocation
//! decision, and this crate owns the boundary around that decision:
//!
//! * [`AbrInput`] is everything an algorithm may see. It is built by the host
//!   (the moqtail relay) from the QUIC connection and the switching sets, and
//!   it borrows: no host type appears in this crate's signatures.
//! * [`AbrAlgorithm`] maps that input to a [`Selection`]. It is synchronous
//!   and holds per-connection state, so an algorithm is a plain state machine
//!   that can be unit tested without a transport.
//! * [`registry`] lists the algorithms a relay runs and turns an algorithm id
//!   into a fresh instance.
//!
//! Adding an algorithm therefore means: one directory under [`algorithms`],
//! one line in the registry, and nothing in the host. See the crate README.
//!
//! This crate deliberately knows nothing about how a decision is enforced. The
//! host applies the returned selection to the groups it forwards, ignores any
//! selection it cannot honour, and owns every subscription, track and stream
//! the algorithm influences.

pub mod algorithms;
pub mod registry;

use std::collections::HashMap;

/// What an algorithm decided: switching set id -> the `relay_track_id` of the
/// member to forward for this group, or `None` when the set forwards nothing.
///
/// Every set handed to [`AbrAlgorithm::decide`] must appear in the map. A set
/// the selection leaves out is treated as `None` by the host and reported,
/// never waited on forever.
pub type Selection = HashMap<u64, Option<u64>>;

/// One switching set, copied out of the host at decision time.
///
/// Members are sorted by ascending throughput threshold, so `members.first()`
/// is the lowest tier and `members.last()` the highest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetSnapshot {
  /// Switching set id, as advertised by the publisher.
  pub id: u64,
  /// The algorithm this set is assigned to. The host groups sets by this
  /// field and only shows an algorithm the sets it owns.
  pub algorithm_id: u64,
  /// Priority tier: 0 is the highest. Lower ranks are served first.
  pub rank: u8,
  /// Relative weight inside the rank tier. Zero-weight sets are never served.
  pub weight: u64,
  /// Whether the set has an active subscriber right now. An inactive set is
  /// not served, so a subscriber that stops consuming frees the budget for
  /// other sets.
  pub active: bool,
  /// `(throughput threshold kbps, relay track id)`, ascending by threshold.
  pub members: Vec<(u64, u64)>,
  /// The subscriber's reorder buffer in ms, negotiated for this set: how long
  /// the subscriber is willing to hold a group stream before the relay may
  /// discard it. `0` means no latency budget, and an algorithm that switches
  /// against a deadline has nothing to switch against.
  pub subscriber_buffer_ms: u64,
  /// How long the host has seen this set's consecutive groups arriving
  /// a fixed distance apart, in ms, or 0 until it has two to compare.
  pub group_span_ms: u64,
  /// The set's own negotiated knobs, handed over exactly as the subscriber
  /// sent them and in the order the algorithm defined. The host never reads
  /// them, which is what lets an algorithm add a knob without a change here:
  /// `params[0]` means whatever the algorithm with this id says it means.
  /// Shorter lists are allowed; a subscriber that offered nothing sends none.
  pub algorithm_params: Vec<u64>,
}

impl SetSnapshot {
  /// The lowest-quality member: the fallback when bandwidth is scarce.
  pub fn lowest_member(&self) -> Option<u64> {
    self.members.first().map(|(_, track)| *track)
  }

  /// The highest-quality member: what the set would like, budget permitting.
  pub fn top_member(&self) -> Option<u64> {
    self.members.last().map(|(_, track)| *track)
  }

  /// The highest member whose threshold `budget_kbps` covers, or `None` when
  /// even the lowest tier does not fit.
  pub fn member_up_to(&self, budget_kbps: u64) -> Option<u64> {
    self
      .members
      .iter()
      .rev()
      .find(|(threshold, _)| *threshold <= budget_kbps)
      .map(|(_, track)| *track)
  }

  /// The threshold of the highest member, i.e. what the set would need to
  /// serve its top track.
  pub fn top_threshold_kbps(&self) -> Option<u64> {
    self.members.last().map(|(threshold, _)| *threshold)
  }

  /// Whether `relay_track_id` is one of this set's members.
  pub fn has_track(&self, relay_track_id: u64) -> bool {
    self
      .members
      .iter()
      .any(|(_, track)| *track == relay_track_id)
  }
}

/// Everything an algorithm may see about one forwarding decision.
///
/// The signal list is the contract's surface: a new signal is added here, and
/// the host collects it. Algorithms must not reach around this struct for
/// connection, track or configuration state.
#[derive(Debug, Clone)]
pub struct AbrInput<'a> {
  /// Identifier of the connection this decision is for, for logging and
  /// correlating. It is not a map key: each connection runs its own instance
  /// (see [`registry::Registry::create`]), so algorithms keep state in `self`.
  pub client_id: u64,
  /// The media group being decided. Groups arrive in increasing order.
  pub group_id: u64,
  /// The transport's bandwidth estimate in kbps, or `0` when no estimate
  /// exists yet (a fresh connection). How to treat an unknown estimate is up
  /// to the algorithm: see [`AbrInput::budget_kbps`].
  pub bandwidth_estimate_kbps: u64,
  /// The operator's hard cap in kbps, or `0` when uncapped.
  pub configured_cap_kbps: u64,
  /// The sets assigned to this algorithm, in the order the host holds them.
  /// Empty means the connection currently has no switching sets.
  pub sets: &'a [SetSnapshot],
  /// Why the host is asking. A periodic sample of the same group is not a new
  /// group: an algorithm that reacts to the state inside a group takes its
  /// evidence differently for the two, and one that does not care can ignore
  /// this field entirely.
  pub reason: DecisionReason,
  /// A monotonic clock in milliseconds, on an epoch of the host's choosing.
  /// Only differences are meaningful, and an algorithm that needs to know how
  /// long something took takes the difference between two decisions. It is an
  /// input rather than a call to the clock so that a test can hand the
  /// algorithm the elapsed time it wants to exercise a timeout.
  pub now_ms: u64,
  /// The live forwarding streams of every set this connection has, one entry
  /// per stream. The count of a set's streams, the age of its oldest, and
  /// which groups are in flight are all read from here; see the helpers on
  /// [`AbrInput`]. Empty on a connection that has forwarded nothing yet.
  pub streams: &'a [StreamSnapshot],
  /// Streams reset for delivery timeout since this instance's previous
  /// decision. Congestion evidence: the transport dropped objects it had
  /// already committed to, which is how an algorithm learns it overran.
  pub stream_timeouts_since_last_decision: u64,
}

/// One live forwarding stream, as the host saw it open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamSnapshot {
  /// The set the stream carries, resolved by the host from its relay track.
  /// A stream whose track belongs to no set is not reported at all.
  pub set_id: u64,
  /// The member being forwarded.
  pub relay_track_id: u64,
  /// The media group being carried.
  pub group_id: u64,
  /// How long the stream has been open, in ms: `now_ms - when it opened`.
  /// A stream that finished and is only draining its last Object is still
  /// open in this sense, which is the point: it still occupies the pipeline.
  pub age_ms: u64,
  /// The stream was opened to re-send a group an earlier decision overtook,
  /// not to carry new media. Such a stream is the consequence of a downshift,
  /// so a depth that counted it would let the downshift report itself as new
  /// congestion and downshift again.
  pub resent: bool,
}

/// Why [`AbrAlgorithm::decide`] is being called.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecisionReason {
  /// A new media group started, and this is the decision for it.
  GroupBoundary,
  /// The host is re-evaluating while the same group is being served, because
  /// bandwidth moves faster than groups. `group_id` is the group in flight.
  PeriodicSample,
  /// A stream was reset for delivery timeout, which is evidence that the
  /// current choice is not sustainable, and the host wants a decision on the
  /// group in flight now rather than at the next boundary.
  StreamTimeout,
}

impl AbrInput<'_> {
  /// The bandwidth available to allocate: the transport estimate and the
  /// operator cap, whichever is stricter.
  ///
  /// A `0` estimate means "no estimate yet", not "no bandwidth", so it is
  /// ignored unless there is also no cap. With neither signal the result is
  /// [`u64::MAX`], meaning unconstrained, because a relay that believes it has
  /// no bandwidth would forward nothing at all. Algorithms that want to start
  /// conservatively on a cold connection should test
  /// `bandwidth_estimate_kbps == 0` themselves.
  pub fn budget_kbps(&self) -> u64 {
    match (self.bandwidth_estimate_kbps, self.configured_cap_kbps) {
      (0, 0) => u64::MAX,
      (0, cap) => cap,
      (est, 0) => est,
      (est, cap) => est.min(cap),
    }
  }

  /// Live forwarding streams on one set, including one still draining its
  /// last Object and any opened to re-send an overtaken group.
  pub fn open_streams(&self, set_id: u64) -> u64 {
    self
      .streams
      .iter()
      .filter(|stream| stream.set_id == set_id)
      .count() as u64
  }

  /// Live streams carrying `relay_track_id` on `set_id`: how much of the
  /// pipeline is occupied at one particular quality. A stream opened to
  /// re-send an overtaken group does not count, because that delivery is what
  /// a decision caused, not what the network is doing.
  pub fn open_streams_at_track(&self, set_id: u64, relay_track_id: u64) -> u64 {
    self
      .streams
      .iter()
      .filter(|stream| {
        stream.set_id == set_id && stream.relay_track_id == relay_track_id && !stream.resent
      })
      .count() as u64
  }

  /// Age in ms of the oldest live stream of a set, 0 when it has none: how
  /// far the set's delivery has fallen behind the moment it was asked for.
  pub fn oldest_stream_age_ms(&self, set_id: u64) -> u64 {
    self
      .streams
      .iter()
      .filter(|stream| stream.set_id == set_id)
      .map(|stream| stream.age_ms)
      .max()
      .unwrap_or(0)
  }

  /// The groups of a set that have a live stream: the deliveries the
  /// algorithm could decide to overtake.
  pub fn open_groups(&self, set_id: u64) -> Vec<u64> {
    let mut groups: Vec<u64> = self
      .streams
      .iter()
      .filter(|stream| stream.set_id == set_id)
      .map(|stream| stream.group_id)
      .collect();
    groups.sort_unstable();
    groups.dedup();
    groups
  }

  /// The observed span of a set's groups in ms, or 0 until the host has seen
  /// two consecutive groups go by. A healthy group's stream lives about one
  /// group span, so an age beyond the span is the part that piled up rather
  /// than the part that is simply in flight.
  pub fn group_span_ms(&self, set_id: u64) -> u64 {
    self
      .sets
      .iter()
      .find(|set| set.id == set_id)
      .map(|set| set.group_span_ms)
      .unwrap_or(0)
  }

  /// Live forwarding streams across the sets that have a subscriber: the
  /// connection's queue depth, which is what a stream-based congestion signal
  /// should measure. Streams on a set nobody is watching are not evidence of
  /// congestion on this connection's media.
  pub fn active_stream_depth(&self) -> u64 {
    self
      .sets
      .iter()
      .filter(|set| set.active)
      .map(|set| self.open_streams(set.id))
      .sum::<u64>()
  }
}

/// A bandwidth allocation algorithm: one instance per connection, owned by
/// that connection for its lifetime.
///
/// Instances are created by an [`AbrAlgorithmFactory`], so per-connection
/// state lives in fields instead of a process-wide map keyed by client id
/// (which would grow without bound). The trait is synchronous on purpose: an
/// allocation is a computation over [`AbrInput`], and keeping I/O out of it is
/// what makes the algorithms testable and the boundary hold.
pub trait AbrAlgorithm: Send {
  /// Decide which member of each set in `input.sets` to forward for
  /// `input.group_id`.
  ///
  /// Implementations must return an entry for every set they were given, must
  /// only name tracks that belong to the set they are returned for, and must
  /// not assume the sets arrive in any particular order. The host validates
  /// all of this and degrades violations visibly rather than trusting them.
  fn decide(&mut self, input: &AbrInput) -> Selection;
}

/// Creates the per-connection instances of one algorithm.
///
/// The factory is the process-wide half (registered once, shared across
/// connections, `Sync`); the instance is the per-connection half. Keeping them
/// apart is what allows an algorithm to hold mutable state without locking or
/// client-id keys.
pub trait AbrAlgorithmFactory: Send + Sync {
  /// The algorithm's wire id, as advertised in SETUP and named by a
  /// SWITCHING_SET_ASSIGNMENT.
  fn id(&self) -> u64;

  /// Name for logs and diagnostics.
  fn name(&self) -> &'static str;

  /// A fresh instance with no history, for one connection.
  fn create(&self) -> Box<dyn AbrAlgorithm>;
}

#[cfg(test)]
mod tests {
  use super::*;

  fn stream(set_id: u64, relay_track_id: u64, group_id: u64, age_ms: u64) -> StreamSnapshot {
    StreamSnapshot {
      set_id,
      relay_track_id,
      group_id,
      age_ms,
      resent: false,
    }
  }

  fn input_with<'a>(streams: &'a [StreamSnapshot], sets: &'a [SetSnapshot]) -> AbrInput<'a> {
    AbrInput {
      client_id: 1,
      group_id: 9,
      bandwidth_estimate_kbps: 0,
      configured_cap_kbps: 0,
      reason: DecisionReason::GroupBoundary,
      now_ms: 1000,
      sets,
      streams,
      stream_timeouts_since_last_decision: 0,
    }
  }

  fn set(id: u64, group_span_ms: u64) -> SetSnapshot {
    SetSnapshot {
      id,
      algorithm_id: 0,
      rank: 0,
      weight: 1,
      active: true,
      members: vec![(500, 10), (1000, 11)],
      subscriber_buffer_ms: 0,
      algorithm_params: vec![],
      group_span_ms,
    }
  }

  /// A re-sent stream still occupies the pipeline, so the raw depth counts it;
  /// only the per-quality question leaves it out.
  #[test]
  fn depth_counts_every_live_stream_and_quality_counts_only_new_delivery() {
    let resent = StreamSnapshot {
      resent: true,
      ..stream(1, 11, 7, 10)
    };
    let streams = [stream(1, 10, 8, 20), resent, stream(2, 20, 8, 5)];
    let sets = [set(1, 0), set(2, 0)];
    let input = input_with(&streams, &sets);

    assert_eq!(input.open_streams(1), 2);
    assert_eq!(input.open_streams(2), 1);
    assert_eq!(input.open_streams(3), 0, "an unknown set has no streams");

    assert_eq!(input.open_streams_at_track(1, 11), 0);
    assert_eq!(input.open_streams_at_track(1, 10), 1);
  }

  #[test]
  fn the_oldest_stream_and_the_open_groups_of_a_set() {
    let streams = [stream(1, 10, 8, 20), stream(1, 11, 9, 240)];
    let sets = [set(1, 0)];
    let input = input_with(&streams, &sets);

    assert_eq!(input.oldest_stream_age_ms(1), 240);
    assert_eq!(input.oldest_stream_age_ms(2), 0, "no streams, no age");
    assert_eq!(input.open_groups(1), vec![8, 9]);
  }

  #[test]
  fn depth_over_the_active_sets_only() {
    let streams = [
      stream(1, 10, 8, 0),
      stream(2, 20, 8, 0),
      stream(3, 30, 8, 0),
    ];
    let paused = SetSnapshot {
      active: false,
      ..set(2, 0)
    };
    let sets = [set(1, 0), paused];
    let input = input_with(&streams, &sets);

    assert_eq!(
      input.active_stream_depth(),
      1,
      "a set with no subscriber and a set nobody is watching are not congestion here"
    );
  }

  /// The cadence is a per-set measurement; a set the host has not measured
  /// reads as 0 rather than as some other set's number.
  #[test]
  fn each_set_carries_its_own_group_cadence() {
    let sets = [set(1, 1000), set(2, 0)];
    let input = input_with(&[], &sets);
    assert_eq!(input.group_span_ms(1), 1000);
    assert_eq!(input.group_span_ms(2), 0);
    assert_eq!(input.group_span_ms(77), 0);
  }
}
