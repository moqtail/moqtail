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
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};

use moqtail_ssts::SetSnapshot;
use switching_set::SwitchingSetManager;

/// SSTS per-group allocation decisions: group -> set -> selected relay
/// track id, or `None` when nothing from the set is forwarded.
pub type GroupDecisions = HashMap<u64, HashMap<u64, Option<u64>>>;

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
  /// what this relay actually runs (which is empty when the relay has the
  /// feature off, even if the client asked for it).
  pub fn new(client_setup: &Setup) -> Self {
    let (abr_tx, abr_rx) = tokio::sync::mpsc::channel(100);

    let config = crate::server::config::AppConfig::load();
    let negotiated_algorithms = advertised_algorithms(client_setup)
      .into_iter()
      .filter(|id| config.ssts_algorithms.contains(id))
      .collect();

    Self {
      negotiated_algorithms,
      switching_sets: Arc::new(RwLock::new(SwitchingSetManager::new())),
      abr_tx,
      abr_rx: Arc::new(Mutex::new(Some(abr_rx))),
      group_decisions: Arc::new(RwLock::new(HashMap::new())),
      decision_notify: Arc::new(tokio::sync::Notify::new()),
    }
  }

  /// SSTS was negotiated for this connection (see `negotiated_algorithms`).
  pub fn enabled(&self) -> bool {
    !self.negotiated_algorithms.is_empty()
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
