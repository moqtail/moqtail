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
use serde::{Deserialize, Serialize};

/// Top-level Request Enum (Driver -> Client)
#[derive(Deserialize, Debug)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum RpcRequest {
  /// Initiate QUIC connection
  Connect { params: ConnectParams, id: u64 },
  /// Disconnect and shutdown
  Disconnect { id: u64 },
  /// Request to receive a track (Draft-16 Section 9.9)
  Subscribe { params: SubscribeParams, id: u64 },
  /// This acts as a Publisher-initiated subscription.
  Publish { params: PublishParams, id: u64 },
  /// Update parameters of an existing subscription (Draft-16 Section 9.11)
  UpdateSubscription {
    params: UpdateSubscriptionParams,
    id: u64,
  },
  /// Stop receiving a track (Draft-16 Section 9.12)
  Unsubscribe { params: UnsubscribeParams, id: u64 },
  /// Retrieve past objects (Draft-16 Section 9.16)
  Fetch { params: FetchParams, id: u64 },
  /// Cancel a fetch request (Draft-16 Section 9.18)
  FetchCancel { params: FetchCancelParams, id: u64 },
  PublishNamespace {
    params: PublishNamespaceParams,
    id: u64,
  },

  /// Stop advertising a namespace (Draft-16 Section 9.24)
  UnpublishNamespace {
    params: UnpublishNamespaceParams,
    id: u64,
  },
  /// Ask relay for available tracks (Draft-16 Section 9.25)
  SubscribeNamespace {
    params: SubscribeNamespaceParams,
    id: u64,
  },
  /// Check if a track exists without subscribing (Draft-16 Section 9.19)
  TrackStatus { params: TrackStatusParams, id: u64 },
}

// --- Parameter Structs ---

#[derive(Debug, Deserialize)]
pub struct ConnectParams {
  pub url: String,
  // Optional role override (subscriber, publisher, both)
  pub role: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SubscribeParams {
  pub namespace: String,
  pub track: String,

  // --- Draft-16 Optional Parameters ---
  /// Priority (0-255), lower is better. Default: 128
  pub priority: Option<u8>,

  /// "ascending" or "descending". Default: "ascending"
  pub group_order: Option<String>,

  /// "latest_object", "absolute_start", "absolute_range"
  pub filter_type: Option<String>,

  /// Used if filter_type is absolute_start/range
  pub start_group: Option<u64>,
  pub start_object: Option<u64>,

  /// Used if filter_type is absolute_range
  pub end_group: Option<u64>,

  /// Auth token string (Section 9.2.2.1)
  pub authorization_token: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PublishParams {
  pub namespace: String,
  pub track: String,
  pub start_group: u64,
  pub start_object: u64,
  pub end_group: Option<u64>, // Optional limit for the blaster
  pub priority: Option<u8>,
  pub authorizaton_token: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateSubscriptionParams {
  pub subscription_id: u64,
  pub priority: Option<u8>,
  pub end_group: Option<u64>,
  pub end_object: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct UnsubscribeParams {
  pub subscription_id: u64,
}

#[derive(Debug, Deserialize)]
pub struct FetchParams {
  pub namespace: String,
  pub track: String,

  pub start_group: u64,
  pub start_object: u64,

  /// If end_object is 0, it means "end of group" usually, but strict mapping required
  pub end_group: u64,
  pub end_object: u64,

  pub priority: Option<u8>,
}

#[derive(Debug, Deserialize)]
pub struct FetchCancelParams {
  pub fetch_id: u64,
}

#[derive(Debug, Deserialize)]
pub struct PublishNamespaceParams {
  pub namespace: String,
  pub authorization_token: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UnpublishNamespaceParams {
  pub namespace: String,
}

#[derive(Debug, Deserialize)]
pub struct SubscribeNamespaceParams {
  pub namespace_prefix: String,
  pub authorization_token: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct TrackStatusParams {
  pub namespace: String,
  pub track: String,
}

// --- Response & Notifications (Client -> Driver) ---

#[derive(Serialize, Debug)]
pub struct RpcResponse {
  pub jsonrpc: String,
  pub result: serde_json::Value,
  pub id: u64,
}

impl RpcResponse {
  pub fn success(id: u64, value: serde_json::Value) -> Self {
    Self {
      jsonrpc: "2.0".to_string(),
      result: value,
      id,
    }
  }
}

#[derive(Serialize, Debug)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum RpcNotification {
  Log { params: LogParams },
  OnStatUpdate { params: StatParams },

  // Notifications for received network events
  OnPeerSubscribe { params: PeerSubscribeParams },
  OnPeerUnsubscribe { params: PeerUnsubscribeParams },
  OnPeerPublishNamespace { params: PeerPublishNamespaceParams },
  OnGoAway { params: GoAwayParams },
  OnPeerPublish { params: PeerPublishParams },
  OnPeerNamespace { params: PeerNamespaceParams },
}

#[derive(Debug, Serialize)]
pub struct LogParams {
  pub message: String,
  pub level: String,
}

#[derive(Debug, Serialize)]
pub struct StatParams {
  pub object_size: usize,
  pub group_id: u64,
  pub object_id: u64,
  // Optional: add cache hit/miss status for Relays later
}

#[derive(Debug, Serialize)]
pub struct PeerSubscribeParams {
  pub subscribe_id: u64,
  pub namespace: String,
  pub track: String,
}

#[derive(Debug, Serialize)]
pub struct PeerUnsubscribeParams {
  pub subscribe_id: u64,
}
#[derive(Debug, Serialize)]
pub struct PeerPublishNamespaceParams {
  pub namespace: String,
}

#[derive(Debug, Serialize)]
pub struct GoAwayParams {
  pub new_session_uri: String,
}

#[derive(Debug, Serialize)]
pub struct PeerPublishParams {
  pub namespace: String,
  pub track: String,
  pub start_group: u64,
  pub start_object: u64,
}

#[derive(Debug, Serialize)]
pub struct PeerNamespaceParams {
  pub prefix: String,
  pub matched_namespace: String,
}
