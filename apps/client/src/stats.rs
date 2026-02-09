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

use std::time::Instant;
use tracing::info;

pub struct ReceptionStats {
  pub total_received: u64,
  pub parse_errors: u64,
  pub sequence_gaps: u64,
  last_group: u64,
  last_object: u64,
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
      start_time: Instant::now(),
    }
  }

  /// Record a received object and validate sequence ordering.
  /// Returns true if the sequence is valid, false if a gap was detected.
  pub fn record_object(&mut self, group_id: u64, object_id: u64) -> bool {
    self.total_received += 1;

    if self.total_received == 1 {
      // First object, no sequence check needed
      self.last_group = group_id;
      self.last_object = object_id;
      return true;
    }

    let expected_next = (group_id == self.last_group && object_id == self.last_object + 1)
      || (group_id == self.last_group + 1 && object_id == 0);

    let sequence_ok = if expected_next {
      true
    } else {
      self.sequence_gaps += 1;
      false
    };

    self.last_group = group_id;
    self.last_object = object_id;
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
