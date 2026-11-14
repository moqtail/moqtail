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

//! Object processing time, measured on the delivery path.
//!
//! Recording a value is an index computation and a relaxed atomic add: no locks, no
//! allocation. Histograms only ever count up; the observability layer takes copies
//! to compute percentiles over windows.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Bucket `i` holds values up to `10 µs × 2^i`; the last one holds everything above.
pub const BUCKETS: usize = 24;
const FIRST_BUCKET_MICROS: u64 = 10;

/// The upper bound of each bucket in microseconds; the last is unbounded.
pub fn bucket_bounds_micros() -> [u64; BUCKETS] {
  let mut bounds = [u64::MAX; BUCKETS];
  for (i, bound) in bounds.iter_mut().enumerate().take(BUCKETS - 1) {
    *bound = FIRST_BUCKET_MICROS << i;
  }
  bounds
}

fn bucket_of(micros: u64) -> usize {
  if micros <= FIRST_BUCKET_MICROS {
    return 0;
  }
  // The smallest i with 10 << i >= micros.
  let ratio = micros.div_ceil(FIRST_BUCKET_MICROS);
  let i = (u64::BITS - (ratio - 1).leading_zeros()) as usize;
  i.min(BUCKETS - 1)
}

/// A histogram written by one task at a time.
#[derive(Debug, Default)]
pub struct Histogram {
  buckets: [AtomicU64; BUCKETS],
}

impl Histogram {
  pub fn record(&self, value: Duration) {
    let micros = value.as_micros().min(u64::MAX as u128) as u64;
    self.buckets[bucket_of(micros)].fetch_add(1, Ordering::Relaxed);
  }

  /// The count in each bucket, since the histogram was created.
  pub fn counts(&self) -> [u64; BUCKETS] {
    std::array::from_fn(|i| self.buckets[i].load(Ordering::Relaxed))
  }
}

/// Keeps a value on its own cache line, so shards written by different threads
/// do not slow each other down.
#[derive(Debug, Default)]
#[repr(align(128))]
struct Padded<T>(T);

const SHARDS: usize = 16;

/// A histogram written by many threads: each thread adds to its own shard, and
/// reading sums them.
#[derive(Debug, Default)]
pub struct ShardedHistogram {
  shards: [Padded<Histogram>; SHARDS],
}

thread_local! {
  static SHARD: usize = {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    NEXT.fetch_add(1, Ordering::Relaxed) as usize % SHARDS
  };
}

impl ShardedHistogram {
  pub fn record(&self, value: Duration) {
    SHARD.with(|shard| self.shards[*shard].0.record(value));
  }

  pub fn counts(&self) -> [u64; BUCKETS] {
    let mut total = [0; BUCKETS];
    for shard in &self.shards {
      for (sum, count) in total.iter_mut().zip(shard.0.counts()) {
        *sum += count;
      }
    }
    total
  }
}

/// A counter added to by many threads: each thread adds to its own shard, and
/// reading sums them.
#[derive(Debug, Default)]
pub struct ShardedCounter {
  shards: [Padded<AtomicU64>; SHARDS],
}

impl ShardedCounter {
  pub fn add(&self, n: u64) {
    SHARD.with(|shard| self.shards[*shard].0.fetch_add(n, Ordering::Relaxed));
  }

  pub fn get(&self) -> u64 {
    self
      .shards
      .iter()
      .map(|s| s.0.load(Ordering::Relaxed))
      .sum()
  }
}

