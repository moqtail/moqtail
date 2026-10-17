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

use moka::future::Cache;
use moka::notification::RemovalCause;
use moqtail::model::data::fetch_object::FetchObjectPayload;
use std::sync::Arc;
use tokio::fs::OpenOptions;
use tokio::io::AsyncWriteExt;
use tokio::sync::RwLock;
use tracing::{error, info, trace};

use super::config::{AppConfig, CacheExpirationType};

/// Composite cache key combining relay_track_id and group_id for global uniqueness
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CacheKey {
  pub relay_track_id: u64,
  pub group_id: u64,
}

impl CacheKey {
  /// Create a new cache key
  pub fn new(relay_track_id: u64, group_id: u64) -> Self {
    Self {
      relay_track_id,
      group_id,
    }
  }
}

impl std::fmt::Display for CacheKey {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(f, "track:{}_group:{}", self.relay_track_id, self.group_id)
  }
}

// Type alias for the cached value (group objects)
type GroupObjects = Arc<RwLock<Vec<FetchObjectPayload>>>;

#[derive(Debug, Clone)]
pub struct TrackCache {
  pub relay_track_id: u64,
  // Moka cache for storing groups of objects with composite keys
  cache: Cache<CacheKey, GroupObjects>,
  #[allow(dead_code)] // Used in eviction listener closure
  log_folder: String,
}

impl TrackCache {
  pub fn new(relay_track_id: u64, cache_size: usize, config: &AppConfig) -> Self {
    let log_folder = config.log_folder.clone();
    let log_folder_for_listener = log_folder.clone();

    let cache_builder = Cache::builder()
      .max_capacity(cache_size as u64)
      .eviction_listener(move |key: Arc<CacheKey>, value: GroupObjects, cause| {
        let relay_track_id = key.relay_track_id;
        let group_id = key.group_id;
        let log_folder = log_folder_for_listener.clone();

        tokio::spawn(async move {
          let object_count = value.read().await.len();
          Self::log_cache_eviction(log_folder, relay_track_id, group_id, object_count, cause).await;
        });
      });

    // Configure expiration based on config
    let cache = match config.cache_expiration_type {
      CacheExpirationType::Ttl => {
        info!(
          "track_cache::new | configuring TTL cache | track: {} duration: {}min",
          relay_track_id, config.cache_expiration_minutes
        );
        cache_builder
          .time_to_live(config.get_cache_expiration_duration())
          .build()
      }
      CacheExpirationType::Tti => {
        info!(
          "track_cache::new | configuring TTI cache | track: {} duration: {}min",
          relay_track_id, config.cache_expiration_minutes
        );
        cache_builder
          .time_to_idle(config.get_cache_expiration_duration())
          .build()
      }
    };

    Self {
      relay_track_id,
      cache,
      log_folder,
    }
  }

  /// Log cache eviction events to cache_eviction.log
  async fn log_cache_eviction(
    log_folder: String,
    relay_track_id: u64,
    group_id: u64,
    object_count: usize,
    cause: RemovalCause,
  ) {
    let log_filename = "cache_eviction.log";
    let log_path = std::path::Path::new(&log_folder).join(log_filename);

    let cause_str = match cause {
      RemovalCause::Size => "SIZE",
      RemovalCause::Expired => "EXPIRED",
      RemovalCause::Explicit => "EXPLICIT",
      RemovalCause::Replaced => "REPLACED",
    };

    let log_entry = format!(
      "{},{},{},{}\n",
      relay_track_id, group_id, object_count, cause_str
    );

    // Create logs directory if it doesn't exist
    if let Err(e) = tokio::fs::create_dir_all(&log_folder).await {
      error!("Failed to create log directory {}: {:?}", log_folder, e);
      return;
    }

    // Append to log file
    match OpenOptions::new()
      .create(true)
      .append(true)
      .open(&log_path)
      .await
    {
      Ok(mut file) => {
        if let Err(e) = file.write_all(log_entry.as_bytes()).await {
          error!(
            "Failed to write to cache eviction log file {:?}: {:?}",
            log_path, e
          );
        }
      }
      Err(e) => {
        error!(
          "Failed to open cache eviction log file {:?}: {:?}",
          log_path, e
        );
      }
    }
  }

