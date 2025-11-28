use moka::future::Cache;
use moka::notification::RemovalCause;
use moqtail::model::common::location::Location;
use moqtail::model::data::fetch_object::FetchObject;
use std::sync::Arc;
use tokio::fs::OpenOptions;
use tokio::io::AsyncWriteExt;
use tokio::sync::{
  RwLock,
  mpsc::{Receiver, channel},
};
use tracing::{debug, error, info, warn};

use super::config::{AppConfig, CacheExpirationType};

/// Composite cache key combining track_alias and group_id for global uniqueness
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CacheKey {
  pub track_alias: u64,
  pub group_id: u64,
}

impl CacheKey {
  /// Create a new cache key
  pub fn new(track_alias: u64, group_id: u64) -> Self {
    Self {
      track_alias,
      group_id,
    }
  }
}

impl std::fmt::Display for CacheKey {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(f, "track:{}_group:{}", self.track_alias, self.group_id)
  }
}

// Type alias for the cached value (group objects)
type GroupObjects = Arc<RwLock<Vec<FetchObject>>>;

#[derive(Debug, Clone)]
pub struct TrackCache {
  pub track_alias: u64,
  // Moka cache for storing groups of objects with composite keys
  cache: Cache<CacheKey, GroupObjects>,
  #[allow(dead_code)] // Used in eviction listener closure
  log_folder: String,
}

#[derive(Debug, Clone)]
pub enum CacheConsumeEvent {
  Object(FetchObject),
  EndLocation(Location),
  NoObject,
}

impl TrackCache {
  pub fn new(track_alias: u64, cache_size: usize, config: &AppConfig) -> Self {
    let log_folder = config.log_folder.clone();
    let log_folder_for_listener = log_folder.clone();

    let cache_builder = Cache::builder()
      .max_capacity(cache_size as u64)
      .eviction_listener(move |key: Arc<CacheKey>, value: GroupObjects, cause| {
        let track_alias = key.track_alias;
        let group_id = key.group_id;
        let log_folder = log_folder_for_listener.clone();

        tokio::spawn(async move {
          let object_count = value.read().await.len();
          Self::log_cache_eviction(log_folder, track_alias, group_id, object_count, cause).await;
        });
      });

    // Configure expiration based on config
    let cache = match config.cache_expiration_type {
      CacheExpirationType::Ttl => {
        info!(
          "track_cache::new | configuring TTL cache | track: {} duration: {}min",
          track_alias, config.cache_expiration_minutes
        );
        cache_builder
          .time_to_live(config.get_cache_expiration_duration())
          .build()
      }
      CacheExpirationType::Tti => {
        info!(
          "track_cache::new | configuring TTI cache | track: {} duration: {}min",
          track_alias, config.cache_expiration_minutes
        );
        cache_builder
          .time_to_idle(config.get_cache_expiration_duration())
          .build()
      }
    };

    Self {
      track_alias,
      cache,
      log_folder,
    }
  }

  /// Log cache eviction events to cache_eviction.log
  async fn log_cache_eviction(
    log_folder: String,
    track_alias: u64,
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
      track_alias, group_id, object_count, cause_str
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

  pub async fn add_object(&self, object: FetchObject) {
    let cache_key = CacheKey::new(self.track_alias, object.group_id);

    // Check if group al  y exists in cache
    if let Some(existing_objects) = self.cache.get(&cache_key).await {
      // Add object to existing group
      let mut objects = existing_objects.write().await;
      objects.push(object.clone());
      debug!(
        "track_cache::add_object | added object to existing group | track: {} group: {} object_id: {} total_objects: {}",
        self.track_alias,
        object.group_id,
        object.object_id,
        objects.len()
      );
    } else {
      // Create new group with this object
      let new_group_objects = Arc::new(RwLock::new(vec![object.clone()]));
      self.cache.insert(cache_key, new_group_objects).await;
      debug!(
        "track_cache::add_object | created new group | track: {} group: {} object_id: {}",
        self.track_alias, object.group_id, object.object_id
      );
    }
  }

  pub async fn read_objects(&self, start: Location, end: Location) -> Receiver<CacheConsumeEvent> {
    let (tx, rx) = channel(32); // Smaller buffer for memory efficiency
    let cache = self.cache.clone();
    let track_alias = self.track_alias;

    // TODO: this can be done without using a task and sender-receiver pattern
    // but I'm doing this in order to lay the foundation for the future
    // when the cache will be filled eventually.
    tokio::spawn(async move {
      info!(
        "read_objects | track: {} start: {:?}, end: {:?}",
        track_alias, start, end
      );

      // TODO: compare objects as well
      if start.group > end.group {
        warn!("start group cannot be greater than end group");
        return;
      }

      // Collect all groups in the range that exist in cache
      let mut groups_in_range = Vec::new();

      for group_id in start.group..=end.group {
        let cache_key = CacheKey::new(track_alias, group_id);
        if let Some(objects) = cache.get(&cache_key).await {
          groups_in_range.push((group_id, objects));
        }
      }

      if groups_in_range.is_empty() {
        if let Err(err) = tx.send(CacheConsumeEvent::NoObject).await {
          warn!("read_objects | An error occurred: {:?}", err);
          return;
        }
        return;
      }

      // Send end location based on last group found
      if let Some((last_group_id, last_objects)) = groups_in_range.last() {
        let objects_guard = last_objects.read().await;
        let end_object_id = if let Some(last_object) = objects_guard.last() {
          if end.object > 0 {
            std::cmp::min(last_object.object_id + 1, end.object)
          } else {
            // TODO: Implement the logic to find the last object in the group
            // If End Location.Object in the FETCH request was 0 and the
            // response covers the last Object in the Group, End Location is
            // {Fetch.End Location.Group, 0}

            last_object.object_id + 1
          }
        } else {
          0
        };
        let end_location = Location::new(*last_group_id, end_object_id);
        info!(
          "read_objects | track: {} groups_found: {} end_location: {:?}",
          track_alias,
          groups_in_range.len(),
          &end_location
        );
        if let Err(err) = tx.send(CacheConsumeEvent::EndLocation(end_location)).await {
          warn!("read_objects | An error occurred: {:?}", err);
          return;
        }
      }

      // Send objects from all groups in range
      for (group_id, objects_arc) in groups_in_range {
        let objects = objects_arc.read().await;
        let mut object_counter = 0;
        for object in objects.iter() {
          // Apply range filtering
          if group_id == start.group && start.object > 0 && object.object_id < start.object {
            continue; // Skip objects before start
          }

          // stop when we reach end
          if group_id > end.group
            || (group_id == end.group && end.object > 0 && object.object_id >= end.object)
          {
            break; // Stop at end boundary
          }

          object_counter += 1;

          if let Err(err) = tx.send(CacheConsumeEvent::Object(object.clone())).await {
            warn!("read_objects | An error occurred: {:?}", err);
            break; // Client disconnected
          }
        }

        info!(
          "read_objects | track: {} processed group_id: {} with {} objects",
          track_alias, group_id, object_counter
        );
      }
    });

    rx
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
    let cache_key = CacheKey::new(self.track_alias, group_id);
    self.cache.get(&cache_key).await
  }

  /// Check if a group exists in cache
  #[allow(dead_code)]
  pub async fn contains_group(&self, group_id: u64) -> bool {
    let cache_key = CacheKey::new(self.track_alias, group_id);
    self.cache.contains_key(&cache_key)
  }
}
