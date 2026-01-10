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

use super::client::MOQTClient;
use super::track::Track;
use moqtail::model::control::subscribe::Subscribe;
use moqtail::model::data::full_track_name::FullTrackName;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::info;

#[derive(Clone)]
pub struct TrackManager {
  pub tracks: Arc<RwLock<HashMap<FullTrackName, Arc<RwLock<Track>>>>>,
  pub track_aliases: Arc<RwLock<BTreeMap<u64, FullTrackName>>>,
}

impl TrackManager {
  pub fn new() -> Self {
    TrackManager {
      tracks: Arc::new(RwLock::new(HashMap::new())),
      track_aliases: Arc::new(RwLock::new(BTreeMap::new())),
    }
  }

  pub async fn add_track(
    &self,
    track_alias: u64,
    full_track_name: FullTrackName,
    track: Track,
  ) -> Arc<RwLock<Track>> {
    let mut tracks = self.tracks.write().await;
    let mut track_aliases = self.track_aliases.write().await;

    let track = Arc::new(RwLock::new(track));
    tracks.insert(full_track_name.clone(), track.clone());
    track_aliases.insert(track_alias, full_track_name.clone());

    info!(
      "Added track with alias {}: {:?}",
      track_alias, full_track_name
    );

    track
  }

  pub async fn remove_track_by_alias(&self, track_alias: u64) {
    let mut track_aliases = self.track_aliases.write().await;
    if let Some(full_track_name) = track_aliases.remove(&track_alias) {
      let mut tracks = self.tracks.write().await;
      tracks.remove(&full_track_name);
      info!(
        "Removed track with alias {}: {:?}",
        track_alias, full_track_name
      );
    }
  }

  pub async fn remove_track_by_name(&self, full_track_name: &FullTrackName) {
    let mut tracks = self.tracks.write().await;
    if tracks.remove(full_track_name).is_some() {
      let mut track_aliases = self.track_aliases.write().await;
      // Find and remove the alias
      let alias_to_remove = track_aliases.iter().find_map(|(alias, name)| {
        if name == full_track_name {
          Some(*alias)
        } else {
          None
        }
      });
      if let Some(alias) = alias_to_remove {
        track_aliases.remove(&alias);
      }
      info!("Removed track: {:?}", full_track_name);
    }
  }

  pub async fn get_track(&self, full_track_name: &FullTrackName) -> Option<Arc<RwLock<Track>>> {
    let tracks = self.tracks.read().await;
    tracks.get(full_track_name).cloned()
  }

  pub async fn has_track(&self, full_track_name: &FullTrackName) -> bool {
    let tracks = self.tracks.read().await;
    tracks.contains_key(full_track_name)
  }

  pub async fn has_track_alias(&self, track_alias: &u64) -> bool {
    let track_aliases = self.track_aliases.read().await;
    track_aliases.contains_key(track_alias)
  }

  pub async fn get_full_track_name(&self, track_alias: u64) -> Option<FullTrackName> {
    let track_aliases = self.track_aliases.read().await;
    track_aliases.get(&track_alias).cloned()
  }

  pub async fn get_all_tracks(&self) -> HashMap<FullTrackName, Arc<RwLock<Track>>> {
    let tracks = self.tracks.read().await;
    tracks.clone()
  }

  pub async fn add_subscription(
    &self,
    full_track_name: &FullTrackName,
    subscriber: Arc<MOQTClient>,
    subscribe_message: Subscribe,
  ) -> Result<(), anyhow::Error> {
    if let Some(track_lock) = self.get_track(full_track_name).await {
      let track = track_lock.write().await;
      track.add_subscription(subscriber, subscribe_message).await
    } else {
      Err(anyhow::anyhow!("Track not found: {:?}", full_track_name))
    }
  }
}
