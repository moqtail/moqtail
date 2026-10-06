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

/// Where one track has got to. A soft switch has two tracks delivering at once,
/// their objects interleaved on separate streams, so each is followed on its own.
struct TrackSequence {
  last_group: u64,
  last_object: u64,
  /// How many group ids the group being received spans. One for an ordinary
  /// track; four for a track whose groups are four times as long as the grid its
  /// ids are numbered on. Zero where it is not known yet, which is the case for
  /// the group a subscriber joins part way through: the Prior Group ID Gap that
  /// would have said rides on the object starting it, which was never received.
  last_group_span: u64,
}

pub struct ReceptionStats {
  pub total_received: u64,
  pub parse_errors: u64,
  pub sequence_gaps: u64,
  last_group: u64,
  last_object: u64,
  tracks: HashMap<u64, TrackSequence>,
  start_time: Instant,
}

impl ReceptionStats {
  pub fn new() -> Self {
    Self {
      total_received: 0,
      parse_errors: 0,
      sequence_gaps: 0,
      last_group: 0,
      last_object: 0,
      tracks: HashMap::new(),
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

  /// Record a received object and validate sequence ordering within its track.
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
    track_alias: u64,
    group_id: u64,
    object_id: u64,
    prior_group_gap: u64,
  ) -> bool {
    self.total_received += 1;
    self.last_group = group_id;
    self.last_object = object_id;

    let Some(track) = self.tracks.get_mut(&track_alias) else {
      // First object of this track, nothing to check it against.
      self.tracks.insert(
        track_alias,
        TrackSequence {
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

  pub fn record_parse_error(&mut self) {
    self.parse_errors += 1;
  }

  pub fn elapsed_ms(&self) -> u128 {
    self.start_time.elapsed().as_millis()
  }

  pub fn report(&self) {
    info!(
      "Reception stats: total={}, last=group:{}/object:{}, errors={}, gaps={}, elapsed={}ms",
      self.total_received,
      self.last_group,
      self.last_object,
      self.parse_errors,
      self.sequence_gaps,
      self.elapsed_ms()
    );

    if self.parse_errors == 0 && self.sequence_gaps == 0 {
      info!("All objects received with correct sequence");
    } else {
      info!(
        "Found {} parse errors and {} sequence gaps",
        self.parse_errors, self.sequence_gaps
      );
    }
  }
}
