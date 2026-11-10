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

//! The forward gate: whether one subscription's track carries a group, as the
//! connection's allocation decided.

use super::{AbrMessage, SstsState};
use moqtail::model::data::full_track_name::FullTrackName;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use tracing::warn;

/// `group` value meaning "nothing cached". Group ids are varints, so the top of
/// the u64 range is not a reachable group.
const GROUP_NONE: u64 = u64::MAX;

/// The gate for one subscription.
///
/// It keeps its answer for one group, so the rest of that group's Objects do not
/// re-derive it. A video track runs tens of Objects per group, and the answer
/// cannot change while `epoch` still matches the connection's: `SstsState::epoch`
/// moves on any membership change and on every recorded decision.
///
/// Only the subscription's own event task reads or writes it, so the three are
/// never observed mid-update and `Relaxed` is enough.
#[derive(Debug)]
pub struct ForwardGate {
  full_track_name: FullTrackName,
  relay_track_id: u64,
  epoch: AtomicU64,
  group: AtomicU64,
  forward: AtomicBool,
}

impl ForwardGate {
  pub fn new(full_track_name: FullTrackName, relay_track_id: u64) -> Self {
    Self {
      full_track_name,
      relay_track_id,
      epoch: AtomicU64::new(0),
      group: AtomicU64::new(GROUP_NONE),
      forward: AtomicBool::new(false),
    }
  }

  /// Whether the track carries `group_id`.
  ///
  /// Answered from the cache while the connection's epoch is unchanged, so
  /// every Object of a group after the one that triggered the decision costs
  /// three atomic loads. On a miss it resolves against the switching sets and
  /// the decision map, waiting for a decision if the group has none yet.
  pub async fn allows(&self, ssts: &SstsState, group_id: u64) -> bool {
    if self.group.load(Ordering::Relaxed) == group_id
      && self.epoch.load(Ordering::Relaxed) == ssts.epoch()
    {
      return self.forward.load(Ordering::Relaxed);
    }

    let (epoch, forward) = self.resolve(ssts, group_id).await;

    // Stamped with the epoch `resolve` sampled before its reads, not the one
    // current now: anything that moved while it was resolving has already
    // advanced the counter past this, so the next Object resolves again rather
    // than trusting an answer that was overtaken.
    self.forward.store(forward, Ordering::Relaxed);
    self.group.store(group_id, Ordering::Relaxed);
    self.epoch.store(epoch, Ordering::Relaxed);
    forward
  }

  /// Resolve the gate for `group_id` from scratch, returning the epoch the
  /// answer was read at along with the answer.
  async fn resolve(&self, ssts: &SstsState, group_id: u64) -> (u64, bool) {
    loop {
      // Created before the reads below and never polled on the path that
      // finds a decision, so it costs an atomic load and nothing else. Only
      // creating it matters: tokio guarantees a `Notified` observes any
      // `notify_waiters` from the moment it exists, which is what keeps a
      // decision recorded between the reads and the wait from being missed.
      let notified = ssts.decision_notify.notified();

      // Sampled before the reads, so a change racing with them leaves the
      // caller's cache stamped with a superseded epoch.
      let epoch = ssts.epoch();

      let set_id = {
        let manager = ssts.switching_sets.read().await;
        manager
          .get_set_for_track(&self.full_track_name)
          .map(|set| set.id)
      };

      // Not in a switching set, or no longer in one: SSTS does not apply to
      // this track, so the Object is forwarded. This also covers the set being
      // torn down while an Object of it waited for a decision that will now
      // never arrive.
      let Some(set_id) = set_id else {
        return (epoch, true);
      };

      let decision = {
        let decisions = ssts.group_decisions.read().await;
        decisions
          .get(&group_id)
          .and_then(|sets| sets.get(&set_id))
          .copied()
      };

      match decision {
        // The selected track carries the group. `None` selects no track at
        // all, so nothing from the set is forwarded for this group.
        Some(chosen) => return (epoch, chosen == Some(self.relay_track_id)),
        None => {
          if let Err(e) = ssts.abr_tx.send(AbrMessage::NewGroup(group_id)).await {
            warn!("Failed to send NewGroup to ABR: {:?}", e);
          }
          notified.await;
        }
      }
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use moqtail::model::control::setup::Setup;
  use std::collections::HashMap;

  fn track(name: &str) -> FullTrackName {
    FullTrackName::try_new("meet/room1", name).unwrap()
  }

  fn ssts() -> SstsState {
    SstsState::new(&Setup::new(vec![]), &[])
  }

  #[tokio::test]
  async fn a_track_in_no_switching_set_is_forwarded() {
    let gate = ForwardGate::new(track("video"), 10);
    assert!(gate.allows(&ssts(), 3).await);
  }

  #[tokio::test]
  async fn only_the_chosen_track_carries_a_decided_group() {
    let ssts = ssts();
    {
      let mut sets = ssts.switching_sets.write().await;
      sets
        .assign(track("hi"), 10, 1, 7, 0, 3000, 5, 2, 0)
        .unwrap();
      sets.assign(track("lo"), 11, 2, 7, 0, 500, 5, 2, 1).unwrap();
    }
    ssts
      .record_group_decision(4, HashMap::from([(7, Some(11))]), 0)
      .await;

    let hi = ForwardGate::new(track("hi"), 10);
    let lo = ForwardGate::new(track("lo"), 11);
    assert!(!hi.allows(&ssts, 4).await);
    assert!(lo.allows(&ssts, 4).await);

    // Leaving the set moves the epoch, so the cached answer for group 4 is not reused.
    ssts.switching_sets.write().await.remove(&track("hi"));
    assert!(hi.allows(&ssts, 4).await);
  }
}
