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
use std::sync::{
  Arc,
  atomic::{AtomicU64, Ordering},
};
use tokio::sync::{Mutex, RwLock};

use switching_set::SwitchingSetManager;

/// SSTS per-group allocation decisions: group -> set -> selected relay
/// track id, or `None` when nothing from the set is forwarded.
pub type GroupDecisions = HashMap<u64, HashMap<u64, Option<u64>>>;

pub(crate) enum AbrMessage {
  /// An Object arrived on a group larger than the previously largest group:
  /// run the bandwidth allocation for that group.
  NewGroup(u64),
  /// A forwarding stream timed out on close and was reset.
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
  /// Open forwarding streams per switching set (set id -> counter),
  /// maintained by the open/close stream paths.
  pub open_streams_per_set: Arc<RwLock<HashMap<u64, Arc<AtomicU64>>>>,
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
      open_streams_per_set: Arc::new(RwLock::new(HashMap::new())),
    }
  }

  /// SSTS was negotiated for this connection (see `negotiated_algorithms`).
  pub fn enabled(&self) -> bool {
    !self.negotiated_algorithms.is_empty()
  }

  /// The single validation a SWITCHING_SET_ASSIGNMENT gets, shared by both
  /// entry points (SUBSCRIBE and the PUBLISH_OK of a pushed track) so they
  /// cannot drift apart. SSTS is unusable unless the client negotiated it in
  /// SETUP, and only with an algorithm both sides actually run.
  pub fn validate_assignment(&self, algorithm_id: u64) -> Result<(), String> {
    if self.negotiated_algorithms.is_empty() {
      return Err("SSTS was not negotiated in SETUP".to_string());
    }
    if !super::abr::SUPPORTED_SSTS_ALGORITHMS.contains(&algorithm_id)
      || !self.negotiated_algorithms.contains(&algorithm_id)
    {
      return Err(format!("unsupported SSTS algorithm {algorithm_id}"));
    }
    Ok(())
  }

  /// Increment the open-stream counter of the switching set this
  /// relay_track_id belongs to. No-op if the track is not in any set.
  pub async fn on_stream_opened(&self, relay_track_id: u64) {
    let set_id = {
      let sets = self.switching_sets.read().await;
      sets.get_set_id_for_relay_track(relay_track_id)
    };
    if let Some(set_id) = set_id {
      let counter = {
        let mut map = self.open_streams_per_set.write().await;
        map
          .entry(set_id)
          .or_insert_with(|| Arc::new(AtomicU64::new(0)))
          .clone()
      };
      counter.fetch_add(1, Ordering::SeqCst);
    }
  }

  /// Decrement the open-stream counter of the switching set this
  /// relay_track_id belongs to (no-op if unknown or already zero).
  pub async fn on_stream_closed(&self, relay_track_id: u64) {
    let set_id = {
      let sets = self.switching_sets.read().await;
      sets.get_set_id_for_relay_track(relay_track_id)
    };
    if let Some(set_id) = set_id
      && let Some(counter) = { self.open_streams_per_set.read().await.get(&set_id).cloned() }
    {
      // Atomic check-and-subtract to prevent underflow from concurrent
      // decrements.
      let _ = counter.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |x| {
        if x > 0 { Some(x - 1) } else { None }
      });
    }
  }
}
