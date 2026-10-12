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

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum StreamType {
  Fetch,
  Subgroup, // stream number is passed inside
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StreamId {
  pub stream_type: StreamType,
  pub relay_track_id: u64,
  pub group_id: Option<u64>,
  pub subgroup_id: Option<u64>,
  pub fetch_request_id: Option<u64>,
}

impl StreamId {
  fn new(
    stream_type: StreamType,
    relay_track_id: u64,
    group_id: Option<u64>,
    subgroup_id: Option<u64>,
    fetch_request_id: Option<u64>,
  ) -> Self {
    Self {
      stream_type,
      relay_track_id,
      group_id,
      subgroup_id,
      fetch_request_id,
    }
  }

  pub fn new_fetch(relay_track_id: u64, fetch_request_id: u64) -> Self {
    Self::new(
      StreamType::Fetch,
      relay_track_id,
      None,
      None,
      Some(fetch_request_id),
    )
  }

  pub fn new_subgroup(relay_track_id: u64, group_id: u64, subgroup_id: Option<u64>) -> Self {
    Self::new(
      StreamType::Subgroup,
      relay_track_id,
      Some(group_id),
      subgroup_id,
      None,
    )
  }

  pub fn get_stream_id(&self) -> String {
    self.to_string()
  }
}

impl fmt::Display for StreamId {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    match self.stream_type {
      StreamType::Fetch => write!(
        f,
        "fetch_{}_{}",
        self.relay_track_id,
        self.fetch_request_id.unwrap_or(0),
      ),

      // This string keys the send streams, so an absent Subgroup ID must not read
      // as any real one: rendering it as 0 silently merged every subgroup whose
      // header defers its ID onto the stream belonging to subgroup 0.
      StreamType::Subgroup => match self.subgroup_id {
        Some(subgroup_id) => write!(
          f,
          "subgroup_{}_{}_{}",
          self.relay_track_id,
          self.group_id.unwrap(),
          subgroup_id,
        ),
        None => write!(
          f,
          "subgroup_{}_{}_unresolved",
          self.relay_track_id,
          self.group_id.unwrap(),
        ),
      },
    }
  }
}

/// Concurrent subscriptions for the same track requires
/// The client must use a composite key that contains the request id and stream_id
/// to avoid stream id collisions because of concurrent subscriptions
/// for the same track.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SendStreamKey {
  pub request_id: u64,
  pub stream_id: StreamId,
}

impl SendStreamKey {
  pub fn new(request_id: u64, stream_id: StreamId) -> Self {
    Self {
      request_id,
      stream_id,
    }
  }
}

impl fmt::Display for SendStreamKey {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    write!(f, "req_{}_{}", self.request_id, self.stream_id)
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::collections::HashMap;

  #[test]
  fn two_subscriptions_to_one_track_get_their_own_send_stream() {
    let stream_id = StreamId::new_subgroup(7, 3, Some(0));
    let mut streams = HashMap::new();
    streams.insert(SendStreamKey::new(2, stream_id.clone()), "first");
    streams.insert(SendStreamKey::new(4, stream_id.clone()), "second");

    assert_eq!(streams.len(), 2);
    assert_eq!(streams[&SendStreamKey::new(2, stream_id.clone())], "first");
    assert_eq!(streams[&SendStreamKey::new(4, stream_id)], "second");
  }

  #[test]
  fn a_send_stream_key_names_its_request() {
    let key = SendStreamKey::new(4, StreamId::new_subgroup(7, 3, Some(0)));
    assert_eq!(key.to_string(), "req_4_subgroup_7_3_0");
  }
}
