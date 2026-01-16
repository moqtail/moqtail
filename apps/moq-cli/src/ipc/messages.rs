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

#[derive(Deserialize, Debug)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum RpcRequest {
  Connect { params: ConnectParams, id: u64 },
  Subscribe { params: SubscribeParams, id: u64 },
  Unsubscribe { params: UnsubscribeParams, id: u64 },
  // PublishTrack { params: PublishParams, id: u64 },
  // ...
}

#[derive(Debug, Deserialize)]
pub struct ConnectParams {
  pub url: String,
}

#[derive(Debug, Deserialize)]
pub struct SubscribeParams {
  pub namespace: String,
  pub track: String,
}

#[derive(Debug, Deserialize)]
pub struct UnsubscribeParams {
  pub subscription_id: u64,
}

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
}
