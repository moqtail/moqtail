// Copyright 2025 The MOQtail Authors
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

use moqtail::model::property::object_property::ObjectProperty;
use std::collections::HashMap;
use std::time::Instant;
use tracing::info;

/// Where one subscription has got to. A soft switch has two tracks delivering at
/// once, and two subscriptions to one track each get their own copy, their objects
/// interleaved on separate streams, so each is followed on its own.
struct TrackSequence {
  received: u64,
  gaps: u64,
  /// Objects that should not have been delivered, by the subscription's Range Filters.
  violations: u64,
  last_group: u64,
  last_object: u64,
  /// How many group ids the group being received spans. One for an ordinary
  /// track; four for a track whose groups are four times as long as the grid its
  /// ids are numbered on. Zero where it is not known yet, which is the case for
  /// the group a subscriber joins part way through: the Prior Group ID Gap that
  /// would have said rides on the object starting it, which was never received.
  last_group_span: u64,
}

/// What one subscription received, as `ReceptionStats::per_track` reports it.
#[derive(Debug, PartialEq, Eq)]
pub struct TrackReport {
  pub track: u64,
  pub received: u64,
  /// Of `received`, the objects a fill delivered.
  pub fill: u64,
  pub gaps: u64,
  pub violations: u64,
}

pub struct ReceptionStats {
  pub total_received: u64,
  /// Of `total_received`, the objects a fill delivered.
  pub fill_received: u64,
  pub parse_errors: u64,
  pub sequence_gaps: u64,
  /// Copies of an object a subscription already had.
  pub duplicates: u64,
  /// Objects a subscription's Range Filters do not pass.
  pub filter_violations: u64,
  last_group: u64,
  last_object: u64,
  tracks: HashMap<u64, TrackSequence>,
  /// Fill objects per subscription, kept out of its sequence: a fill and the live
  /// objects arrive on separate streams and interleave, which would read as gaps.
  fills: HashMap<u64, u64>,
  start_time: Instant,
}

impl ReceptionStats {
  pub fn new() -> Self {
    Self {
      total_received: 0,
      fill_received: 0,
      parse_errors: 0,
      sequence_gaps: 0,
      duplicates: 0,
      filter_violations: 0,
      last_group: 0,
      last_object: 0,
      tracks: HashMap::new(),
      fills: HashMap::new(),
      start_time: Instant::now(),
    }
  }

  /// The Prior Group ID Gap an object carries, or 0 where it carries none.
  pub fn prior_group_gap(properties: Option<&Vec<ObjectProperty>>) -> u64 {
    properties
      .into_iter()
      .flatten()
      .find_map(|property| match property {
        ObjectProperty::PriorGroupIdGap { gap } => Some(*gap),
        _ => None,
      })
      .unwrap_or(0)
  }

  /// Record a received object and validate sequence ordering within the
  /// subscription it belongs to, which `track` names.
  /// Returns true if the sequence is valid, false if a gap was detected.
  ///
  /// `prior_group_gap` is the Prior Group ID Gap the object carries: how many
  /// group ids its publisher skipped before it. A track whose groups are longer
  /// than the grid its ids are numbered on skips ids by design, and what it skips
  /// is not missing data.
  ///
  /// The next group is expected where the one before it ended, so the span used
  /// is that of the group just left rather than of the one arriving.
  pub fn record_object(
    &mut self,
    track: u64,
    group_id: u64,
    object_id: u64,
    prior_group_gap: u64,
  ) -> bool {
    self.total_received += 1;
    self.last_group = group_id;
    self.last_object = object_id;

    let Some(track) = self.tracks.get_mut(&track) else {
      // First object of this track, nothing to check it against.
      self.tracks.insert(
        track,
        TrackSequence {
          received: 1,
          gaps: 0,
          violations: 0,
          last_group: group_id,
          last_object: object_id,
          last_group_span: if object_id == 0 {
            prior_group_gap + 1
          } else {
            0
          },
        },
      );
      return true;
    };

    let sequence_ok = if group_id == track.last_group {
      object_id == track.last_object + 1
    } else {
      // A new group starts at object 0, where the last one ended -- unless where
      // the last one ended is exactly what was never learnt.
      object_id == 0
        && (track.last_group_span == 0 || group_id == track.last_group + track.last_group_span)
    };

    track.received += 1;
    if !sequence_ok {
      track.gaps += 1;
    }
    if group_id != track.last_group {
      track.last_group_span = prior_group_gap + 1;
    }
    track.last_group = group_id;
    track.last_object = object_id;

    if !sequence_ok {
      self.sequence_gaps += 1;
    }
    sequence_ok
  }

