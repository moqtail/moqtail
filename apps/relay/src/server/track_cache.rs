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
use moqtail::model::common::location::Location;
use moqtail::model::data::fetch_object::FetchObjectPayload;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
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

/// At most this many separate ranges are remembered as absent. Forgetting one only
/// sends a later request for it upstream.
const MAX_ABSENT_RANGES: usize = 65_536;

/// The Location after `location`.
fn after(location: &Location) -> Option<Location> {
  match location.object.checked_add(1) {
    Some(object) => Some(Location::new(location.group, object)),
    None => location
      .group
      .checked_add(1)
      .map(|group| Location::new(group, 0)),
  }
}

/// Inclusive Location ranges known to hold no Object, kept sorted, disjoint and
/// merged. That an Object does not exist never changes, so these outlive the cached
/// Objects: once a group is evicted, what is left of it is what is known to be absent.
#[derive(Debug, Default)]
struct AbsentRanges {
  /// Start to end, both inclusive.
  ranges: BTreeMap<Location, Location>,
}

impl AbsentRanges {
  fn insert(&mut self, from: Location, to: Location) {
    let (mut from, mut to) = (from, to);
    // Every range that overlaps or touches this one starts no later than the
    // Location after its end.
    let limit = after(&to).unwrap_or_else(|| to.clone());
    let touching: Vec<(Location, Location)> = self
      .ranges
      .range(..=limit)
      .rev()
      .take_while(|(_, end)| after(end).is_none_or(|next| next >= from))
      .map(|(start, end)| (start.clone(), end.clone()))
      .collect();
    for (start, end) in touching {
      self.ranges.remove(&start);
      from = from.min(start);
      to = to.max(end);
    }
    self.ranges.insert(from, to);
    while self.ranges.len() > MAX_ABSENT_RANGES {
      self.ranges.pop_first();
    }
  }

  /// The end of the range holding `location`, if one does.
  fn end_containing(&self, location: &Location) -> Option<Location> {
    self
      .ranges
      .range(..=location.clone())
      .next_back()
      .filter(|(_, end)| *end >= location)
      .map(|(_, end)| end.clone())
  }
}

#[derive(Debug, Clone)]
pub struct TrackCache {
  pub relay_track_id: u64,
  // Moka cache for storing groups of objects with composite keys
  cache: Cache<CacheKey, GroupObjects>,
  absent: Arc<Mutex<AbsentRanges>>,
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
      absent: Arc::new(Mutex::new(AbsentRanges::default())),
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

  /// Records that no Object exists from `from` to `to`, both inclusive.
  pub fn mark_absent(&self, from: Location, to: Location) {
    if from <= to {
      self.absent.lock().unwrap().insert(from, to);
    }
  }