/// Relay-wide counts of what flows through the data path.
#[derive(Debug, Default)]
pub struct RelayCounters {
  pub objects_received: ShardedCounter,
  pub bytes_received: ShardedCounter,
  /// Objects and payload bytes written to subscribers of live tracks.
  pub objects_sent: ShardedCounter,
  pub bytes_sent: ShardedCounter,
  /// Objects and payload bytes written to FETCH responses.
  pub fetch_objects_sent: ShardedCounter,
  pub fetch_bytes_sent: ShardedCounter,
  /// Streams the relay opened to subscribers.
  pub streams_opened: ShardedCounter,
  /// Streams publishers opened to the relay.
  pub streams_accepted: ShardedCounter,
  /// Objects not queued for a subscriber that had too many bytes waiting.
  pub dropped_queue_full: ShardedCounter,
  /// Objects skipped because they waited past the delivery timeout.
  pub dropped_delivery_timeout: ShardedCounter,
  /// Subscriptions reset because they fell too far behind.
  pub subscriptions_too_far_behind: ShardedCounter,
  /// Cached groups removed, by cause.
  pub cache_expired: ShardedCounter,
  pub cache_evicted_for_size: ShardedCounter,
  pub cache_removed: ShardedCounter,
}

impl RelayCounters {
  pub fn object_received(&self, payload_bytes: usize) {
    self.objects_received.add(1);
    self.bytes_received.add(payload_bytes as u64);
  }

  pub fn object_sent(&self, payload_bytes: usize) {
    self.objects_sent.add(1);
    self.bytes_sent.add(payload_bytes as u64);
  }

  pub fn fetch_object_sent(&self, payload_bytes: usize) {
    self.fetch_objects_sent.add(1);
    self.fetch_bytes_sent.add(payload_bytes as u64);
  }
}

/// Traffic of one accounting zone: the tracks under a namespace prefix.
#[derive(Debug, Default)]
pub struct ZoneCounters {
  pub objects_in: ShardedCounter,
  pub bytes_in: ShardedCounter,
  pub objects_out: ShardedCounter,
  pub bytes_out: ShardedCounter,
}

impl ZoneCounters {
  pub fn object_in(&self, payload_bytes: usize) {
    self.objects_in.add(1);
    self.bytes_in.add(payload_bytes as u64);
  }

  pub fn object_out(&self, payload_bytes: usize) {
    self.objects_out.add(1);
    self.bytes_out.add(payload_bytes as u64);
  }
}

/// When an object passed each point of the relay on its way to one subscriber.
#[derive(Debug, Clone, Copy)]
pub struct ObjectTimes {
  /// Read from the publisher's stream.
  pub arrived: Instant,
  /// Handed to the subscribers' queues.
  pub queued: Instant,
  /// Taken from the queue by this subscription's task.
  pub picked: Instant,
  /// Its write to the subscriber's stream returned.
  pub written: Instant,
}

/// Processing time across the whole relay, by stage.
#[derive(Debug, Default)]
pub struct RelayStats {
  /// From arrival to the subscribers' queues. Recorded once per delivery.
  pub ingest: ShardedHistogram,
  /// Waiting in a subscription's queue.
  pub queue: ShardedHistogram,
  /// Writing to the subscriber's stream.
  pub write: ShardedHistogram,
  /// From arrival to written.
  pub total: ShardedHistogram,
  /// From a subscription's creation to its first object written.
  pub first_object: ShardedHistogram,
}

/// Processing time of one track's objects, over all its subscriptions.
#[derive(Debug, Default)]
pub struct TrackStats {
  pub total: ShardedHistogram,
}

/// The handle a subscription records its deliveries with.
#[derive(Debug)]
pub struct SubscriptionStats {
  relay: Arc<RelayStats>,
  /// `None` unless processing time is kept per track.
  track: Option<Arc<TrackStats>>,
  /// `None` unless processing time is kept per subscription.
  pub own: Option<Histogram>,
  /// Measure only objects whose id is a multiple of this.
  sample: u64,
  created: Instant,
  first_written: AtomicBool,
  slow: Option<Arc<dyn SlowObjects>>,
  pub track_id: u64,
  pub conn_id: u64,
  pub request_id: u64,
}

/// Where deliveries and subscriptions slower than a threshold are reported.
pub trait SlowObjects: Send + Sync + std::fmt::Debug {
  /// Deliveries at or under this are not reported.
  fn threshold(&self) -> Duration;
  fn slow_object(&self, stats: &SubscriptionStats, times: ObjectTimes, group: u64, object: u64);

