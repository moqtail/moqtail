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

//! The API's side of [`moqtail_probe::Probe`]: what it keeps of the events the
//! relay core reports.

use moqtail_probe::{ConnectionInfo, Probe};
use std::collections::VecDeque;
use std::sync::Mutex;

pub(crate) struct ApiProbe {
  /// Whether the `state` feature is on; without it nothing here is ever read.
  keep_state: bool,
  closed_capacity: usize,
  /// The most recently closed connections, oldest first.
  closed: Mutex<VecDeque<ConnectionInfo>>,
}

impl ApiProbe {
  pub fn new(keep_state: bool, closed_capacity: usize) -> Self {
    ApiProbe {
      keep_state,
      closed_capacity,
      closed: Mutex::new(VecDeque::new()),
    }
  }

  /// The closed connections, newest first.
  pub fn closed_connections(&self) -> Vec<ConnectionInfo> {
    self.closed.lock().unwrap().iter().rev().cloned().collect()
  }
}

impl Probe for ApiProbe {
  fn keeps_closed_connections(&self) -> bool {
    self.keep_state && self.closed_capacity > 0
  }

  fn connection_closed(&self, connection: ConnectionInfo) {
    let mut closed = self.closed.lock().unwrap();
    if closed.len() == self.closed_capacity {
      closed.pop_front();
    }
    closed.push_back(connection);
  }
}
