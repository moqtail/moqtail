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

//! Pure, synchronous Top-N active-speaker ranker.
//!
//! The relay's TopNCoordinator drives this on a periodic tick. No I/O, no async.

use crate::model::data::full_track_name::FullTrackName;
use std::collections::HashMap;

/// Speech-activity property values (extension key 0x12).
pub const SPEECH_SILENT: u64 = 0;
pub const SPEECH_SPEAKING: u64 = 1;
pub const SPEECH_START: u64 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TieBreakPolicy {
  /// Tracks that entered the ranker earlier (lower arrival_seq) win ties.
  OldestWins,
  /// Tracks most recently updated (higher last_update_seq) win ties.
  MostRecentWins,
}

#[derive(Debug, Clone)]
pub struct TopNRankerConfig {
  /// The extension header key this ranker observes (typically 0x12).
  pub property_type: u64,
  pub tie_break_policy: TieBreakPolicy,
}

#[derive(Debug, Clone)]
pub struct RankEntry {
  /// Connection IDs of all active publishers for this track.
  pub publisher_connection_ids: Vec<usize>,
  /// Latest observed property value (0=SILENT, 1=SPEAKING, 2=SPEECH_START).
  pub property_value: u64,
  /// Monotonic counter: lower = track entered the ranker earlier.
  pub arrival_seq: u64,
  /// Logical clock at the time of last `observe_value` call with a changed value.
  pub last_update_seq: u64,
}

#[derive(Debug, Clone)]
pub struct TopNRanker {
  pub config: TopNRankerConfig,
  entries: HashMap<FullTrackName, RankEntry>,
  next_arrival_seq: u64,
}

impl TopNRanker {
  pub fn new(config: TopNRankerConfig) -> Self {
    Self {
      config,
      entries: HashMap::new(),
      next_arrival_seq: 0,
    }
  }

  /// Record a new property_value observation for a track.
  /// `update_seq` is the coordinator's logical clock at call time.
  /// Only updates `last_update_seq` when the value actually changes.
  /// Returns `true` if the ranking-relevant state changed (new entry or value change).
  pub fn observe_value(
    &mut self,
    full_track_name: &FullTrackName,
    publisher_conn_ids: Vec<usize>,
    value: u64,
    update_seq: u64,
  ) -> bool {
    let mut changed = false;
    let entry = self
      .entries
      .entry(full_track_name.clone())
      .or_insert_with(|| {
        let seq = self.next_arrival_seq;
        self.next_arrival_seq += 1;
        changed = true;
        RankEntry {
          publisher_connection_ids: publisher_conn_ids.clone(),
          property_value: value,
          arrival_seq: seq,
          last_update_seq: update_seq,
        }
      });

    entry.publisher_connection_ids = publisher_conn_ids;
    if entry.property_value != value {
      entry.property_value = value;
      entry.last_update_seq = update_seq;
      changed = true;
    }
    changed
  }

  /// Remove all entries whose publisher_connection_ids contain `connection_id`.
  pub fn remove_publisher(&mut self, connection_id: usize) -> bool {
    let mut removed_any = false;
    self.entries.retain(|_, e| {
      let prev_is_empty = e.publisher_connection_ids.is_empty();
      e.publisher_connection_ids.retain(|id| *id != connection_id);
      if !removed_any && !prev_is_empty && e.publisher_connection_ids.is_empty() {
        removed_any = true;
      }
      !e.publisher_connection_ids.is_empty()
    });
    removed_any
  }

  /// Remove a specific track entirely.
  pub fn remove_track(&mut self, full_track_name: &FullTrackName) -> Option<RankEntry> {
    self.entries.remove(full_track_name)
  }

  /// Returns true if a track has been observed (regardless of its current value).
  pub fn has_entry(&self, full_track_name: &FullTrackName) -> bool {
    self.entries.contains_key(full_track_name)
  }

  /// Compute the top-N tracks for a given subscriber.
  ///
  /// Rules:
  /// - Exclude entries where `requester_conn_id` is in `publisher_connection_ids` (self-exclusion).
  /// - Exclude SILENT entries (value == 0) — they don't occupy slots.
  /// - Sort by property_value descending (SPEECH_START=2 > SPEAKING=1), then apply tie-break.
  /// - Return at most `n` FullTrackNames.
  pub fn compute_top_n(&self, requester_conn_id: usize, n: usize) -> Vec<FullTrackName> {
    if n == 0 {
      return vec![];
    }

    let mut candidates: Vec<(&FullTrackName, &RankEntry)> = self
      .entries
      .iter()
      .filter(|(_, e)| {
        e.property_value > SPEECH_SILENT && !e.publisher_connection_ids.contains(&requester_conn_id)
      })
      .collect();

    candidates.sort_by(|(_, a), (_, b)| {
      // Higher value wins (SPEECH_START=2 beats SPEAKING=1).
      // Sort descending by property_value, then apply tie-break policy.
      b.property_value
        .cmp(&a.property_value)
        .then_with(|| match self.config.tie_break_policy {
          TieBreakPolicy::OldestWins => a.arrival_seq.cmp(&b.arrival_seq),
          TieBreakPolicy::MostRecentWins => b.last_update_seq.cmp(&a.last_update_seq),
        })
    });

    candidates
      .into_iter()
      .take(n)
      .map(|(ftn, _)| ftn.clone())
      .collect()
  }

