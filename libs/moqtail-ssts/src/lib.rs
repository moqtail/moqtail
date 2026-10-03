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
  /// Live forwarding streams per switching set, including streams still
  /// draining their last Object. A set the host has no counter for is absent
  /// and reads as zero; see [`AbrInput::open_streams`].
  pub open_streams_per_set: &'a HashMap<u64, u64>,
  /// Streams reset for delivery timeout since this instance's previous
  /// decision. Congestion evidence: the transport dropped objects it had
  /// already committed to, which is how an algorithm learns it overran.
  pub stream_timeouts_since_last_decision: u64,
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

  /// Live forwarding streams on one set.
  pub fn open_streams(&self, set_id: u64) -> u64 {
    self.open_streams_per_set.get(&set_id).copied().unwrap_or(0)
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