  /// Record an object for a subscription with Range Filters, whose sequence has
  /// gaps by design, and whether it passed them. Returns `passed`.
  pub fn record_filtered_object(
    &mut self,
    track: u64,
    group_id: u64,
    object_id: u64,
    passed: bool,
  ) -> bool {
    self.total_received += 1;
    self.last_group = group_id;
    self.last_object = object_id;
    let sequence = self.tracks.entry(track).or_insert(TrackSequence {
      received: 0,
      gaps: 0,
      violations: 0,
      last_group: group_id,
      last_object: object_id,
      last_group_span: 0,
    });
    sequence.received += 1;
    sequence.last_group = group_id;
    sequence.last_object = object_id;
    if !passed {
      sequence.violations += 1;
      self.filter_violations += 1;
    }
    passed
  }

  pub fn record_duplicates(&mut self, count: usize) {
    self.duplicates += count as u64;
  }

  /// Record an object a fill delivered to the subscription `track` names. It is
  /// counted, but not checked against the sequence or the Range Filters: the fill
  /// covers what was published before the subscription started, under filters of
  /// its own.
  pub fn record_fill_object(&mut self, track: u64) {
    self.total_received += 1;
    self.fill_received += 1;
    *self.fills.entry(track).or_default() += 1;
  }

  /// Per subscription: objects received, how many of them a fill delivered, and
  /// the sequence gaps and filter violations seen.
  pub fn per_track(&self) -> Vec<TrackReport> {
    let mut tracks: Vec<u64> = self
      .tracks
      .keys()
      .chain(self.fills.keys())
      .copied()
      .collect();
    tracks.sort();
    tracks.dedup();
    tracks
      .into_iter()
      .map(|track| {
        let fill = self.fills.get(&track).copied().unwrap_or(0);
        let sequence = self.tracks.get(&track);
        TrackReport {
          track,
          received: sequence.map_or(0, |s| s.received) + fill,
          fill,
          gaps: sequence.map_or(0, |s| s.gaps),
          violations: sequence.map_or(0, |s| s.violations),
        }
      })
      .collect()
  }

  pub fn record_parse_error(&mut self) {
    self.parse_errors += 1;
  }

  pub fn elapsed_ms(&self) -> u128 {
    self.start_time.elapsed().as_millis()
  }

  pub fn report(&self) {
    info!(
      "Reception stats: total={}, fill={}, last=group:{}/object:{}, errors={}, gaps={}, duplicates={}, filter_violations={}, elapsed={}ms",
      self.total_received,
      self.fill_received,
      self.last_group,
      self.last_object,
      self.parse_errors,
      self.sequence_gaps,
      self.duplicates,
      self.filter_violations,
      self.elapsed_ms()
    );

    if self.parse_errors == 0 && self.sequence_gaps == 0 && self.filter_violations == 0 {
      info!("All objects received with correct sequence");
    } else {
      info!(
        "Found {} parse errors, {} sequence gaps and {} filter violations",
        self.parse_errors, self.sequence_gaps, self.filter_violations
      );
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn fill_objects_are_counted_outside_the_sequence() {
    let mut stats = ReceptionStats::new();
    // The live objects start at group 3 while the fill is still delivering
    // groups 1 and 2: interleaved, they would read as gaps.
    stats.record_fill_object(0);
    assert!(stats.record_object(0, 3, 0, 0));
    stats.record_fill_object(0);
    assert!(stats.record_object(0, 3, 1, 0));
    stats.record_fill_object(2);

    assert_eq!(stats.total_received, 5);
    assert_eq!(stats.fill_received, 3);
    assert_eq!(stats.sequence_gaps, 0);
    assert_eq!(
      stats.per_track(),
      vec![
        TrackReport {
          track: 0,
          received: 4,
          fill: 2,
          gaps: 0,
          violations: 0,
        },
        TrackReport {
          track: 2,
          received: 1,
          fill: 1,
          gaps: 0,
          violations: 0,
        },
      ]
    );
  }
}
