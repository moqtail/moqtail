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

//! The API's side of [`relay_hooks::Reporter`]: what it keeps of the events the
//! relay core reports.

use crate::control::{ConnectionCounts, ControlTracker};
use crate::latency::Latency;
use crate::zones::Zones;
use relay_hooks::stats::{SubscriptionStats, ZoneCounters};
use relay_hooks::{ConnectionInfo, Reporter};
use serde::Serialize;
use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};

/// A connection with what the API counted of its control messages and requests.
#[derive(Debug, Clone, Serialize)]
pub struct Connection {
  #[serde(flatten)]
  pub info: ConnectionInfo,
  #[serde(flatten)]
  pub counts: ConnectionCounts,
}

pub(crate) struct ApiReporter {
  /// Whether the `state` feature is on; without it nothing here is ever read.
  keep_state: bool,
  closed_capacity: usize,
  /// The most recently closed connections, oldest first.
  closed: Mutex<VecDeque<Connection>>,
  control: Arc<ControlTracker>,
  /// `None` unless the `latency` feature is on.
  pub latency: Option<Arc<Latency>>,
  /// Connection failures by reason, limit rejections by limit, and closes by who
  /// closed and the code, since the relay started.
  pub failures: Mutex<BTreeMap<&'static str, u64>>,
  pub limit_rejections: Mutex<BTreeMap<&'static str, u64>>,
  pub closes: Mutex<BTreeMap<(String, String), u64>>,
  zones: Arc<Zones>,
}

impl ApiReporter {
  pub fn new(
    keep_state: bool,
    closed_capacity: usize,
    control: Arc<ControlTracker>,
    latency: Option<Arc<Latency>>,
    zones: Arc<Zones>,
  ) -> Self {
    ApiReporter {
      keep_state,
      closed_capacity,
      closed: Mutex::new(VecDeque::new()),
      control,
      latency,
      failures: Mutex::default(),
      limit_rejections: Mutex::default(),
      closes: Mutex::default(),
      zones,
    }
  }

  /// The closed connections, newest first.
  pub fn closed_connections(&self) -> Vec<Connection> {
    self.closed.lock().unwrap().iter().rev().cloned().collect()
  }
}

impl Reporter for ApiReporter {
  fn keeps_closed_connections(&self) -> bool {
    // Also when no history is kept: the control counts of a closed connection are
    // released here.
    self.keep_state
  }

  fn connection_closed(&self, info: ConnectionInfo) {
    let counts = self.control.connection_closed(info.conn_id);
    if let Some(close) = &info.close {
      let code = close
        .code
        .map_or_else(|| "none".to_string(), |c| c.to_string());
      *self
        .closes
        .lock()
        .unwrap()
        .entry((close.by.clone(), code))
        .or_default() += 1;
    }
    if self.closed_capacity == 0 {
      return;
    }
    let mut closed = self.closed.lock().unwrap();
    if closed.len() == self.closed_capacity {
      closed.pop_front();
    }
    closed.push_back(Connection { info, counts });
  }

  fn subscription_stats(
    &self,
    track_id: u64,
    conn_id: u64,
    request_id: u64,
  ) -> Option<Arc<SubscriptionStats>> {
    let latency = self.latency.as_ref()?;
    Some(latency.subscription_stats(track_id, conn_id, request_id))
  }

  fn zone(&self, namespace: &str) -> Option<Arc<ZoneCounters>> {
    (!self.zones.is_empty()).then(|| self.zones.traffic_for(namespace))
  }

  fn connection_failed(&self, reason: &'static str) {
    *self.failures.lock().unwrap().entry(reason).or_default() += 1;
  }

  fn limit_rejected(&self, limit: &'static str) {
    *self
      .limit_rejections
      .lock()
      .unwrap()
      .entry(limit)
      .or_default() += 1;
  }
}
