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
//! forward gate in `subscription.rs` reads, and the controller that runs the
//! bandwidth allocation. The algorithms themselves live outside this module
//! and are reached only through the `moqtail_ssts` contract — no algorithm
//! knows this module exists.

pub mod controller;
pub mod switching_set;

use moqtail::model::control::setup::Setup;
use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::{Mutex, RwLock};

use moqtail_ssts::SetSnapshot;
use moqtail_ssts::registry::registry;
use switching_set::SwitchingSetManager;

/// SSTS per-group allocation decisions: group -> set -> selected relay
/// track id, or `None` when nothing from the set is forwarded.
pub type GroupDecisions = HashMap<u64, HashMap<u64, Option<u64>>>;

/// Parse one algorithm id from the command line, accepting decimal or `0x`
/// hex: the private ids are easier to read in hex, and that is how they appear
/// in the logs and on the wire.
///
/// Which ids exist is this module's business rather than the configuration's,
/// so the check against the registry lives here. Rejecting an unknown id at
/// parse time means the relay reports it with the usage message and a non-zero
/// exit, the way any other bad option is reported, instead of starting up and
/// then failing every subscription that names it.
pub fn parse_algorithm_id(value: &str) -> Result<u64, String> {
  let trimmed = value.trim();
  let parsed = match trimmed.strip_prefix("0x").or(trimmed.strip_prefix("0X")) {
    Some(hex) => u64::from_str_radix(hex, 16),
    None => trimmed.parse::<u64>(),
  };
  let id = parsed.map_err(|_| format!("'{value}' is not an algorithm id"))?;
  if registry().get(id).is_some() {
    Ok(id)
  } else {
    Err(format!(
      "unknown SSTS algorithm id '{value}'; this relay runs {:?}",
      registry().ids()
    ))
  }
}

pub(crate) enum AbrMessage {
  /// An Object arrived for a group this connection has no decision for. The
  /// gate sends this for any group it cannot answer, not only for a new
  /// largest one: the first Object of a group is not necessarily the one that
  /// triggers the decision.
  NewGroup(u64),
  /// A forwarding stream was reset for delivery timeout: congestion evidence
  /// for the next allocation.
  StreamTimeout { group_id: u64 },
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
  /// Bumped when set membership changes.
  /// A deliberate supersede must bump it too when it lands.
  epoch: Arc<AtomicU64>,
}

/// The SSTS algorithms the client's SETUP advertises; an empty list (or the
/// absence of the option) prohibits SSTS.
fn advertised_algorithms(setup: &Setup) -> Vec<u64> {
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
    }
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
  /// set as a snapshot, plus how many forwarding streams are live on each.
  ///
  /// Both come from one read of the manager, so a set cannot appear between the
  /// two reads and show up without its counter. They are copies: no algorithm
  /// runs while a lock is held.
  pub async fn decision_snapshot(&self) -> (Vec<SetSnapshot>, HashMap<u64, u64>) {
    let manager = self.switching_sets.read().await;
    (
      manager.sets.values().map(SetSnapshot::from).collect(),
      manager.open_streams(),
    )
  }

  /// A forwarding stream started for this relay track. No-op when the track is
  /// not in a switching set, so non-SSTS traffic pays nothing.
  ///
  /// Every path that drops a stream from the send-stream map must call
  /// `on_stream_closed` for it as well, or the set's depth never comes back
  /// down and the algorithms read a queue that no longer exists.
  pub async fn on_stream_opened(&self, relay_track_id: u64) {
    self
      .switching_sets
      .write()
      .await
      .note_stream_opened(relay_track_id);
  }

  /// A forwarding stream of this relay track ended: finished, reset, or
  /// stopped by the peer.
  pub async fn on_stream_closed(&self, relay_track_id: u64) {
    self
      .switching_sets
      .write()
      .await
      .note_stream_closed(relay_track_id);
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
    // The client wanting it is not enough; that is what --ssts-enable is for.
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
  async fn decision_snapshot_covers_every_set_with_its_stream_count() {
    let client = setup_advertising(Some(&[0]));
    let state = SstsState::new(&client, &[0]);

    let (sets, streams) = state.decision_snapshot().await;
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
      .assign(track, 42, 1, 7, 0, 1000, 5, 1, 0)
      .unwrap();
    state.on_stream_opened(42).await;

    let (sets, streams) = state.decision_snapshot().await;
    assert_eq!(sets.len(), 1);
    assert_eq!(sets[0].id, 7);
    assert_eq!(sets[0].members, vec![(1000, 42)]);
    assert!(sets[0].active, "one member with activate = 1");
    assert_eq!(streams.get(&7), Some(&1));

    state.on_stream_closed(42).await;
    let (_, streams) = state.decision_snapshot().await;
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
}
