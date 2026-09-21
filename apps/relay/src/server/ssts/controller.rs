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

//! The per-connection decision loop: collect the input, run the algorithms
//! this connection negotiated, validate what came back, publish it for the
//! forward gate.
//!
//! This is the mechanism side of SSTS. The decisions themselves live in
//! `moqtail-ssts`; what happens here is the translation between the two: the
//! relay's state becomes an [`AbrInput`], and an algorithm's [`Selection`]
//! becomes a per-group decision the gate in `subscription.rs` can act on.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use moqtail_ssts::registry::registry;
use moqtail_ssts::{
  AbrAlgorithm, AbrInput, DecisionReason, Selection, SetSnapshot, StreamSnapshot, Tuning,
};
use tokio::select;
use tracing::warn;

use moqtail::model::error::StreamResetCode;
use moqtail_ssts::registry::registry;
use moqtail_ssts::{
  AbrAlgorithm, AbrInput, DecisionReason, Selection, SetSnapshot, StreamSnapshot, Tuning,
};
use tokio::select;
use tracing::warn;

use crate::server::config::AppConfig;
use crate::server::session::client::MOQTClient;

use super::AbrMessage;

/// How often the decision is re-evaluated without a new group. The answer for
/// the group in flight cannot change.
const DECISION_INTERVAL: Duration = Duration::from_millis(100);

/// Number of groups of decisions to keep behind the live edge, so a publisher
/// running behind still gets a decision instead of forwarding unselected.
const DECISION_WINDOW: u64 = 5;

/// An algorithm this connection runs, with the identity it was created from so
/// that a bad decision can be attributed in the log.
struct Running {
  id: u64,
  name: &'static str,
  algorithm: Box<dyn AbrAlgorithm>,
}

/// Spawn the SSTS decision loop for a connection that has switching sets.
pub(crate) fn start(client: Arc<MOQTClient>) {
  tokio::spawn(async move {
    // Claiming the receiver is what makes starting twice a no-op: the second
    // task finds nothing to read from and leaves.
    let Some(mut rx) = client.ssts.abr_rx.lock().await.take() else {
      return;
    };

    let client_id = client.connection_id as u64;
    // The host's tuning, read once per connection: an algorithm is built with
    // the values it will keep for its lifetime, and never goes looking for a
    // value of its own while it decides.
    let tuning = Tuning::from_pairs(&AppConfig::load().ssts_algorithm_params);
    let mut running: Vec<Running> = Vec::new();
    for id in &client.ssts.negotiated_algorithms {
      match registry().get(*id) {
        Some(factory) => running.push(Running {
          id: *id,
          name: factory.name(),
          algorithm: factory.create(&tuning),
        }),
        None => warn!(
          client_id,
          algorithm_id = id,
          "SSTS: negotiated algorithm is not registered; its sets will forward nothing"
        ),
      }
    }
    if running.is_empty() {
      return;
    }

    let mut ticker = tokio::time::interval(DECISION_INTERVAL);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    // The group the periodic re-evaluation runs for.
    let mut last_group: Option<u64> = None;
    // The largest group ever seen. Decisions are pruned against this rather
    // than against the group being decided, so an Object that arrives late on
    // an old group cannot prune the decisions of the live edge.
    let mut largest_group: u64 = 0;
    // Streams reset for delivery timeout since the previous decision, keyed by
    // the set that lost them: the evidence belongs to the set whose delivery
    // overran, and an algorithm that holds one tier for the connection adds the
    // numbers up itself. Read only by this connection's algorithms, so it lives
    // here instead of in shared state.
    let mut stream_timeouts: HashMap<u64, u64> = HashMap::new();

    loop {
      select! {
        msg = rx.recv() => {
          match msg {
            Some(AbrMessage::NewGroup(group_id)) => {
              last_group = Some(group_id.max(last_group.unwrap_or(0)));
              largest_group = largest_group.max(group_id);
              // One boundary per group.
              if client.ssts.has_group_decision(group_id).await {
                // The answer already exists; wake anyone waiting on it
                client.ssts.decision_notify.notify_waiters();
                continue;
              }
              decide(
                &mut running,
                &client,
                group_id,
                largest_group,
                &mut stream_timeouts,
                DecisionReason::GroupBoundary,
              )
              .await;
            }
            Some(AbrMessage::StreamTimeout { set_id, group_id }) => {
              largest_group = largest_group.max(group_id);
              if let Some(set_id) = set_id {
                *stream_timeouts.entry(set_id).or_default() += 1;
              }
              if let Some(group_id) = last_group {
                decide(
                  &mut running,
                  &client,
                  group_id,
                  largest_group,
                  &mut stream_timeouts,
                  DecisionReason::StreamTimeout,
                )
                .await;
              }
            }
            None => return,
          }
        }
        // The group in flight is re-evaluated on a clock as well, because the
        // bandwidth changes between groups. An algorithm that only reads the
        // evidence at a boundary can ignore the difference; one that watches
        // the state inside a group takes these as its samples.
        _ = ticker.tick() => {
          if let Some(group_id) = last_group {
            decide(
              &mut running,
              &client,
              group_id,
              largest_group,
              &mut stream_timeouts,
              DecisionReason::PeriodicSample,
            )
            .await;
          }
        }
        _ = client.connection.closed() => {
          return;
        }
      }
    }
  });
}