  pub async fn add_object(&self, object: FetchObjectPayload) {
    let cache_key = CacheKey::new(self.relay_track_id, object.group_id);

    let group_objects = self
      .cache
      .get_with(cache_key, async { Arc::new(RwLock::new(Vec::new())) })
      .await;

    // Subgroups of a group arrive on separate streams and interleave, but a FETCH
    // serializes Object IDs as ascending deltas, so the group is kept sorted.
    let mut objects = group_objects.write().await;
    match objects.binary_search_by_key(&object.object_id, |o| o.object_id) {
      Ok(_) => return,
      Err(pos) => objects.insert(pos, object),
    }
    trace!(
      "track_cache::add_object | added object to group | track: {} group: {} total_objects: {}",
      self.relay_track_id,
      cache_key.group_id,
      objects.len()
    );
  }

  /// Get cache statistics (for monitoring/debugging)
  #[allow(dead_code)]
  pub async fn get_cache_stats(&self) -> (u64, u64) {
    (self.cache.entry_count(), self.cache.weighted_size())
  }

  /// Manually run pending tasks (for testing or maintenance)
  #[allow(dead_code)]
  pub async fn run_pending_tasks(&self) {
    self.cache.run_pending_tasks().await;
  }

  /// Get a specific group if it exists
  #[allow(dead_code)]
  pub async fn get_group(&self, group_id: u64) -> Option<GroupObjects> {
    let cache_key = CacheKey::new(self.relay_track_id, group_id);
    self.cache.get(&cache_key).await
  }

  /// Check if a group exists in cache
  #[allow(dead_code)]
  pub async fn contains_group(&self, group_id: u64) -> bool {
    let cache_key = CacheKey::new(self.relay_track_id, group_id);
    self.cache.contains_key(&cache_key)
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::server::config::Cli;
  use bytes::Bytes;
  use clap::Parser;
  use moqtail::model::data::constant::ObjectForwardingPreference;
  use tokio::sync::Barrier;

  fn object(group_id: u64, subgroup_id: u64) -> FetchObjectPayload {
    FetchObjectPayload {
      group_id,
      subgroup_id,
      object_id: subgroup_id,
      publisher_priority: 0,
      forwarding_preference: ObjectForwardingPreference::Subgroup,
      properties: None,
      payload: Bytes::from_static(b"x"),
    }
  }

  const SUBGROUPS: u64 = 8;

  #[tokio::test]
  async fn interleaved_subgroups_are_stored_in_object_order() {
    let config = AppConfig::from_cli(Cli::parse_from(["relay"]));
    let cache = TrackCache::new(1, 10_000, &config);

    // Subgroup 1 runs ahead of subgroup 0, and one object is delivered twice.
    for (subgroup_id, object_id) in [(1, 3), (1, 4), (0, 0), (0, 1), (1, 4), (0, 2)] {
      let mut o = object(0, subgroup_id);
      o.object_id = object_id;
      cache.add_object(o).await;
    }

    let group = cache.get_group(0).await.unwrap();
    let ids: Vec<u64> = group.read().await.iter().map(|o| o.object_id).collect();
    assert_eq!(ids, vec![0, 1, 2, 3, 4]);
  }

  #[tokio::test(flavor = "multi_thread", worker_threads = 8)]
  async fn first_objects_of_concurrent_subgroups_all_land_in_the_group() {
    let config = AppConfig::from_cli(Cli::parse_from(["relay"]));
    let cache = TrackCache::new(1, 10_000, &config);

    for group_id in 0..1000 {
      let barrier = Arc::new(Barrier::new(SUBGROUPS as usize));
      let tasks: Vec<_> = (0..SUBGROUPS)
        .map(|subgroup_id| {
          let cache = cache.clone();
          let barrier = barrier.clone();
          tokio::spawn(async move {
            barrier.wait().await;
            cache.add_object(object(group_id, subgroup_id)).await;
          })
        })
        .collect();
      for task in tasks {
        task.await.unwrap();
      }

      let group = cache.get_group(group_id).await.unwrap();
      assert_eq!(
        group.read().await.len(),
        SUBGROUPS as usize,
        "group {group_id} lost an object"
      );
    }
  }
}
