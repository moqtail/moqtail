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

//! SSTS switching sets (draft-wilaw-moq-moqt-ssts).
//!
//! A switching set is a collection of tracks representing the same content
//! at different throughput levels; the ABR selects exactly one track per
//! active set to forward. `throughput threshold` is a per-member property,
//! while `weight`, `activate` and `rank` are set properties: when several
//! subscriptions in the same set specify different values, the most recently
//! received message wins.

use moqtail::model::data::full_track_name::FullTrackName;
use moqtail_ssts::SetSnapshot;
use std::collections::HashMap;
use std::fmt;

/// The one place mechanism state becomes algorithm state: everything, and only
/// everything, an algorithm is allowed to know about a set.
impl From<&SwitchingSet> for SetSnapshot {
  fn from(set: &SwitchingSet) -> Self {
    Self {
      id: set.id,
      algorithm_id: set.algorithm_id,
      rank: set.rank,
      weight: set.weight,
      active: set.is_active(),
      members: set
        .members
        .iter()
        .map(|m| (m.throughput_threshold_kbps, m.relay_track_id))
        .collect(),
    }
  }
}

#[derive(Debug, Clone)]
pub enum SwitchingSetError {
  /// The track is already assigned to a different switching set; the
  /// subscription MUST be rejected with a Parameter Error.
  TrackInDifferentSet,
  /// The track does not belong to any switching set.
  TrackNotInSet,
}

impl fmt::Display for SwitchingSetError {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    match self {
      Self::TrackInDifferentSet => {
        write!(f, "track is already assigned to a different switching set")
      }
      Self::TrackNotInSet => write!(f, "track is not in any switching set"),
    }
  }
}

/// A track's membership in a switching set.
#[derive(Debug, Clone)]
pub struct SwitchingSetMember {
  pub full_track_name: FullTrackName,
  pub relay_track_id: u64,
  /// Minimum throughput (kbps) required to select this track. Property of
  /// the subscription.
  pub throughput_threshold_kbps: u64,
  /// Bookkeeping: the request id that established this track's subscription
  /// (SUBSCRIBE request id, or the PUBLISH request id for pushed tracks).
  #[allow(dead_code)]
  pub request_id: u64,
}

/// A switching set: tracks representing the same content at different
/// throughput levels.
#[derive(Debug, Clone)]
pub struct SwitchingSet {
  pub id: u64,
  pub algorithm_id: u64,
  /// Sorted by throughput threshold, ascending (index 0 = lowest quality).
  pub members: Vec<SwitchingSetMember>,
  /// Relative weight for bandwidth allocation among sets that share the same
  /// rank; 1..=10.
  pub weight: u64,
  /// 0 pauses SSTS for this set; switching activates once the number of
  /// assigned tracks is >= this value.
  pub activate: u64,
  /// Degradation priority; lower values are protected first.
  pub rank: u8,
}

impl SwitchingSet {
  /// Whether the allocation runs for this set: enabled and enough tracks.
  pub fn is_active(&self) -> bool {
    self.activate > 0 && self.members.len() as u64 >= self.activate
  }

  /// The highest-throughput track in the set, if any.
  #[allow(dead_code)]
  pub fn top_member(&self) -> Option<&SwitchingSetMember> {
    self.members.last()
  }
}

#[derive(Debug, Default)]
pub struct SwitchingSetManager {
  pub sets: HashMap<u64, SwitchingSet>,
  /// Track -> switching set id. A track MUST only be in one set at a time.
  pub track_to_set: HashMap<FullTrackName, u64>,
  /// Relay track id -> switching set id, so a forwarding stream can be
  /// attributed to the set it carries.
  pub relay_track_to_set: HashMap<u64, u64>,
  /// Live forwarding streams per set. Owned here, next to the sets, because
  /// the counter only means something while the set exists: a set that is
  /// torn down takes its count with it, so a publisher that later reuses the
  /// id does not inherit a queue depth it never created.
  open_streams: HashMap<u64, u64>,
}

impl SwitchingSetManager {
  pub fn new() -> Self {
    Self::default()
  }

  /// A forwarding stream opened for the set this relay track belongs to.
  pub fn note_stream_opened(&mut self, relay_track_id: u64) {
    if let Some(set_id) = self.relay_track_to_set.get(&relay_track_id) {
      *self.open_streams.entry(*set_id).or_default() += 1;
    }
  }

  /// A forwarding stream of this relay track ended, by finishing, being reset
  /// or being stopped by the peer.
  pub fn note_stream_closed(&mut self, relay_track_id: u64) {
    let Some(set_id) = self.relay_track_to_set.get(&relay_track_id).copied() else {
      return;
    };
    if let Some(count) = self.open_streams.get_mut(&set_id) {
      *count = count.saturating_sub(1);
      if *count == 0 {
        self.open_streams.remove(&set_id);
      }
    }
  }

  /// Live forwarding streams per set, for the algorithms' input. A set with no
  /// entry has no streams.
  pub fn open_streams(&self) -> HashMap<u64, u64> {
    self.open_streams.clone()
  }

