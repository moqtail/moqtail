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

use crate::server::stream_id::StreamId;
use bytes::Bytes;
use fnv::FnvHasher;
use moqtail::model::common::tuple::{Tuple, TupleField};
use moqtail::model::control::constant::GroupOrder;
use moqtail::{
  model::control::control_message::ControlMessageTrait, transport::data_stream_handler::HeaderInfo,
};
use once_cell::sync::Lazy;
use std::hash::Hasher;
use std::time::Instant;

// Static reference time: set when the program starts
pub static BASE_TIME: Lazy<Instant> = Lazy::new(Instant::now);

/// Reserved-namespace policy. Returns a rejection reason when a request for this
/// namespace/track must be rejected with DOES_NOT_EXIST: the single `.`
/// namespace, and any `.session` request (session-level tracks are managed
/// locally and never forwarded to other sessions; the relay defines none, and
/// an empty track name under `.session` is defined not to exist). Other reserved
/// namespaces (a first field beginning with `.`) are passed through unchanged.
pub fn reserved_namespace_rejection(
  namespace: &Tuple,
  track_name: &TupleField,
) -> Option<&'static str> {
  if namespace.is_reserved_dot() {
    return Some("the '.' namespace is reserved and does not exist");
  }
  if namespace.is_session_namespace() {
    if track_name.is_empty() {
      return Some(".session with an empty track name does not exist");
    }
    return Some("unrecognized session-level track");
  }
  None
}

pub fn print_msg_bytes(msg: &impl ControlMessageTrait) {
  let bytes = msg.serialize();
  print_bytes(bytes.as_ref().unwrap());
}

pub fn print_bytes(buffer: &Bytes) {
  for byte in buffer.iter() {
    print!("{byte:02X} ");
  }
  println!();
}

pub fn bytes_to_hex(buffer: &Bytes) -> String {
  let mut hex = String::new();
  for byte in buffer.iter() {
    hex.push_str(&format!("{byte:02X} "));
  }
  hex
}

pub fn build_stream_id(relay_track_id: u64, header: &HeaderInfo) -> StreamId {
  match header {
    HeaderInfo::Fetch {
      header,
      fetch_request: _,
    } => StreamId::new_fetch(relay_track_id, header.request_id),
    HeaderInfo::Subgroup { header } => {
      StreamId::new_subgroup(relay_track_id, header.group_id, header.subgroup_id)
    }
  }
}

pub fn passed_time_since_start() -> u128 {
  (Instant::now() - *BASE_TIME).as_millis()
}

pub fn fnv_hash(bytes: &[u8]) -> u64 {
  let mut hasher = FnvHasher::default();
  hasher.write(bytes);
  hasher.finish()
}

/// Compute QUIC stream priority from MOQT scheduling parameters.
///
/// The i32 space is divided into 65536 bands (one per sub_prio × pub_prio pair).
/// Within each band, group_id determines relative position according to group_order:
///   Ascending / Original – lower group_id = higher priority (counts down from band_max)
///   Descending            – higher group_id = higher priority (counts up from band_min)
pub(crate) fn compute_stream_priority(
  sub_prio: u8,
  pub_prio: u8,
  group_order: GroupOrder,
  group_id: u64,
) -> i32 {
  const BAND_SIZE: i64 = 65536;
  let priority_index = (255 - sub_prio as i64) * 256 + (255 - pub_prio as i64);
  let band_min = i32::MIN as i64 + priority_index * BAND_SIZE;
  let group_slot = (group_id % BAND_SIZE as u64) as i64;
  match group_order {
    GroupOrder::Ascending | GroupOrder::Original => (band_min + BAND_SIZE - 1 - group_slot) as i32,
    GroupOrder::Descending => (band_min + group_slot) as i32,
  }
}

#[cfg(test)]
mod tests {
  use super::compute_stream_priority;
  use moqtail::model::control::constant::GroupOrder;

  #[test]
  fn test_highest_priority_near_i32_max() {
    let p = compute_stream_priority(0, 0, GroupOrder::Ascending, 0);
    assert!(
      p > 2_100_000_000,
      "highest priority should be near i32::MAX, got {p}"
    );
  }

  #[test]
  fn test_lowest_priority_near_i32_min() {
    let p = compute_stream_priority(255, 255, GroupOrder::Ascending, 0);
    assert!(
      p < -2_100_000_000,
      "lowest priority should be near i32::MIN, got {p}"
    );
  }

  #[test]
  fn test_ascending_lower_group_higher_priority() {
    let p0 = compute_stream_priority(0, 0, GroupOrder::Ascending, 0);
    let p1 = compute_stream_priority(0, 0, GroupOrder::Ascending, 1);
    assert!(p0 > p1, "group 0 should outrank group 1 in Ascending order");
  }

  #[test]
  fn test_descending_higher_group_higher_priority() {
    let p0 = compute_stream_priority(0, 0, GroupOrder::Descending, 0);
    let p1 = compute_stream_priority(0, 0, GroupOrder::Descending, 1);
    assert!(
      p1 > p0,
      "group 1 should outrank group 0 in Descending order"
    );
  }

  #[test]
  fn test_original_same_as_ascending() {
    for g in [0u64, 1, 100, 65535] {
      assert_eq!(
        compute_stream_priority(10, 20, GroupOrder::Original, g),
        compute_stream_priority(10, 20, GroupOrder::Ascending, g),
        "Original should behave like Ascending for group {g}"
      );
    }
  }

  #[test]
  fn test_subscriber_priority_dominates() {
    // sub=0,pub=255 must outrank sub=1,pub=0 regardless of group
    let high = compute_stream_priority(0, 255, GroupOrder::Ascending, 0);
    let low = compute_stream_priority(1, 0, GroupOrder::Ascending, 0);
    assert!(
      high > low,
      "subscriber priority must dominate publisher priority"
    );
  }

  #[test]
  fn test_publisher_priority_tie_break() {
    let high = compute_stream_priority(10, 0, GroupOrder::Ascending, 0);
    let low = compute_stream_priority(10, 1, GroupOrder::Ascending, 0);
    assert!(high > low, "lower pub_prio number = higher priority");
  }

  #[test]
  fn test_all_values_within_i32_range() {
    for sub in [0u8, 128, 255] {
      for pub_ in [0u8, 128, 255] {
        for &order in &[
          GroupOrder::Ascending,
          GroupOrder::Descending,
          GroupOrder::Original,
        ] {
          for group in [0u64, 1, 65534, 65535, 65536, u64::MAX] {
            let _ = compute_stream_priority(sub, pub_, order, group); // must not panic/overflow
          }
        }
      }
    }
  }
}
