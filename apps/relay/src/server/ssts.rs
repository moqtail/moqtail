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

//! Sender-side track switching (SSTS): the relay-side mechanism.
//!
//! This is everything the relay owns: the algorithm list negotiated in
//! SETUP, the per-client switching sets, the per-group decision cache the
//! forward gate in `gate.rs` reads, and the controller that runs the
//! bandwidth allocation. The algorithms themselves live outside this module
//! and are reached only through the `moqtail_ssts` contract — no algorithm
//! knows this module exists.

pub mod controller;
pub mod gate;
pub mod switching_set;

use moqtail::model::control::setup::Setup;
use moqtail::model::data::full_track_name::FullTrackName;
use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::{Mutex, RwLock};

use crate::server::stream_id::StreamId;
use moqtail_ssts::registry::registry;
use moqtail_ssts::{SetSnapshot, StreamSnapshot};
use switching_set::SwitchingSetManager;

/// SSTS per-group allocation decisions: group -> set -> selected relay
/// track id, or `None` when nothing from the set is forwarded.
pub type GroupDecisions = HashMap<u64, HashMap<u64, Option<u64>>>;

/// Validates the configured algorithm ids against the registry.
///
/// Which ids exist is this module's business rather than the configuration's,
/// so the check against the registry lives here. Rejecting an unknown id at
/// startup means the relay exits with the error, the way any other bad option
/// is reported, instead of starting up and then failing every subscription
/// that names it.
pub fn check_algorithm_ids(ids: &[u64]) -> Result<(), validator::ValidationError> {
  match ids.iter().find(|id| registry().get(**id).is_none()) {
    None => Ok(()),
    Some(id) => Err(
      validator::ValidationError::new("unknown_ssts_algorithm").with_message(
        format!(
          "unknown SSTS algorithm id '{id}'; this relay runs {:?}",
          registry().ids()
        )
        .into(),
      ),
    ),
  }
}

pub(crate) enum AbrMessage {
  /// An Object arrived for a group this connection has no decision for. The
  /// gate sends this for any group it cannot answer, not only for a new
  /// largest one: the first Object of a group is not necessarily the one that
  /// triggers the decision.
  NewGroup(u64),
  /// An Object was handed to one of this connection's send streams: the
  /// delivery advanced, and the mid-group reading is taken again. The sample
  /// clock is the delivery itself — objects sent pace it, thinning as the
  /// path slows and stopping when it stops. The tick fires when the write
  /// starts, not when it completes, so a sample is never held up by the
  /// backpressure it exists to measure.
  ///
  /// The message names the group and track the Object went out on, because
  /// not every byte is progress: tail Objects draining on a group the gate
  /// has overtaken, and the replay of a re-sent group, are filtered by the
  /// controller against the newest group and the re-send marks.
  ObjectSent { group_id: u64, relay_track_id: u64 },
  /// A forwarding stream was reset for delivery timeout: congestion evidence
  /// for the next allocation.
  ///
  /// `set_id` is the set that lost the stream, or `None` when the track belongs
  /// to no set. The evidence is keyed by set because a timeout on one ladder is
  /// not evidence about another; a timeout with no set has no switching
  /// decision to inform and is dropped.
  StreamTimeout { set_id: Option<u64>, group_id: u64 },
}

/// All the SSTS state a connection carries, behind one handle.
#[derive(Debug, Clone)]
pub struct SstsState {
  /// The SSTS algorithms negotiated for this connection: the client's SETUP
  /// advertisement intersected with the list this relay runs. Empty means
  /// SSTS is unavailable, for any reason.
  pub negotiated_algorithms: Vec<u64>,
  pub switching_sets: Arc<RwLock<SwitchingSetManager>>,
  pub abr_tx: tokio::sync::mpsc::Sender<AbrMessage>,
  pub abr_rx: Arc<Mutex<Option<tokio::sync::mpsc::Receiver<AbrMessage>>>>,
  /// SSTS decisions (see `GroupDecisions`).
  pub group_decisions: Arc<RwLock<GroupDecisions>>,
  pub decision_notify: Arc<tokio::sync::Notify>,
  /// Bumped when set membership changes. A deliberate supersede must bump it
  /// too when it lands.
  epoch: Arc<AtomicU64>,
  /// The moment this connection's SSTS state was created, on the monotonic
  /// clock the algorithms are handed. Stream ages and the elapsed time between
  /// decisions are differences against it, so the epoch itself never means
  /// anything and a test can start one at zero.
  started_at: std::time::Instant,
  /// The same moment as a wall-clock timestamp in ms since the Unix epoch.
  /// `now_ms` is monotonic and means nothing outside this process; an
  /// algorithm handed an external schedule (the oracle's trace t0) needs the
  /// two joined, and this is the join.
  started_at_unix_ms: u64,
}