/// Run every algorithm over the sets assigned to it, for `group_id`.
async fn decide(
  running: &mut [Running],
  client: &Arc<MOQTClient>,
  group_id: u64,
  largest_group: u64,
  stream_timeouts: &mut HashMap<u64, u64>,
  reason: DecisionReason,
) {
  let now_ms = client.ssts.now_ms();
  let (sets, streams): (Vec<SetSnapshot>, Vec<StreamSnapshot>) =
    client.ssts.decision_snapshot(now_ms).await;
  if sets.is_empty() {
    return;
  }

  let bandwidth_estimate_kbps = client.connection.bandwidth_estimate_kbps();
  let configured_cap_kbps = AppConfig::load().write_kbps_limit;
  // The estimate every algorithm is being handed, on one line, whether or not
  // the algorithm reads it: an algorithm that switches on stream depth says
  // nothing about the estimate, and the first question asked of a decision is
  // still what the transport thought the path was worth at that moment.
  tracing::debug!(
    client_id = client.connection_id,
    group_id,
    ?reason,
    bandwidth_estimate_kbps,
    configured_cap_kbps,
    "SSTS: transport estimate for this decision"
  );
  // Taken once and handed to every algorithm: they all decide at this moment,
  // so they all see the same evidence.
  let timeouts = std::mem::take(stream_timeouts);

  let mut decisions = Selection::with_capacity(sets.len());
  for algorithm in running.iter_mut() {
    let algorithm_sets: Vec<SetSnapshot> = sets
      .iter()
      .filter(|set| set.algorithm_id == algorithm.id)
      .cloned()
      .collect();
    if algorithm_sets.is_empty() {
      continue;
    }

    let input = AbrInput {
      client_id: client.connection_id as u64,
      group_id,
      bandwidth_estimate_kbps,
      configured_cap_kbps,
      reason,
      now_ms,
      sets: &algorithm_sets,
      streams: &streams,
      stream_timeouts_since_last_decision: &timeouts,
    };
    merge(
      &mut decisions,
      validate(
        algorithm.name,
        algorithm.algorithm.decide(&input),
        &algorithm_sets,
      ),
    );
  }

  // A set no running algorithm owns gets an explicit "forward nothing".
  // Leaving it out of the map would stall the gate on that group instead.
  let unowned: Vec<&SetSnapshot> = sets
    .iter()
    .filter(|set| !decisions.contains_key(set.id))
    .collect();
  for set in unowned {
    warn!(
      algorithm_id = set.algorithm_id,
      set_id = set.id,
      "SSTS: no running algorithm owns this set; forwarding nothing"
    );
    decisions.insert(set.id, None);
  }

  // A group already being delivered keeps its answer: recording is
  // insert-if-absent. A changed answer for the group in flight is spent here;
  // the algorithms have still run, and an algorithm that means to move the
  // in-flight groups of a set says so through `Selection::supersede`.
  //
  // Resolve those requests before the selection moves into the record. They
  // belong to this cycle only; the per-group map records what to forward,
  // never what was overtaken.
  let overtakes: Vec<(u64, u64, Option<u64>)> = decisions
    .supersede
    .iter()
    .filter_map(|(set_id, request)| {
      decisions
        .tracks
        .get(set_id)
        .and_then(|track| *track)
        .map(|track_id| (*set_id, track_id, request.resend_deadline_ms))
    })
    .collect();

  let recorded = client
    .ssts
    .record_group_decision(
      group_id,
      decisions.tracks,
      largest_group.saturating_sub(DECISION_WINDOW),
    )
    .await;

  if recorded {
    // No epoch bump: answers are final, so recording one makes no cached
    // verdict stale; waiters wake on the notify.
    client.ssts.decision_notify.notify_waiters();
  }

  for (set_id, track_id, resend_deadline_ms) in overtakes {
    supersede_overtaken(client, set_id, track_id, resend_deadline_ms).await;
  }
}

