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

use crate::server::subscription::Subscription;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Weak;
use tokio::sync::RwLock;

/// A client's subscriptions, keyed by the Request ID of the SUBSCRIBE (or PUBLISH)
/// that created each one. A client can hold several subscriptions to the same Track,
/// so the Track name does not identify one.
#[derive(Clone, Debug)]
pub struct RequestSubscriptionMap {
  pub map: Arc<RwLock<HashMap<u64, Weak<RwLock<Subscription>>>>>,
}

impl RequestSubscriptionMap {
  pub fn new() -> Self {
    Self {
      map: Arc::new(RwLock::new(HashMap::new())),
    }
  }

  pub async fn add(&self, request_id: u64, subscription: Weak<RwLock<Subscription>>) {
    let mut map = self.map.write().await;
    map.insert(request_id, subscription);
  }

  pub async fn get(&self, request_id: u64) -> Option<Weak<RwLock<Subscription>>> {
    let map = self.map.read().await;
    map.get(&request_id).cloned()
  }

  pub async fn remove(&self, request_id: u64) -> bool {
    let mut map = self.map.write().await;
    map.remove(&request_id).is_some()
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[tokio::test]
  async fn add_get_and_remove_subscription() {
    let m = RequestSubscriptionMap::new();

    // initially none
    assert!(m.get(2).await.is_none());

    // insert a Weak pointer (no real Subscription needed for map semantics)
    let w: Weak<RwLock<Subscription>> = Weak::new();
    m.add(2, w.clone()).await;

    assert!(m.get(2).await.is_some());

    // remove should return true
    assert!(m.remove(2).await);

    // now gone
    assert!(m.get(2).await.is_none());

    // removing again returns false
    assert!(!m.remove(2).await);
  }

  #[tokio::test]
  async fn subscriptions_to_one_track_are_kept_apart() {
    let m = RequestSubscriptionMap::new();
    let w: Weak<RwLock<Subscription>> = Weak::new();

    // Two subscriptions to the same Track, told apart by Request ID.
    m.add(2, w.clone()).await;
    m.add(4, w.clone()).await;

    // Cancelling one leaves the other in place.
    assert!(m.remove(2).await);
    assert!(m.get(2).await.is_none());
    assert!(m.get(4).await.is_some());
  }
}
