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
use moqtail::model::data::full_track_name::FullTrackName;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Weak;
use tokio::sync::RwLock;

#[derive(Clone, Debug)]
pub struct TrackSubscriptionMap {
  pub map: Arc<RwLock<HashMap<FullTrackName, Weak<RwLock<Subscription>>>>>,
}

impl TrackSubscriptionMap {
  pub fn new() -> Self {
    Self {
      map: Arc::new(RwLock::new(HashMap::new())),
    }
  }

  pub async fn add_subscription(
    &self,
    track_name: FullTrackName,
    subscription: Weak<RwLock<Subscription>>,
  ) {
    let mut map = self.map.write().await;
    map.insert(track_name, subscription);
  }

  pub async fn get_subscription(
    &self,
    track_name: &FullTrackName,
  ) -> Option<Weak<RwLock<Subscription>>> {
    let map = self.map.read().await;
    map.get(track_name).cloned()
  }

  pub async fn remove_subscription(&self, track_name: &FullTrackName) -> bool {
    let mut map = self.map.write().await;
    map.remove(track_name).is_some()
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[tokio::test]
  async fn add_get_and_remove_subscription() {
    let m = TrackSubscriptionMap::new();

    let tn = FullTrackName::try_new("ns", "track").expect("create track name");

    // initially none
    assert!(m.get_subscription(&tn).await.is_none());

    // insert a Weak pointer (no real Subscription needed for map semantics)
    let w: Weak<RwLock<Subscription>> = Weak::new();
    m.add_subscription(tn.clone(), w.clone()).await;

    let got = m.get_subscription(&tn).await;
    assert!(got.is_some());

    // remove should return true
    assert!(m.remove_subscription(&tn).await);

    // now gone
    assert!(m.get_subscription(&tn).await.is_none());

    // removing again returns false
    assert!(!m.remove_subscription(&tn).await);
  }
}
