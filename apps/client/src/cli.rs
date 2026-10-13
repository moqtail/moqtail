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

use crate::filter_spec::{
  ObjectPropertySpec, parse_object_property, parse_range_filter, parse_track_property,
};
use clap::{Parser, ValueEnum};
use moqtail::model::control::constant::{GroupOrder, SwitchMode};
use moqtail::model::parameter::message_parameter::MessageParameter;
use moqtail::model::property::track_property::TrackProperty;

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

/// Which Location Filter a subscribe uses.
#[derive(Debug, Clone, Copy, PartialEq, ValueEnum)]
pub enum CliFilter {
  /// No filter: every Object
  None,
  /// From --filter-start-group groups before the next group (0 = the next group, 1 = the current one)
  RelativeStart,
  /// From --filter-start-group/--filter-start-object
  AbsoluteStart,
  /// As above, ending with group Start Group plus --end-group-delta
  AbsoluteStartGroupEnd,
  /// As above, ending with Object --filter-end-object of that group
  AbsoluteRange,
  /// Objects published from now on
  NextObject,
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

/// Which of the two prefix subscriptions a request is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum PrefixKind {
  /// Namespace discovery: NAMESPACE and NAMESPACE_DONE come back on the stream
  Namespace,
  /// Track delivery: a PUBLISH per matching track comes back on its own stream
  Tracks,
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
  /// Discover the namespaces under a prefix (SUBSCRIBE_NAMESPACE)
  SubscribeNamespace,
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
  /// Command to run (publish, publish-namespace, subscribe, subscribe-namespace,
  /// subscribe-tracks, or fetch)
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

  /// Subgroups each group is split into, Object i going to subgroup i mod N, each
  /// on its own stream (publish only)
  #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u64).range(1..))]
  pub subgroups_per_group: u64,

  /// Publisher priority of each subgroup in turn, cycling: 0,64,128 gives subgroup 0
  /// priority 0, subgroup 1 priority 64 and so on; a datagram takes the one for the
  /// subgroup it would have gone to (publish only; default --publisher-priority)
  #[arg(long, value_delimiter = ',')]
  pub priorities: Vec<u8>,

  /// An Object Property to set on every Object, as TYPE=SOURCE; repeatable (publish
  /// only). TYPE is even; SOURCE is group, object, subgroup, mod:M (the Object ID
  /// modulo M) or a number. Example: 0x3c=mod:3
  #[arg(long, value_parser = parse_object_property)]
  pub object_property: Vec<ObjectPropertySpec>,

  /// A Track Property to advertise in PUBLISH, SUBSCRIBE_OK and FETCH_OK, as
  /// TYPE=VALUE with TYPE even; repeatable (publish only). Example: 0x3c=1
  #[arg(long, value_parser = parse_track_property)]
  pub track_property: Vec<TrackProperty>,

  /// Track alias (publish only, random if not specified)
  #[arg(long)]
  pub track_alias: Option<u64>,

  /// Seconds after announcing to withdraw the namespace, by resetting its request
  /// stream while the session stays open (publish-namespace only, 0 = never).
  #[arg(long, default_value_t = 0)]
  pub withdraw_after: u64,

  /// Duration to listen in seconds, 0 = indefinite (subscribe and the prefix
  /// subscriptions). A prefix subscription cancels itself when it elapses, by
  /// resetting its request stream while the session stays open.
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

  /// Last object ID of the end group, inclusive (fetch only)
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

  /// Subscription Forward State (subscribe, and each track subscribe-tracks brings).
  #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
  pub forward: bool,

  /// Whether the response should carry Track Properties; false sends
  /// INCLUDE_PROPERTIES=0 (subscribe, subscribe-tracks and fetch)
  #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
  pub include_properties: bool,

  /// Location Filter to subscribe with (subscribe: defaults to next-object;
  /// subscribe-tracks: sent only when given, as where each pushed track starts)
  #[arg(long, value_enum)]
  pub filter: Option<CliFilter>,

