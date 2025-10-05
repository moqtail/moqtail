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
  pub track_alias: u64,
  pub group_id: Option<u64>,
  pub subgroup_id: Option<u64>,
  pub fetch_request_id: Option<u64>,
}

impl StreamId {
  fn new(
    stream_type: StreamType,
    track_alias: u64,
    group_id: Option<u64>,
    subgroup_id: Option<u64>,
    fetch_request_id: Option<u64>,
  ) -> Self {
    Self {
      stream_type,
      track_alias,
      group_id,
      subgroup_id,
      fetch_request_id,
    }
  }

  pub fn new_fetch(track_alias: u64, fetch_request_id: u64) -> Self {
    Self::new(
      StreamType::Fetch,
      track_alias,
      None,
      None,
      Some(fetch_request_id),
    )
  }

  pub fn new_subgroup(track_alias: u64, group_id: u64, subgroup_id: Option<u64>) -> Self {
    Self::new(
      StreamType::Subgroup,
      track_alias,
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
        self.track_alias,
        self.fetch_request_id.unwrap_or(0),
      ),

      StreamType::Subgroup => write!(
        f,
        "subgroup_{}_{}_{}",
        self.track_alias,
        self.group_id.unwrap(),
        self.subgroup_id.unwrap_or(0),
      ),
    }
  }
}
