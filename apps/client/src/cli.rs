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

use clap::{Parser, ValueEnum};
use moqtail::model::control::constant::{FetchType, GroupOrder};

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum CliGroupOrder {
  Original,
  Ascending,
  Descending,
}

impl From<CliGroupOrder> for GroupOrder {
  fn from(o: CliGroupOrder) -> Self {
    match o {
      CliGroupOrder::Original => GroupOrder::Original,
      CliGroupOrder::Ascending => GroupOrder::Ascending,
      CliGroupOrder::Descending => GroupOrder::Descending,
    }
  }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum CliJoiningType {
  /// Joining Fetch relative to the Largest group (start = largest - joining_start)
  Relative,
  /// Absolute Joining Fetch (start = joining_start)
  Absolute,
}

impl From<CliJoiningType> for FetchType {
  fn from(t: CliJoiningType) -> Self {
    match t {
      CliJoiningType::Relative => FetchType::RelativeFetch,
      CliJoiningType::Absolute => FetchType::AbsoluteFetch,
    }
  }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum DeliveryMode {
  /// Send/receive objects via unidirectional streams with subgroup headers
  Subgroup,
  /// Send/receive objects via QUIC datagrams
  Datagram,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum Transport {
  /// WebTransport over HTTP/3 (derives an https:// URI from the moqt:// URL)
  WebTransport,
  /// Native QUIC (carries authority/path in Setup Options)
  Quic,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum Command {
  /// Publish objects to a track
  Publish,
  /// Publish a namespace and auto-respond to subscribes with test data
  PublishNamespace,
  /// Subscribe to a track and receive objects
  Subscribe,
  /// Subscribe to all tracks under a namespace prefix (SUBSCRIBE_TRACKS)
  SubscribeTracks,
  /// Fetch specific object ranges from a track
  Fetch,
}

#[derive(Parser, Debug)]
#[command(
  name = "moqtail-client",
  author,
  version,
  about = "MOQtail test client"
)]
pub struct Cli {
  /// Command to run (publish, publish-namespace, subscribe, or fetch)
  #[arg(long, short, value_enum)]
  pub command: Command,

  /// Server address (moqt:// URI, e.g. moqt://127.0.0.1:4433/path#type:value)
  #[arg(long, short, default_value = "moqt://127.0.0.1:4433")]
  pub server: String,

  /// Transport to establish the moqt:// session over
  #[arg(long, value_enum, default_value = "web-transport")]
  pub transport: Transport,

  /// Track namespace
  #[arg(long, short, default_value = "moqtail")]
  pub namespace: String,

  /// Track name
  #[arg(long, short = 'T', default_value = "demo")]
  pub track_name: String,

  /// Skip certificate validation (for testing with self-signed certs)
  #[arg(long, default_value_t = false)]
  pub no_cert_validation: bool,

  /// Delivery mode: how objects are sent/received (subgroup streams or QUIC datagrams)
  #[arg(long, value_enum, default_value = "subgroup")]
  pub delivery_mode: DeliveryMode,

  /// Number of groups to send (publish only)
  #[arg(long, default_value_t = 100)]
  pub group_count: u64,

  /// Interval between objects in milliseconds (publish only)
  #[arg(long, short, default_value_t = 1000)]
  pub interval: u64,

  /// Number of objects per group (publish only)
  #[arg(long, default_value_t = 10)]
  pub objects_per_group: u64,

  /// Stride between object IDs within a group (publish only). >1 leaves gaps in
  /// the object IDs (e.g. 2 emits 0, 2, 4) and sets the Prior Object ID Gap
  /// property so gaps are communicated explicitly.
  #[arg(long, default_value_t = 1)]
  pub object_id_step: u64,

  /// Stride between group IDs (publish only). >1 leaves gaps in the group IDs
  /// (e.g. 2 emits 0, 2, 4) and sets the Prior Group ID Gap property
  /// so gaps are communicated explicitly.
  #[arg(long, default_value_t = 1)]
  pub group_id_step: u64,

  /// Payload size in bytes (publish only)
  #[arg(long, default_value_t = 1200)]
  pub payload_size: usize,

  /// Track alias (publish only, random if not specified)
  #[arg(long)]
  pub track_alias: Option<u64>,

  /// Duration to listen in seconds, 0 = indefinite (subscribe only)
  #[arg(long, short, default_value_t = 0)]
  pub duration: u64,

  /// Start group ID (fetch only)
  #[arg(long, default_value_t = 1)]
  pub start_group: u64,

  /// Start object ID (fetch only)
  #[arg(long, default_value_t = 0)]
  pub start_object: u64,

  /// End group ID (fetch only)
  #[arg(long, default_value_t = 5)]
  pub end_group: u64,

  /// End object ID (fetch only)
  #[arg(long, default_value_t = 3)]
  pub end_object: u64,

  /// Cancel the fetch after receiving N objects (fetch only, 0 = no cancel)
  #[arg(long, default_value_t = 0)]
  pub cancel_after: u64,

  /// Subscriber priority 0 (highest) – 255 (lowest) (subscribe only)
  #[arg(long, default_value_t = 128)]
  pub subscriber_priority: u8,

  /// Publisher priority 0 (highest) – 255 (lowest) (publish only)
  #[arg(long, default_value_t = 128)]
  pub publisher_priority: u8,

  /// Group order for the track (subscribe only)
  #[arg(long, value_enum, default_value = "ascending")]
  pub group_order: CliGroupOrder,

  /// Additional track to subscribe to for priority testing: "track-name:priority"
  /// e.g. --extra-track demo2:200
  #[arg(long)]
  pub extra_track: Option<String>,

  /// Subscription Forward State (subscribe only). Set false to test that a
  /// joining FETCH against a non-forwarding subscription is rejected.
  #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
  pub forward: bool,

  /// Seconds after subscribing to send a REQUEST_UPDATE setting Forward State 1
  /// (subscribe only, 0 = never). Use with --forward false to test that delivery
  /// resumes after Forward flips 0->1.
  #[arg(long, default_value_t = 0)]
  pub update_forward_after: u64,

  /// After subscribing, issue a Joining FETCH referencing the subscription
  /// (subscribe only).
  #[arg(long, default_value_t = false)]
  pub joining_fetch: bool,

  /// Joining FETCH start group (subscribe + --joining-fetch only)
  #[arg(long, default_value_t = 0)]
  pub joining_start: u64,

  /// Joining FETCH type (subscribe + --joining-fetch only)
  #[arg(long, value_enum, default_value = "relative")]
  pub joining_type: CliJoiningType,
}