/// The SSTS algorithms the client's SETUP advertises; an empty list (or the
/// absence of the option) prohibits SSTS.
pub(crate) fn advertised_algorithms(setup: &Setup) -> Vec<u64> {
  use moqtail::model::parameter::setup_option::SetupOption;
  setup
    .setup_options
    .iter()
    .find_map(|kvp| match SetupOption::deserialize(kvp) {
      Ok(SetupOption::SstsAlgorithms { algorithms }) => Some(algorithms),
      _ => None,
    })
    .unwrap_or_default()
}

impl SstsState {
  /// SSTS for one connection: what the client advertised, intersected with
  /// what this relay runs (`relay_algorithms`, which is empty when the feature
  /// is off, even if the client asked for it).
  ///
  /// The relay's list is a parameter rather than a config read, because this
  /// intersection is the whole of the negotiation and has to be testable
  /// without a process-wide configuration.
  pub fn new(client_setup: &Setup, relay_algorithms: &[u64]) -> Self {
    let (abr_tx, abr_rx) = tokio::sync::mpsc::channel(100);

    let negotiated_algorithms = advertised_algorithms(client_setup)
      .into_iter()
      .filter(|id| relay_algorithms.contains(id))
      .collect();

    // The manager owns the counter and bumps it on every membership change,
    // so no caller has to remember to; this is the read handle for the gate.
    let switching_sets = SwitchingSetManager::new();
    let epoch = switching_sets.epoch_handle();

    Self {
      negotiated_algorithms,
      switching_sets: Arc::new(RwLock::new(switching_sets)),
      abr_tx,
      abr_rx: Arc::new(Mutex::new(Some(abr_rx))),
      group_decisions: Arc::new(RwLock::new(HashMap::new())),
      decision_notify: Arc::new(tokio::sync::Notify::new()),
      epoch,
      started_at: std::time::Instant::now(),
      started_at_unix_ms: std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0),
    }
  }

  /// This connection's monotonic clock in ms, which is the only clock an
  /// algorithm is allowed to know: it is passed in as a number, so a test can
  /// move time forward instead of waiting for it.
  pub fn now_ms(&self) -> u64 {
    self.started_at.elapsed().as_millis() as u64
  }

  /// This connection's monotonic clock in ms, expressed on the Unix epoch:
  /// `started_at_unix_ms + now_ms()`. An algorithm handed an external
  /// schedule in wall-clock terms (the oracle's trace t0) converts it to this
  /// process's monotonic clock with it.
  pub fn now_unix_ms(&self) -> u64 {
    self.started_at_unix_ms.saturating_add(self.now_ms())
  }

  /// SSTS was negotiated for this connection (see `negotiated_algorithms`).
  pub fn enabled(&self) -> bool {
    !self.negotiated_algorithms.is_empty()
  }

  /// The current membership epoch (see the field).
  pub fn epoch(&self) -> u64 {
    self.epoch.load(Ordering::Acquire)
  }

  /// Whether a decision has already been recorded for `group_id`.
  pub async fn has_group_decision(&self, group_id: u64) -> bool {
    self.group_decisions.read().await.contains_key(&group_id)
  }

  /// Invalidate the gate's cached verdicts. Set membership moves this by
  /// itself; this exists for the one other writer allowed past
  /// `record_group_decision`: a deliberate supersede, which moves a decided
  /// group's answer together with the reset and re-send choreography, and so
  /// must invalidate what the gate cached about it.
  pub fn bump_epoch(&self) {
    self.epoch.fetch_add(1, Ordering::Release);
  }

  /// Record the allocation for `group_id`, only if the group has
  /// no answer yet. A group's decision is final once recorded.
  pub async fn record_group_decision(
    &self,
    group_id: u64,
    decision: HashMap<u64, Option<u64>>,
    oldest_kept: u64,
  ) -> bool {
    let mut decisions = self.group_decisions.write().await;
    match decisions.entry(group_id) {
      Entry::Vacant(slot) => {
        slot.insert(decision);
        decisions.retain(|&group, _| group >= oldest_kept || group == group_id);
        true
      }
      Entry::Occupied(_) => false,
    }
  }

  /// The single validation a SWITCHING_SET_ASSIGNMENT gets, shared by both
  /// entry points (SUBSCRIBE and the PUBLISH_OK of a pushed track) so they
  /// cannot drift apart.
  ///
  /// `negotiated_algorithms` is already the intersection of what the client
  /// advertised and what this relay runs, so one containment test covers both
  /// sides.
  pub fn validate_assignment(&self, algorithm_id: u64) -> Result<(), String> {
    if self.negotiated_algorithms.is_empty() {
      return Err("SSTS was not negotiated in SETUP".to_string());
    }
    if !self.negotiated_algorithms.contains(&algorithm_id) {
      return Err(format!(
        "SSTS algorithm {algorithm_id} was not negotiated for this connection"
      ));
    }
    Ok(())
  }

  /// The algorithms' view of this connection at decision time: every switching
  /// set as a snapshot, plus the live forwarding streams of the whole
  /// connection.
  ///
  /// Both come from one read of the manager, so a set cannot appear between the
  /// two reads and show up without its streams. They are copies: no algorithm
  /// runs while a lock is held.
  pub async fn decision_snapshot(&self, now_ms: u64) -> (Vec<SetSnapshot>, Vec<StreamSnapshot>) {
    let manager = self.switching_sets.read().await;
    let sets: Vec<SetSnapshot> = manager.sets.values().map(SetSnapshot::from).collect();
    (sets, manager.stream_table(now_ms))
  }

  /// A forwarding stream started for this relay track. No-op when the track is
  /// not in a switching set, so non-SSTS traffic pays nothing.
  ///
  /// Every path that drops a stream from the send-stream map must call
  /// `on_stream_closed` for it as well, or the set's depth never comes back
  /// down and the algorithms read a queue that no longer exists.
  pub async fn on_stream_opened(&self, stream_id: &StreamId) {
    let now_ms = self.now_ms();
    self
      .switching_sets
      .write()
      .await
      .note_stream_opened(stream_id, now_ms);
  }

  /// A forwarding stream of this relay track ended: finished, reset, or
  /// stopped by the peer.
  pub async fn on_stream_closed(&self, stream_id: &StreamId) {
    self
      .switching_sets
      .write()
      .await
      .note_stream_closed(stream_id);
  }

  /// How long a forwarding stream of this relay track may linger after its
  /// group ended, when the subscriber negotiated a buffer for its set.
  ///
  /// `None` means the track is not in a switching set, its set runs the
  /// default algorithm, or no buffer was negotiated, and the caller's
  /// configured timeout applies. Negotiating a buffer is how a subscriber says
  /// it has somewhere to put late media; without that the operator's setting
  /// stays the answer, so turning the feature on cannot silently start
  /// discarding data for a subscriber that never asked for switching.
  /// The live streams of `set_id` that carry a member other than `keep`, i.e.
  /// the deliveries an algorithm overtook when it selected `keep`. Group id
  /// first, ascending.
  pub(crate) async fn overtaken_streams(
    &self,
    set_id: u64,
    keep_track: u64,
  ) -> Vec<(u64, StreamId)> {
    let sets = self.switching_sets.read().await;
    sets.streams_above(set_id, keep_track)
  }

  /// Record that a group's delivery was overtaken, so the streams opened for it
  /// from now on are reported to the algorithms as re-sends rather than as new
  /// congestion, and carry the deadline the decision asked the re-send to meet.
  pub(crate) async fn mark_group_resent(
    &self,
    set_id: u64,
    group_id: u64,
    deadline_ms: Option<u64>,
  ) {
    self
      .switching_sets
      .write()
      .await
      .mark_group_resent(set_id, group_id, deadline_ms);
  }

  /// Whether this track's copy of `group_id` is a re-send of an overtaken
  /// delivery, which is what entitles it to have its cached head replayed.
  pub(crate) async fn is_resent_group(&self, relay_track_id: u64, group_id: u64) -> bool {
    let sets = self.switching_sets.read().await;
    match sets.set_of_relay_track(relay_track_id) {
      Some(set) => sets.is_resent_group(set.id, group_id),
      None => false,
    }
  }

  /// The full track name carrying this relay track, when it is in a switching
  /// set. Used to find the subscription that will own a replacement stream.
  pub(crate) async fn full_track_name_of_relay_track(
    &self,
    relay_track_id: u64,
  ) -> Option<FullTrackName> {
    self
      .switching_sets
      .read()
      .await
      .full_track_name_of_relay_track(relay_track_id)
  }

  /// The switching set carrying this relay track, if any.
  pub(crate) async fn set_id_of_relay_track(&self, relay_track_id: u64) -> Option<u64> {
    let sets = self.switching_sets.read().await;
    let set = sets.set_of_relay_track(relay_track_id)?;
    Some(set.id)
  }

  /// How long closing one forwarding stream may wait for its FIN before the
  /// stream is reset. `None` closes gracefully.
  ///
  /// The timer is a property of the switching set: no set means no timer, and
  /// a set on the default algorithm has none either. A re-send deadline
  /// replaces the set's buffer for the group it was ordered for; zero means
  /// never discard there too. Nothing here reads a connection-wide setting.
  pub(crate) async fn stream_close_timeout(
    &self,
    relay_track_id: u64,
    group_id: u64,
  ) -> Option<std::time::Duration> {
    let sets = self.switching_sets.read().await;
    let set = sets.set_of_relay_track(relay_track_id)?;
    if let Some(ms) = sets.resent_deadline_ms(set.id, group_id) {
      return (ms > 0).then(|| std::time::Duration::from_millis(ms));
    }
    set.discard_timeout()
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use moqtail::model::parameter::setup_option::SetupOption;

  fn setup_advertising(algorithms: Option<&[u64]>) -> Setup {
    let options: Vec<moqtail::model::common::pair::KeyValuePair> = match algorithms {
      Some(ids) => vec![
        SetupOption::new_ssts_algorithms(ids.to_vec())
          .try_into()
          .unwrap(),
      ],
      None => vec![],
    };
    Setup::new(options)
  }

  #[test]
  fn negotiation_keeps_only_what_both_sides_offer() {
    let private = moqtail_ssts::registry::PRIVATE_ALGORITHM_ID_BASE;
    let client = setup_advertising(Some(&[0, private]));
    let state = SstsState::new(&client, &[0]);
    assert_eq!(state.negotiated_algorithms, vec![0]);
    assert!(state.enabled());

    // The other way round: a client that only offers the default, on a relay
    // that also runs the experimental one, gets the default.
    let client = setup_advertising(Some(&[0]));
    let state = SstsState::new(&client, &[0, private]);
    assert_eq!(state.negotiated_algorithms, vec![0]);
  }

  #[test]
  fn a_client_that_offers_nothing_negotiates_nothing() {
    // Both the empty list and the absent option mean the same thing to the
    // protocol, and both have to leave SSTS off.
    for setup in [setup_advertising(Some(&[])), setup_advertising(None)] {
      let state = SstsState::new(&setup, &[0]);
      assert!(state.negotiated_algorithms.is_empty());
      assert!(!state.enabled());
      assert!(state.validate_assignment(0).is_err());
    }
  }

  #[test]
  fn a_relay_with_the_feature_off_negotiates_nothing() {
    // The client wanting it is not enough; that is what `ssts_enable` is for.
    let client = setup_advertising(Some(&[0]));
    let state = SstsState::new(&client, &[]);
    assert!(state.negotiated_algorithms.is_empty());
    assert!(!state.enabled());
  }

  #[test]
  fn validate_assignment_follows_the_negotiated_list() {
    let private = moqtail_ssts::registry::PRIVATE_ALGORITHM_ID_BASE;
    let client = setup_advertising(Some(&[0, private]));
    let state = SstsState::new(&client, &[0]);

    state.validate_assignment(0).unwrap();
    // Advertised by the client but not run here: the relay must not take a
    // switching set assignment for an algorithm it cannot evaluate.
    let error = state.validate_assignment(private).unwrap_err();
    assert!(error.contains("not negotiated"), "got {error}");
  }

  #[tokio::test]
  async fn decision_snapshot_covers_every_set_with_its_streams() {
    let client = setup_advertising(Some(&[0]));
    let state = SstsState::new(&client, &[0]);

    let (sets, streams) = state.decision_snapshot(state.now_ms()).await;
    assert!(sets.is_empty() && streams.is_empty());

    let track = moqtail::model::data::full_track_name::FullTrackName::new(
      moqtail::model::common::tuple::Tuple::from_utf8_path("ns"),
      moqtail::model::common::tuple::TupleField::from_utf8("t"),
    )
    .unwrap();
    state
      .switching_sets
      .write()
      .await
      .assign(track, 42, 1, 7, 0, 1000, 5, 1, 0, 0, vec![])
      .unwrap();
    let stream = crate::server::stream_id::StreamId::new_subgroup(42, 3, Some(0));
    state.on_stream_opened(&stream).await;

    let (sets, streams) = state.decision_snapshot(state.now_ms()).await;
    assert_eq!(sets.len(), 1);
    assert_eq!(sets[0].id, 7);
    assert_eq!(sets[0].members, vec![(1000, 42)]);
    assert!(sets[0].active, "one member with activate = 1");
    assert_eq!(streams.len(), 1);
    assert_eq!(streams[0].set_id, 7);
    assert_eq!(streams[0].group_id, 3);
    assert!(!streams[0].resent, "an ordinary delivery is not a re-send");

    state.on_stream_closed(&stream).await;
    let (_, streams) = state.decision_snapshot(state.now_ms()).await;
    assert!(streams.is_empty(), "a drained set reports no streams");
  }

  fn allocation(track: u64) -> HashMap<u64, Option<u64>> {
    HashMap::from([(7, Some(track))])
  }

  #[tokio::test]
  async fn a_group_decided_once_ignores_later_answers() {
    let state = SstsState::new(&setup_advertising(Some(&[0])), &[0]);
    assert!(state.record_group_decision(7, allocation(12), 0).await);
    // A re-evaluation of the same group with a different answer cannot move
    // it: applying it mid-delivery would splice the group across renditions.
    assert!(!state.record_group_decision(7, allocation(10), 0).await);
    assert_eq!(
      state.group_decisions.read().await.get(&7),
      Some(&allocation(12)),
      "the recorded answer survives a changed re-evaluation"
    );
    assert!(state.has_group_decision(7).await);
  }

  #[tokio::test]
  async fn decisions_prune_behind_the_live_edge_and_a_pruned_group_decides_again() {
    let state = SstsState::new(&setup_advertising(Some(&[0])), &[0]);
    assert!(state.record_group_decision(10, allocation(11), 0).await);
    // The live edge moves to 14, keeping back to 12: group 10 falls out.
    assert!(state.record_group_decision(14, allocation(11), 12).await);
    {
      let recorded = state.group_decisions.read().await;
      assert!(!recorded.contains_key(&10), "pruned behind the live edge");
      assert!(recorded.contains_key(&14));
    }

    assert!(!state.has_group_decision(10).await);
    assert!(state.record_group_decision(10, allocation(9), 12).await);
    assert!(state.has_group_decision(10).await);

    // It is pruned by the next decision, not by its own.
    assert!(state.record_group_decision(20, allocation(9), 18).await);
    assert!(!state.has_group_decision(10).await);
  }

  #[tokio::test]
  async fn stream_close_timeout_comes_only_from_the_set() {
    let private = moqtail_ssts::registry::PRIVATE_ALGORITHM_ID_BASE;
    let state = SstsState::new(&setup_advertising(Some(&[private])), &[private]);
    let track = moqtail::model::data::full_track_name::FullTrackName::new(
      moqtail::model::common::tuple::Tuple::from_utf8_path("ns"),
      moqtail::model::common::tuple::TupleField::from_utf8("t"),
    )
    .unwrap();
    state
      .switching_sets
      .write()
      .await
      .assign(track, 42, 1, 7, private, 1000, 5, 1, 0, 250, vec![])
      .unwrap();

    assert_eq!(
      state.stream_close_timeout(42, 3).await,
      Some(std::time::Duration::from_millis(250))
    );
    assert_eq!(
      state.stream_close_timeout(99, 3).await,
      None,
      "a track outside every set has no timer"
    );

    // A decision may put its own deadline on one group, and zero means never.
    {
      let mut sets = state.switching_sets.write().await;
      sets.mark_group_resent(7, 4, Some(0));
      sets.mark_group_resent(7, 5, Some(100));
    }
    assert_eq!(state.stream_close_timeout(42, 4).await, None);
    assert_eq!(
      state.stream_close_timeout(42, 5).await,
      Some(std::time::Duration::from_millis(100))
    );
  }
}