  /// Start group for the absolute filters, or groups back from the next group for
  /// --filter relative-start (subscribe and subscribe-tracks)
  #[arg(long, default_value_t = 0)]
  pub filter_start_group: u64,

  /// Start object for the absolute filters (subscribe and subscribe-tracks)
  #[arg(long, default_value_t = 0)]
  pub filter_start_object: u64,

  /// Groups after the Start Group that the absolute filters with an end end at
  #[arg(long, default_value_t = 0)]
  pub end_group_delta: u64,

  /// Last Object of the End Group for --filter absolute-range
  #[arg(long, default_value_t = 0)]
  pub filter_end_object: u64,

  /// Also retrieve what is already published over --filter's range, on a fill
  /// fetch stream (subscribe, and each track subscribe-tracks brings)
  #[arg(long, default_value_t = false)]
  pub fill: bool,

  /// Fill from this many groups before the next group instead of over --filter's
  /// range (1 = the current group); implies --fill (subscribe and subscribe-tracks)
  #[arg(long)]
  pub fill_start_group: Option<u64>,

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

  /// Start group the switch asks to begin at, to switch at a future group
  /// boundary rather than at the next one (0 = subscribe with --filter instead)
  #[arg(long, default_value_t = 0)]
  pub switch_start_group: u64,

  /// A Range Filter to request with; repeatable (subscribe, fetch and
  /// subscribe-tracks). Every object received is checked against the filters, and
  /// one that does not pass is reported as a filter violation.
  ///
  /// KIND:SET[:PROPERTY]:RANGES, or KIND:clear. KIND is subgroup, object, priority,
  /// object-property or track-property; SET is the SetID (0-255); PROPERTY is the
  /// Property Type, for the two property kinds only; RANGES is a comma-separated
  /// list of A-B, A- (open-ended, last only) or A, in decimal or 0x hex. Examples:
  /// object:0:0-2,5-  object-property:1:0x3c:10-20  priority:clear
  #[arg(long, value_parser = parse_range_filter)]
  pub range_filter: Vec<MessageParameter>,

  /// A Range Filter for the fill fetch stream, inside FILL_PARAMETERS; repeatable.
  /// Implies --fill (subscribe and subscribe-tracks)
  #[arg(long, value_parser = parse_range_filter)]
  pub fill_range_filter: Vec<MessageParameter>,

  /// Seconds after subscribing to send a REQUEST_UPDATE carrying
  /// --update-range-filter (subscribe only, 0 = never)
  #[arg(long, default_value_t = 0)]
  pub update_filters_after: u64,

  /// A Range Filter for that REQUEST_UPDATE; repeatable. KIND:clear removes a kind
  /// (subscribe only)
  #[arg(long, value_parser = parse_range_filter)]
  pub update_range_filter: Vec<MessageParameter>,

  /// Send Range Filters even past the server's MAX_FILTER_RANGES (0 when it sent
  /// none), to test that the server refuses them. Without this the client refuses
  /// to send them.
  #[arg(long, default_value_t = false)]
  pub ignore_filter_limit: bool,

  /// Seconds after subscribing to send a REQUEST_UPDATE setting Forward State 1
  /// (subscribe only, 0 = never). Use with --forward false to test that delivery
  /// resumes after Forward flips 0->1.
  #[arg(long, default_value_t = 0)]
  pub update_forward_after: u64,

  /// Once the prefix subscription is live, issue a second one of this kind on the
  /// same prefix and the same session, and log what comes back (subscribe-namespace
  /// and subscribe-tracks only). Overlap is judged per session, so this is what
  /// shows whether two prefix subscriptions are allowed to share a prefix: the two
  /// kinds keep separate spaces and both succeed, while a second of the same kind
  /// is refused with PREFIX_OVERLAP.
  #[arg(long, value_enum)]
  pub overlap: Option<PrefixKind>,
}