  /// The Objects of `group` from `lo` to `hi`, if the cache accounts for every
  /// Location there: each one either holds an Object the cache has or is known to
  /// hold none. Otherwise None, and the range has to be asked for upstream.
  pub async fn objects_if_complete(
    &self,
    group: u64,
    lo: u64,
    hi: u64,
  ) -> Option<Vec<FetchObjectPayload>> {
    let objects = match self.get_group(group).await {
      Some(objects) => {
        let objects = objects.read().await;
        let first = objects.partition_point(|o| o.object_id < lo);
        objects[first..]
          .iter()
          .take_while(|o| o.object_id <= hi)
          .cloned()
          .collect()
      }
      None => Vec::new(),
    };

    let absent = self.absent.lock().unwrap();
    let mut held = objects.iter().map(|o| o.object_id).peekable();
    let mut cursor = lo;
    while cursor <= hi {
      if let Some(end) = absent.end_containing(&Location::new(group, cursor)) {
        if end.group > group || end.object >= hi {
          break;
        }
        cursor = end.object + 1;
      } else if held.peek() == Some(&cursor) {
        held.next();
        match cursor.checked_add(1) {
          Some(next) => cursor = next,
          None => break,
        }
      } else {
        return None;
      }
    }
    Some(objects)
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

  /// Whether the cache holds no Object of the track. A group is only ever created
  /// by an Object going into it, so no group means no Object. Expired and evicted
  /// groups are swept out first, since the count lags behind them.
  pub async fn is_empty(&self) -> bool {
    self.cache.run_pending_tasks().await;
    self.cache.entry_count() == 0
  }

  /// Get a specific group if it exists
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

  fn loc(group: u64, object: u64) -> Location {
    Location::new(group, object)
  }

  #[test]
  fn absent_ranges_merge_when_they_overlap_or_touch() {
    let mut absent = AbsentRanges::default();
    absent.insert(loc(1, 0), loc(1, 3));
    absent.insert(loc(1, 8), loc(1, 9));
    absent.insert(loc(1, 4), loc(1, 7));
    assert_eq!(absent.ranges.len(), 1);
    assert_eq!(absent.end_containing(&loc(1, 5)), Some(loc(1, 9)));

    // The last Object ID of a group touches the first of the next.
    absent.insert(loc(1, 10), loc(1, u64::MAX));
    absent.insert(loc(2, 0), loc(2, 4));
    assert_eq!(absent.ranges.len(), 1);
    assert_eq!(absent.end_containing(&loc(1, 0)), Some(loc(2, 4)));
    assert_eq!(absent.end_containing(&loc(2, 5)), None);
  }

  #[test]
  fn the_oldest_absent_ranges_are_forgotten_past_the_cap() {
    let mut absent = AbsentRanges::default();
    for group in 0..=MAX_ABSENT_RANGES as u64 {
      absent.insert(loc(group, 1), loc(group, 1));
    }
    assert_eq!(absent.ranges.len(), MAX_ABSENT_RANGES);
    assert_eq!(absent.end_containing(&loc(0, 1)), None);
    assert!(absent.end_containing(&loc(1, 1)).is_some());
  }

  fn object_at(group_id: u64, object_id: u64) -> FetchObjectPayload {
    let mut o = object(group_id, 0);
    o.object_id = object_id;
    o
  }

  #[tokio::test]
  async fn a_range_is_complete_only_when_every_location_is_accounted_for() {
    let config = AppConfig::from_cli(Cli::parse_from(["relay"]));
    let cache = TrackCache::new(1, 10_000, &config);
    for object_id in [0, 1, 4] {
      cache.add_object(object_at(5, object_id)).await;
    }

    // 2 and 3 are unknown.
    assert!(cache.objects_if_complete(5, 0, 4).await.is_none());
    assert_eq!(cache.objects_if_complete(5, 0, 1).await.unwrap().len(), 2);

    cache.mark_absent(loc(5, 2), loc(5, 3));
    let objects = cache.objects_if_complete(5, 0, 4).await.unwrap();
    let ids: Vec<u64> = objects.iter().map(|o| o.object_id).collect();
    assert_eq!(ids, [0, 1, 4]);
    // Past 4 nothing is known.
    assert!(cache.objects_if_complete(5, 0, 5).await.is_none());
  }

  #[tokio::test]
  async fn a_group_known_to_be_empty_is_complete_with_no_objects() {
    let config = AppConfig::from_cli(Cli::parse_from(["relay"]));
    let cache = TrackCache::new(1, 10_000, &config);
    cache.mark_absent(loc(6, 0), loc(8, u64::MAX));
    assert_eq!(
      cache.objects_if_complete(7, 0, u64::MAX).await,
      Some(vec![])
    );
    assert_eq!(cache.objects_if_complete(8, 3, 9).await, Some(vec![]));
    assert!(cache.objects_if_complete(9, 0, 0).await.is_none());
    assert!(cache.is_empty().await, "known absence is no cached Object");
  }

  #[tokio::test]
  async fn what_is_absent_outlives_an_evicted_group() {
    let config = AppConfig::from_cli(Cli::parse_from(["relay"]));
    let cache = TrackCache::new(1, 10_000, &config);
    cache.add_object(object_at(5, 0)).await;
    cache.mark_absent(loc(5, 1), loc(5, u64::MAX));
    assert!(cache.objects_if_complete(5, 0, u64::MAX).await.is_some());

    cache.cache.invalidate(&CacheKey::new(1, 5)).await;
    // Object 0 is unknown again; the rest is still known not to exist.
    assert!(cache.objects_if_complete(5, 0, u64::MAX).await.is_none());
    assert_eq!(
      cache.objects_if_complete(5, 1, u64::MAX).await,
      Some(vec![])
    );
  }
}