  /// Number of entries currently tracked (including SILENT ones).
  pub fn entry_count(&self) -> usize {
    self.entries.len()
  }
}

#[cfg(test)]
mod tests {

  use super::*;
  use crate::model::common::tuple::Tuple;
  use crate::model::data::full_track_name::FullTrackName;

  fn make_ftn(ns: &str, name: &str) -> FullTrackName {
    use crate::model::common::tuple::TupleField;
    let ns_tuple = Tuple::from_utf8_path(ns);
    FullTrackName::new(ns_tuple, TupleField::from_utf8(name)).unwrap()
  }

  fn make_ranker() -> TopNRanker {
    TopNRanker::new(TopNRankerConfig {
      property_type: 0x12,
      tie_break_policy: TieBreakPolicy::OldestWins,
    })
  }

  #[test]
  fn test_top_n_basic() {
    let mut r = make_ranker();
    let a = make_ftn("/room/alice", "audio");
    let b = make_ftn("/room/bob", "audio");
    let c = make_ftn("/room/carol", "audio");

    r.observe_value(&a, vec![10], SPEECH_SPEAKING, 1);
    r.observe_value(&b, vec![20], SPEECH_START, 2);
    r.observe_value(&c, vec![30], SPEECH_SPEAKING, 3);

    // top-2 for subscriber 99 (no self-exclusion): bob (SPEECH_START=2) first
    let top2 = r.compute_top_n(99, 2);
    assert_eq!(top2.len(), 2);
    assert_eq!(top2[0], b);
    // alice and carol both SPEAKING; alice arrived first → OldestWins → alice second
    assert_eq!(top2[1], a);
  }

  #[test]
  fn test_silent_excluded() {
    let mut r = make_ranker();
    let a = make_ftn("/room/alice", "audio");
    let b = make_ftn("/room/bob", "audio");

    r.observe_value(&a, vec![10], SPEECH_SPEAKING, 1);
    r.observe_value(&b, vec![20], SPEECH_SILENT, 2);

    let top2 = r.compute_top_n(99, 2);
    assert_eq!(top2.len(), 1);
    assert_eq!(top2[0], a);
  }

  #[test]
  fn test_self_exclusion() {
    let mut r = make_ranker();
    let a = make_ftn("/room/alice", "audio");
    let b = make_ftn("/room/bob", "audio");

    r.observe_value(&a, vec![10], SPEECH_SPEAKING, 1);
    r.observe_value(&b, vec![20], SPEECH_SPEAKING, 2);

    // subscriber 10 is alice's publisher → alice excluded
    let top2 = r.compute_top_n(10, 2);
    assert_eq!(top2.len(), 1);
    assert_eq!(top2[0], b);
  }

  #[test]
  fn test_unranked_tracks_not_in_ranker() {
    let mut r = make_ranker();
    let a = make_ftn("/room/alice", "audio");
    let v = make_ftn("/room/alice", "video"); // never calls observe_value

    r.observe_value(&a, vec![10], SPEECH_SPEAKING, 1);

    assert!(r.has_entry(&a));
    assert!(!r.has_entry(&v)); // video track never entered ranker
    let top = r.compute_top_n(99, 5);
    assert_eq!(top, vec![a]);
  }

  #[test]
  fn test_remove_publisher() {
    let mut r = make_ranker();
    let a = make_ftn("/room/alice", "audio");
    let b = make_ftn("/room/bob", "audio");

    r.observe_value(&a, vec![10], SPEECH_SPEAKING, 1);
    r.observe_value(&b, vec![20], SPEECH_SPEAKING, 2);

    r.remove_publisher(10);

    assert!(!r.has_entry(&a));
    assert!(r.has_entry(&b));
  }

  #[test]
  fn test_oldest_wins_tiebreak() {
    let mut r = make_ranker();
    let a = make_ftn("/room/alice", "audio");
    let b = make_ftn("/room/bob", "audio");
    let c = make_ftn("/room/carol", "audio");

    // a entered first, then b, then c — all SPEAKING
    r.observe_value(&a, vec![10], SPEECH_SPEAKING, 1);
    r.observe_value(&b, vec![20], SPEECH_SPEAKING, 2);
    r.observe_value(&c, vec![30], SPEECH_SPEAKING, 3);

    let top2 = r.compute_top_n(99, 2);
    assert_eq!(top2, vec![a, b]);
  }

  #[test]
  fn test_speech_start_priority_over_speaking() {
    let mut r = make_ranker();
    let a = make_ftn("/room/alice", "audio");
    let b = make_ftn("/room/bob", "audio");

    // alice entered first and is SPEAKING; bob entered later with SPEECH_START
    r.observe_value(&a, vec![10], SPEECH_SPEAKING, 1);
    r.observe_value(&b, vec![20], SPEECH_START, 2);

    let top1 = r.compute_top_n(99, 1);
    assert_eq!(top1, vec![b]); // SPEECH_START beats SPEAKING regardless of arrival order
  }
}