/// Deal with the groups a decision overtook.
///
/// Each one gets three changes at once: its decision is flipped to the member
/// the algorithm just chose, so the objects of that group that have not arrived
/// yet come from the new member; its remaining old-member streams are reset, so
/// the subscriber stops receiving a delivery the relay has already abandoned;
/// and the group is marked as re-sent, so the streams the subscriber now causes
/// to be opened are reported to the algorithms as a consequence of the decision
/// rather than as new congestion.
///
/// The relay's own abandoned streams are deliberately not fed to the timeout
/// evidence. They are the algorithm's doing, not the transport's, and reporting
/// them would let every downshift manufacture the evidence for the next one.
async fn supersede_overtaken(
  client: &Arc<MOQTClient>,
  set_id: u64,
  track_id: u64,
  resend_deadline_ms: Option<u64>,
) {
  let overtaken = client.ssts.overtaken_streams(set_id, track_id).await;
  if overtaken.is_empty() {
    return;
  }

  let mut flipped = 0usize;
  {
    let mut group_decisions = client.ssts.group_decisions.write().await;
    for (group_id, _) in &overtaken {
      if let Some(decision) = group_decisions.get_mut(group_id) {
        decision.insert(set_id, Some(track_id));
        flipped += 1;
      }
    }
  }

  let mut groups: Vec<u64> = overtaken.iter().map(|(group_id, _)| *group_id).collect();
  groups.sort_unstable();
  groups.dedup();
  for group_id in &groups {
    client
      .ssts
      .mark_group_resent(set_id, *group_id, resend_deadline_ms)
      .await;
  }

  for (_, stream_id) in &overtaken {
    // The delivery was cancelled by a decision, not by a failure or a timeout,
    // and the code on the wire should say so.
    client
      .reset_stream(stream_id, StreamResetCode::Cancelled.to_u64())
      .await;
  }

  tracing::debug!(
    client_id = client.connection_id,
    set_id,
    relay_track_id = track_id,
    groups = groups.len(),
    streams = overtaken.len(),
    decisions_flipped = flipped,
    "SSTS: overtook the groups in flight for this set"
  );

  if flipped > 0 {
    // The one writer allowed past `record_group_decision`: a flipped group
    // must also move the epoch, or gates keep forwarding the old member from
    // cache and the re-send never replaces the delivery it means to.
    client.ssts.bump_epoch();
    client.ssts.decision_notify.notify_waiters();
  }
}

/// Fold one algorithm's validated answer into the connection's decision.
fn merge(into: &mut Selection, from: Selection) {
  for (set_id, track) in from.tracks {
    into.insert(set_id, track);
  }
  for (set_id, request) in from.supersede {
    into.supersede.insert(set_id, request);
  }
}

/// Check an algorithm's answer against the sets it was shown, so a buggy or
/// hostile algorithm degrades visibly instead of silently blackholing a track.
///
/// Every set in `sets` gets an entry in the result: the gate waits for a
/// decision per set, so an omitted one would block that group forever.
fn validate(algorithm: &str, selection: Selection, sets: &[SetSnapshot]) -> Selection {
  let mut validated = Selection::with_capacity(sets.len());

  for set in sets {
    match selection.get(set.id) {
      Some(Some(track_id)) if set.has_track(*track_id) => {
        validated.insert(set.id, Some(*track_id));
      }
      Some(Some(track_id)) => {
        warn!(
          algorithm,
          set_id = set.id,
          relay_track_id = track_id,
          "SSTS: algorithm selected a track that is not a member of the set; forwarding nothing"
        );
        validated.insert(set.id, None);
      }
      Some(None) => {
        validated.insert(set.id, None);
      }
      None => {
        warn!(
          algorithm,
          set_id = set.id,
          "SSTS: algorithm returned no decision for a set; forwarding nothing"
        );
        validated.insert(set.id, None);
      }
    }
  }

  let invented: Vec<u64> = selection
    .tracks
    .keys()
    .filter(|set_id| !sets.iter().any(|set| set.id == **set_id))
    .copied()
    .collect();
  if !invented.is_empty() {
    warn!(
      algorithm,
      ?invented,
      "SSTS: algorithm returned sets it was not given; ignored"
    );
  }

  // A request to overtake a set is only meaningful for a set the algorithm was
  // shown and gave a real answer to: overtaking while forwarding nothing would
  // cancel a delivery in order to replace it with no delivery.
  for (set_id, request) in &selection.supersede {
    let owned = sets.iter().any(|set| set.id == *set_id);
    let answered = matches!(selection.get(*set_id), Some(Some(_)));
    if owned && answered {
      validated.supersede.insert(*set_id, *request);
    } else if owned {
      warn!(
        algorithm,
        set_id, "SSTS: algorithm asked to overtake a set it forwarded nothing for; ignored"
      );
    }
  }

  validated
}