  /// Subscriptions whose first object took at most this are not reported.
  fn first_object_threshold(&self) -> Duration;
  fn slow_first_object(&self, stats: &SubscriptionStats, waited: Duration);
}

impl SubscriptionStats {
  #[allow(clippy::too_many_arguments)]
  pub fn new(
    relay: Arc<RelayStats>,
    track: Option<Arc<TrackStats>>,
    own: bool,
    sample: u64,
    slow: Option<Arc<dyn SlowObjects>>,
    track_id: u64,
    conn_id: u64,
    request_id: u64,
  ) -> Self {
    SubscriptionStats {
      relay,
      track,
      own: own.then(Histogram::default),
      sample: sample.max(1),
      created: Instant::now(),
      first_written: AtomicBool::new(false),
      slow,
      track_id,
      conn_id,
      request_id,
    }
  }

  /// Records one object written to the subscriber. `arrived` is `None` when the
  /// relay was built without taking arrival times; the queue time then stands in.
  pub fn object_written(
    &self,
    arrived: Option<Instant>,
    queued: Instant,
    picked: Instant,
    group: u64,
    object: u64,
  ) {
    let written = Instant::now();
    if !self.first_written.swap(true, Ordering::Relaxed) {
      let waited = written - self.created;
      self.relay.first_object.record(waited);
      if let Some(slow) = &self.slow
        && waited > slow.first_object_threshold()
      {
        slow.slow_first_object(self, waited);
      }
    }
    if !object.is_multiple_of(self.sample) {
      return;
    }
    let times = ObjectTimes {
      arrived: arrived.unwrap_or(queued),
      queued,
      picked,
      written,
    };
    let total = written - times.arrived;
    self.relay.ingest.record(queued - times.arrived);
    self.relay.queue.record(picked - queued);
    self.relay.write.record(written - picked);
    self.relay.total.record(total);
    if let Some(track) = &self.track {
      track.total.record(total);
    }
    if let Some(own) = &self.own {
      own.record(total);
    }
    if let Some(slow) = &self.slow
      && total > slow.threshold()
    {
      slow.slow_object(self, times, group, object);
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn buckets_double_from_ten_microseconds() {
    assert_eq!(bucket_of(0), 0);
    assert_eq!(bucket_of(10), 0);
    assert_eq!(bucket_of(11), 1);
    assert_eq!(bucket_of(20), 1);
    assert_eq!(bucket_of(21), 2);
    assert_eq!(bucket_of(1_000_000), 17); // 10 µs × 2^17 ≈ 1.3 s
    assert_eq!(bucket_of(u64::MAX), BUCKETS - 1);
    let bounds = bucket_bounds_micros();
    assert_eq!(
      (bounds[0], bounds[3], bounds[BUCKETS - 1]),
      (10, 80, u64::MAX)
    );
  }

  #[test]
  fn a_written_object_reaches_every_level_and_the_first_is_timed() {
    let relay = Arc::new(RelayStats::default());
    let track = Arc::new(TrackStats::default());
    let stats = SubscriptionStats::new(relay.clone(), Some(track.clone()), true, 1, None, 1, 2, 3);
    let queued = Instant::now();
    stats.object_written(Some(queued), queued, queued, 0, 0);
    stats.object_written(Some(queued), queued, queued, 0, 1);
    assert_eq!(relay.total.counts().iter().sum::<u64>(), 2);
    assert_eq!(track.total.counts().iter().sum::<u64>(), 2);
    assert_eq!(stats.own.as_ref().unwrap().counts().iter().sum::<u64>(), 2);
    assert_eq!(relay.first_object.counts().iter().sum::<u64>(), 1);
  }

  #[test]
  fn sampling_measures_every_nth_object_id() {
    let relay = Arc::new(RelayStats::default());
    let stats = SubscriptionStats::new(relay.clone(), None, false, 4, None, 1, 2, 3);
    let now = Instant::now();
    for object in 0..8 {
      stats.object_written(None, now, now, 0, object);
    }
    assert_eq!(relay.total.counts().iter().sum::<u64>(), 2);
  }
}