  #[allow(clippy::too_many_arguments)]
  pub fn assign(
    &mut self,
    full_track_name: FullTrackName,
    relay_track_id: u64,
    request_id: u64,
    switching_set_id: u64,
    algorithm_id: u64,
    throughput_threshold_kbps: u64,
    weight: u64,
    activate: u64,
    rank: u8,
  ) -> Result<(), SwitchingSetError> {
    if let Some(&existing_set_id) = self.track_to_set.get(&full_track_name)
      && existing_set_id != switching_set_id
    {
      return Err(SwitchingSetError::TrackInDifferentSet);
    }

    let member = SwitchingSetMember {
      full_track_name: full_track_name.clone(),
      relay_track_id,
      throughput_threshold_kbps,
      request_id,
    };

    let set = self
      .sets
      .entry(switching_set_id)
      .or_insert_with(|| SwitchingSet {
        id: switching_set_id,
        algorithm_id,
        members: Vec::new(),
        weight,
        activate,
        rank,
      });

    // Set properties: the most recently received message wins.
    set.algorithm_id = algorithm_id;
    set.weight = weight;
    set.activate = activate;
    set.rank = rank;

    // Replace or insert the member.
    let mut previous_relay_track_id = None;
    if let Some(existing) = set
      .members
      .iter_mut()
      .find(|m| m.full_track_name == full_track_name)
    {
      previous_relay_track_id = Some(existing.relay_track_id);
      *existing = member;
    } else {
      set.members.push(member);
    }

    set.members.sort_by_key(|m| m.throughput_threshold_kbps);

    self.track_to_set.insert(full_track_name, switching_set_id);
    // A member re-assigned under a new relay track id must not leave the old
    // one pointing at this set: streams on a track that is no longer a member
    // would keep counting towards this set's depth forever.
    if let Some(previous) = previous_relay_track_id
      && previous != relay_track_id
    {
      self.relay_track_to_set.remove(&previous);
    }
    self
      .relay_track_to_set
      .insert(relay_track_id, switching_set_id);
    Ok(())
  }

  /// Remove a track (unsubscribed or PUBLISH_DONE) and delete the set once its
  /// last track leaves. `activate` is a configured threshold, not a counter,
  /// so it is left alone: `is_active` re-evaluates against the new member
  /// count on its own.
  pub fn remove(&mut self, full_track_name: &FullTrackName) {
    let Some(set_id) = self.track_to_set.remove(full_track_name) else {
      return;
    };
    let Some(set) = self.sets.get_mut(&set_id) else {
      return;
    };
    let relay_track_id = set
      .members
      .iter()
      .find(|m| m.full_track_name == *full_track_name)
      .map(|m| m.relay_track_id);
    set
      .members
      .retain(|m| m.full_track_name != *full_track_name);
    if let Some(relay_track_id) = relay_track_id {
      self.relay_track_to_set.remove(&relay_track_id);
    }
    if set.members.is_empty() {
      self.sets.remove(&set_id);
      // The set is gone, and with it the meaning of its stream count.
      self.open_streams.remove(&set_id);
    }
  }

  /// Apply a SWITCHING_SET_ASSIGNMENT update (REQUEST_UPDATE) to the set the
  /// track belongs to. Only the provided values override the set properties
  /// (last write wins).
  pub fn update_assignment(
    &mut self,
    full_track_name: &FullTrackName,
    weight: Option<u64>,
    activate: Option<u64>,
    rank: Option<u8>,
  ) -> Result<(), SwitchingSetError> {
    let set_id = self
      .track_to_set
      .get(full_track_name)
      .copied()
      .ok_or(SwitchingSetError::TrackNotInSet)?;

    let Some(set) = self.sets.get_mut(&set_id) else {
      return Err(SwitchingSetError::TrackNotInSet);
    };
    if let Some(w) = weight {
      set.weight = w;
    }
    if let Some(a) = activate {
      set.activate = a;
    }
    if let Some(r) = rank {
      set.rank = r;
    }
    Ok(())
  }

