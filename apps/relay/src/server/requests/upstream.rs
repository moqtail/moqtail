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

use std::collections::BTreeMap;
use std::sync::Arc;

use moqtail::transport::data_stream_handler::SubscribeRequest;
use tokio::sync::{RwLock, mpsc};

use crate::server::session::session_context::UpstreamFetchEvent;

/// The requests the relay has sent upstream, by the relay's request id: each
/// SUBSCRIBE it forwarded, and where each FETCH's response goes.
#[derive(Clone, Debug, Default)]
pub struct UpstreamRequests {
  inner: Arc<RwLock<Upstream>>,
}

#[derive(Debug, Default)]
struct Upstream {
  subscribes: BTreeMap<u64, SubscribeRequest>,
  fetches: BTreeMap<u64, mpsc::Sender<UpstreamFetchEvent>>,
}

impl UpstreamRequests {
  pub async fn add_subscribe(&self, request_id: u64, request: SubscribeRequest) {
    self
      .inner
      .write()
      .await
      .subscribes
      .insert(request_id, request);
  }

  pub async fn get_subscribe(&self, request_id: u64) -> Option<SubscribeRequest> {
    self.inner.read().await.subscribes.get(&request_id).cloned()
  }

  pub async fn remove_subscribe(&self, request_id: u64) -> Option<SubscribeRequest> {
    self.inner.write().await.subscribes.remove(&request_id)
  }

  pub async fn add_fetch(&self, request_id: u64, sender: mpsc::Sender<UpstreamFetchEvent>) {
    self.inner.write().await.fetches.insert(request_id, sender);
  }

  pub async fn get_fetch(&self, request_id: u64) -> Option<mpsc::Sender<UpstreamFetchEvent>> {
    self.inner.read().await.fetches.get(&request_id).cloned()
  }

  pub async fn remove_fetch(&self, request_id: u64) {
    self.inner.write().await.fetches.remove(&request_id);
  }
}
