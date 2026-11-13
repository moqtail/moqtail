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

//! The `api:` section of the relay's configuration file.
//!
//! The relay reads it with the rest of its settings and hands it to the API as it
//! is, so the two never copy fields between each other.

use serde::{Deserialize, Serialize};
use smart_default::SmartDefault;
use std::net::SocketAddr;
use std::time::Duration;

/// Settings of the HTTP API and the measurements behind it. YAML keys are the
/// field names under `api:`; a key left out takes the field's default, and an
/// unknown key is a startup error.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, SmartDefault)]
#[serde(default, deny_unknown_fields)]
pub struct ApiSettings {
  /// Starts the HTTP API under `/api/v1`.
  pub enable: bool,
  /// Address of the API. Unset means `127.0.0.1:4480`.
  pub bind: Option<SocketAddr>,
  /// Parts of the API that run: health, state, metrics, latency, qlog, events,
  /// actions, mcp. Unset means health, state, metrics, latency and qlog.
  pub features: Option<Vec<String>>,
  /// Client address ranges allowed on a network `bind`. Unset means loopback only;
  /// ignored on a loopback `bind`.
  pub allow: Option<Vec<String>>,
  /// When set, API reads need it as a bearer token, except health, readiness and
  /// `/api/v1/meta`.
  #[serde(skip_serializing)]
  pub read_token: Option<String>,
  /// Needed for the API's write endpoints; also allows reads.
  #[serde(skip_serializing)]
  pub write_token: Option<String>,
  /// Browser origins allowed to call the API, such as the dashboard's.
  pub cors_origins: Vec<String>,
  /// A second listener serving only `/api/v1/metrics`, for example to open the
  /// metrics to a monitoring network while the rest of the API stays on loopback.
  pub metrics_bind: Option<SocketAddr>,
  /// A token that reads `/api/v1/metrics` and nothing else, for Prometheus.
  #[serde(skip_serializing)]
  pub metrics_token: Option<String>,
  /// The MCP token's salted hash (`sha256d$...`), as `relay api-token` prints it.
  /// The only credential `/api/v1/mcp` accepts. Shown in `/api/v1/config`, since
  /// the hash does not let anyone in.
  pub mcp_token_hash: Option<String>,
  /// Lets MCP clients use the write tools (GOAWAY, close, drain, log filter,
  /// redirect URI). Needs the `actions` feature too.
  pub mcp_write: bool,
  /// How the relay introduces itself to MCP clients, such as `interop-paris`, so an
  /// agent connected to several relays can tell them apart. Unset means the host
  /// name.
  pub mcp_name: Option<String>,
  /// Anything the operator wants apps to read from `/api/v1/meta`, such as the
  /// relay's public host name or region. Served as written, without a token, so
  /// put nothing secret here.
  pub meta: serde_json::Map<String, serde_json::Value>,
  /// Closed connections the API keeps a summary of, oldest dropped first.
  #[default(10_000)]
  pub closed_connection_history: usize,
  /// How finely object processing time is kept: relay, track or subscription.
  #[default("subscription")]
  pub latency_detail: String,
  /// Measure the processing time of every Nth object only; 1 measures all of them.
  #[default(1)]
  pub latency_sample: u64,
  /// Deliveries slower than this, from arrival to written, go to the slowlog.
  #[default(Duration::from_millis(20))]
  #[serde(with = "humantime_serde")]
  pub slowlog_threshold: Duration,
  /// Subscriptions waiting longer than this for their first object go to the slowlog.
  #[default(Duration::from_secs(1))]
  #[serde(with = "humantime_serde")]
  pub slowlog_first_object_threshold: Duration,
  /// Slowlog entries kept.
  #[default(1000)]
  pub slowlog_size: usize,
  /// Namespace prefixes (`a/b`) whose traffic and requests are counted separately;
  /// a namespace belongs to the longest prefix it falls under.
  pub accounting_zones: Vec<String>,
  /// Publish the relay's own metrics as MoQ tracks under
  /// `metrics.moq.arpa/v1/<metrics_resource_id>`. Needs `enable`.
  pub metrics_track_enable: bool,
  /// 16 lowercase hexadecimal characters naming this relay in the metrics
  /// namespace; unset makes one up each time the relay starts.
  pub metrics_resource_id: Option<String>,
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn an_empty_section_takes_the_defaults_and_unknown_keys_are_refused() {
    let empty: ApiSettings = serde_json::from_str("{}").unwrap();
    assert_eq!(empty, ApiSettings::default());
    assert_eq!(empty.closed_connection_history, 10_000);
    assert_eq!(empty.slowlog_threshold, Duration::from_millis(20));
    assert!(serde_json::from_str::<ApiSettings>(r#"{"enabled": true}"#).is_err());
  }

  #[test]
  fn tokens_are_never_written_out() {
    let settings = ApiSettings {
      read_token: Some("r3ad".into()),
      write_token: Some("wr1te".into()),
      metrics_token: Some("m3trics".into()),
      ..Default::default()
    };
    let json = serde_json::to_string(&settings).unwrap();
    assert!(!json.contains("r3ad") && !json.contains("wr1te") && !json.contains("m3trics"));
  }
}
