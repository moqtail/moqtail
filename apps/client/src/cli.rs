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

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ForwardingPreference {
  /// Send objects via unidirectional streams with subgroup headers
  Subgroup,
  /// Send objects via QUIC datagrams
  Datagram,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum Command {
  /// Publish objects to a track
  Publish,
  /// Publish a namespace and auto-respond to subscribes with test data
  PublishNamespace,
  /// Subscribe to a track and receive objects
  Subscribe,
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

  /// Server address
  #[arg(long, short, default_value = "https://127.0.0.1:4433")]
  pub server: String,

  /// Track namespace
  #[arg(long, short, default_value = "moqtail")]
  pub namespace: String,

  /// Track name
  #[arg(long, short = 'T', default_value = "demo")]
  pub track_name: String,

  /// Skip certificate validation (for testing with self-signed certs)
  #[arg(long, default_value_t = false)]
  pub no_cert_validation: bool,

  /// Forwarding preference (subgroup, datagram)
  #[arg(long, value_enum, default_value = "subgroup")]
  pub forwarding_preference: ForwardingPreference,

  /// Number of groups to send (publish only)
  #[arg(long, default_value_t = 100)]
  pub group_count: u64,

  /// Interval between objects in milliseconds (publish only)
  #[arg(long, short, default_value_t = 1000)]
  pub interval: u64,

  /// Number of objects per group (publish only)
  #[arg(long, default_value_t = 10)]
  pub objects_per_group: u64,

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
}