#[cfg(test)]
mod tests {
  use super::merge;
  use super::validate;
  use moqtail_ssts::Selection;
  use moqtail_ssts::SetSnapshot;

  fn set(id: u64, tracks: &[u64]) -> SetSnapshot {
    SetSnapshot {
      id,
      algorithm_id: 0,
      rank: 0,
      weight: 1,
      active: true,
      members: tracks.iter().map(|track| (500 + *track, *track)).collect(),
      subscriber_buffer_ms: 0,
      algorithm_params: vec![],
      group_span_ms: 0,
    }
  }

  /// The gate waits for a decision per set, so an algorithm that answers some
  /// sets and not others must not be able to stall a group.
  #[test]
  fn every_set_gets_an_answer() {
    let sets = [set(1, &[10, 11]), set(2, &[20])];
    let mut answered = Selection::new();
    answered.insert(1, 11u64);

    let validated = validate("test", answered, &sets);
    assert_eq!(validated.tracks[&1], Some(11));
    assert_eq!(
      validated.tracks[&2], None,
      "an unanswered set forwards nothing rather than blocking"
    );
  }

  #[test]
  fn a_track_from_the_wrong_set_is_not_forwarded() {
    let sets = [set(1, &[10, 11]), set(2, &[20])];
    let mut answered = Selection::new();
    answered.insert(1, 20u64);
    answered.insert(2, None);

    let validated = validate("test", answered, &sets);
    assert_eq!(
      validated.tracks[&1], None,
      "a track that is not a member cannot be forwarded"
    );
  }

  #[test]
  fn sets_the_algorithm_was_never_shown_are_dropped() {
    let sets = [set(1, &[10])];
    let mut answered = Selection::new();
    answered.insert(1, 10u64);
    answered.insert(99, 77u64);

    let validated = validate("test", answered, &sets);
    assert!(!validated.tracks.contains_key(&99));
    assert!(!validated.supersede.contains_key(&99));
  }

  /// Overtaking a delivery is the one thing in a decision that destroys work in
  /// progress, so it is honoured only where the algorithm has the standing for
  /// it: a set it was shown, and one it is actually forwarding something.
  #[test]
  fn a_supersede_request_needs_a_set_and_a_track() {
    let sets = [set(1, &[10, 11]), set(2, &[20, 21])];

    let mut answered = Selection::new();
    answered.insert(1, 10u64);
    answered.supersede(1);
    answered.insert(2, None);
    answered.supersede(2);
    answered.supersede(99);

    let validated = validate("test", answered, &sets);
    assert!(validated.supersede.contains_key(&1));
    assert!(
      !validated.supersede.contains_key(&2),
      "overtaking in order to forward nothing is not a switching decision"
    );
    assert!(
      !validated.supersede.contains_key(&99),
      "a set the algorithm never saw cannot be overtaken"
    );
  }

  /// The deadline is the part of the request the algorithm owns, so validation
  /// carries it through untouched for the requests it honours, and drops it with
  /// the ones it does not. Two algorithms answering for disjoint sets each bring
  /// their own, and folding their answers together keeps both.
  #[test]
  fn a_deadline_travels_with_the_request_that_carries_it() {
    let sets = [set(1, &[10, 11]), set(2, &[20, 21])];

    let mut answered = Selection::new();
    answered.insert(1, 10u64);
    answered.supersede_with_deadline(1, 400);
    answered.insert(2, None);
    answered.supersede_with_deadline(2, 300);
    answered.supersede_with_deadline(99, 200);

    let validated = validate("test", answered, &sets);
    assert_eq!(validated.resend_deadline_ms(1), Some(400));
    assert_eq!(
      validated.resend_deadline_ms(2),
      None,
      "a dropped request cannot keep its deadline either"
    );
    assert_eq!(validated.resend_deadline_ms(99), None);

    let mut into = Selection::new();
    into.insert(3, 30u64);
    into.supersede_with_deadline(3, 120);
    merge(&mut into, validated);
    assert_eq!(
      into.resend_deadline_ms(3),
      Some(120),
      "an answer that did not mention a set leaves that set alone"
    );
    assert_eq!(into.resend_deadline_ms(1), Some(400));
  }
}
