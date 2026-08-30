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
use moqtail::model::control::constant::{GroupOrder, SwitchMode};

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

/// Which Subscription Filter a subscribe uses.
#[derive(Debug, Clone, Copy, PartialEq, ValueEnum)]
pub enum CliFilter {
  /// Objects published from now on
  Latest,
  /// Objects from the start of the next group
  NextGroup,
  /// From an explicit Start Location, filling what is already published
  AbsoluteStartFill,
  /// As above, ending at Start Group plus --end-group-delta
  AbsoluteRangeFill,
  /// From --relative-previous groups before the Largest Object
  RelativeStartFill,
}

/// How a switch stops the subscription it suspends.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum CliSwitchMode {
  /// Stop it at the cutover: Forward State 0 and its streams reset
  Hard,
  /// Let it drain to the group before the new subscription's Start Group
  Soft,
}

impl From<CliSwitchMode> for SwitchMode {
  fn from(mode: CliSwitchMode) -> Self {
    match mode {
      CliSwitchMode::Hard => SwitchMode::Hard,
      CliSwitchMode::Soft => SwitchMode::Soft,
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

  /// Seconds after announcing to withdraw the namespace, by resetting its request
  /// stream while the session stays open (publish-namespace only, 0 = never).
  #[arg(long, default_value_t = 0)]
  pub withdraw_after: u64,

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

  /// Subscription Forward State (subscribe only).
  #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
  pub forward: bool,

  /// Subscription Filter to subscribe with (subscribe only)
  #[arg(long, value_enum, default_value = "latest")]
  pub filter: CliFilter,

  /// Start group for the absolute fill filters (subscribe only)
  #[arg(long, default_value_t = 0)]
  pub filter_start_group: u64,

  /// Start object for the absolute fill filters (subscribe only)
  #[arg(long, default_value_t = 0)]
  pub filter_start_object: u64,

  /// Groups after the Start Group that --filter absolute-range-fill ends at
  #[arg(long, default_value_t = 0)]
  pub end_group_delta: u64,

  /// Groups back from the Largest Object that --filter relative-start-fill starts at
  #[arg(long, default_value_t = 0)]
  pub relative_previous: u64,

  /// Seconds after subscribing to switch to --switch-track (subscribe only, 0 = never)
  #[arg(long, default_value_t = 0)]
  pub switch_after: u64,

  /// Track to switch to with SWITCH_FROM (subscribe + --switch-after only)
  #[arg(long)]
  pub switch_track: Option<String>,

  /// How the switch stops the subscription it suspends
  #[arg(long, value_enum, default_value = "hard")]
  pub switch_mode: CliSwitchMode,

  /// Whether the switch asks for PUBLISH_DONE on the suspended subscription
  #[arg(long, default_value_t = false)]
  pub switch_publish_done: bool,

  /// Seconds after subscribing to send a REQUEST_UPDATE setting Forward State 1
  /// (subscribe only, 0 = never). Use with --forward false to test that delivery
  /// resumes after Forward flips 0->1.
  #[arg(long, default_value_t = 0)]
  pub update_forward_after: u64,
}