  pub fn get_set_for_track(&self, full_track_name: &FullTrackName) -> Option<&SwitchingSet> {
    self
      .track_to_set
      .get(full_track_name)
      .and_then(|id| self.sets.get(id))
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use moqtail::model::common::tuple::{Tuple, TupleField};

  fn make_track(ns: &str, name: &str) -> FullTrackName {
    FullTrackName::new(Tuple::from_utf8_path(ns), TupleField::from_utf8(name))
      .expect("create track name")
  }

  fn assign(
    manager: &mut SwitchingSetManager,
    track: &FullTrackName,
    relay_track_id: u64,
    set_id: u64,
    threshold: u64,
  ) {
    manager
      .assign(
        track.clone(),
        relay_track_id,
        relay_track_id,
        set_id,
        0,
        threshold,
        5,
        2,
        0,
      )
      .unwrap();
  }

  #[test]
  fn test_assign_and_remove() {
    let mut manager = SwitchingSetManager::new();
    let track1 = make_track("ns", "1080p");
    let track2 = make_track("ns", "480p");

    assign(&mut manager, &track1, 10, 1, 3000);
    assign(&mut manager, &track2, 20, 1, 800);

    // Assigning the same track to a different set must fail.
    assert!(matches!(
      manager.assign(track1.clone(), 10, 1, 2, 0, 3000, 5, 2, 0),
      Err(SwitchingSetError::TrackInDifferentSet)
    ));

    manager.remove(&track1);
    assert!(manager.get_set_for_track(&track1).is_none());

    let set = manager.get_set_for_track(&track2).unwrap();
    assert_eq!(set.members.len(), 1);
    // `activate` is a configured threshold, not a counter: removing a member
    // must not change it.
    assert_eq!(set.activate, 2);
  }

  #[test]
  fn test_remove_deletes_emptied_set() {
    let mut manager = SwitchingSetManager::new();
    let track = make_track("ns", "t");
    assign(&mut manager, &track, 1, 7, 100);

    manager.remove(&track);
    // The last member removed the set entirely.
    assert!(!manager.sets.contains_key(&7));
    assert!(manager.get_set_for_track(&track).is_none());
  }

  #[test]
  fn test_is_active() {
    let mut manager = SwitchingSetManager::new();
    let track1 = make_track("ns", "a");
    let track2 = make_track("ns", "b");

    // activate = 2, only one track assigned: not active yet.
    manager
      .assign(track1.clone(), 1, 1, 1, 0, 100, 5, 2, 0)
      .unwrap();
    assert!(!manager.get_set_for_track(&track1).unwrap().is_active());

    manager
      .assign(track2.clone(), 2, 2, 1, 0, 200, 5, 2, 0)
      .unwrap();
    assert!(manager.get_set_for_track(&track2).unwrap().is_active());
  }

  #[test]
  fn test_update_assignment_last_write_wins() {
    let mut manager = SwitchingSetManager::new();
    let track = make_track("ns", "t");
    assign(&mut manager, &track, 1, 3, 100);

    manager
      .update_assignment(&track, Some(9), Some(0), Some(4))
      .unwrap();
    let set = manager.get_set_for_track(&track).unwrap();
    assert_eq!(set.weight, 9);
    assert_eq!(set.activate, 0);
    assert_eq!(set.rank, 4);
    // Paused sets are never active.
    assert!(!set.is_active());

    // None fields keep the previous values.
    manager
      .update_assignment(&track, None, None, Some(1))
      .unwrap();
    let set = manager.get_set_for_track(&track).unwrap();
    assert_eq!(set.weight, 9);
    assert_eq!(set.activate, 0);
    assert_eq!(set.rank, 1);
  }

  #[test]
  fn test_relay_track_to_set_mapping() {
    let mut manager = SwitchingSetManager::new();
    let track = make_track("ns", "t");
    assign(&mut manager, &track, 42, 7, 100);
    assert_eq!(manager.relay_track_to_set.get(&42), Some(&7));

    manager.remove(&track);
    assert_eq!(manager.relay_track_to_set.get(&42), None);
  }

  #[test]
  fn test_update_assignment_unknown_track() {
    let mut manager = SwitchingSetManager::new();
    let track = make_track("ns", "t");
    assert!(matches!(
      manager.update_assignment(&track, Some(1), None, None),
      Err(SwitchingSetError::TrackNotInSet)
    ));
  }
  #[test]
  fn test_reassigned_track_does_not_leave_a_stale_mapping() {
    // A track re-subscribed under a new relay track id must stop counting
    // streams opened on the old one, or the set keeps a depth it no longer has.
    let mut manager = SwitchingSetManager::new();
    let track = make_track("ns", "t");
    assign(&mut manager, &track, 42, 7, 100);

    manager
      .assign(track.clone(), 43, 2, 7, 0, 100, 5, 2, 0)
      .unwrap();
    assert_eq!(manager.relay_track_to_set.get(&43), Some(&7));
    assert_eq!(manager.relay_track_to_set.get(&42), None);
  }

  #[test]
  fn test_stream_counts_follow_the_set() {
    let mut manager = SwitchingSetManager::new();
    let track = make_track("ns", "t");
    assign(&mut manager, &track, 42, 7, 100);

    manager.note_stream_opened(42);
    manager.note_stream_opened(42);
    assert_eq!(manager.open_streams().get(&7), Some(&2));

    manager.note_stream_closed(42);
    assert_eq!(manager.open_streams().get(&7), Some(&1));

    // More closes than opens cannot make the count negative.
    manager.note_stream_closed(42);
    manager.note_stream_closed(42);
    assert_eq!(manager.open_streams().get(&7), None);
  }

  #[test]
  fn test_removing_a_set_clears_its_stream_count() {
    // The count of a set that no longer exists has no meaning, and a publisher
    // that reuses the id must not inherit it.
    let mut manager = SwitchingSetManager::new();
    let track = make_track("ns", "t");
    assign(&mut manager, &track, 42, 7, 100);
    manager.note_stream_opened(42);
    assert_eq!(manager.open_streams().get(&7), Some(&1));

    manager.remove(&track);
    assert_eq!(manager.open_streams().get(&7), None);

    assign(&mut manager, &track, 42, 7, 100);
    assert_eq!(manager.open_streams().get(&7), None);
  }
}
