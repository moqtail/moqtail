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

//! Waiting for a publisher's streams and datagrams to finish before its track is declared done.

use super::Track;
use crate::server::utils;
use moqtail::model::control::publish_done::PublishDone;
use moqtail::model::data::full_track_name::FullTrackName;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio::sync::{Notify, RwLock};
use tokio::time::Instant;
use tracing::{debug, warn};

/// How many data streams each publisher has finished sending for one track.
///
/// A PUBLISH_DONE carries the number of data streams its sender opened, and it
/// travels on the request stream, which overtakes those streams. Ending the
/// downstream subscriptions the moment it lands would cut off objects still
/// arriving, so the relay waits here for the streams to be accounted for first.
#[derive(Debug, Default)]
pub struct PublisherStreamProgress {
  /// publisher_connection_id -> data streams finished for this track.
  closed: RwLock<HashMap<usize, u64>>,
  changed: Notify,
}

impl PublisherStreamProgress {
  /// Record one finished data stream from this publisher.
  pub async fn stream_closed(&self, publisher_connection_id: usize) {
    {
      let mut closed = self.closed.write().await;
      *closed.entry(publisher_connection_id).or_insert(0) += 1;
    }
    self.changed.notify_waiters();
  }

  pub async fn closed_count(&self, publisher_connection_id: usize) -> u64 {
    self
      .closed
      .read()
      .await
      .get(&publisher_connection_id)
      .copied()
      .unwrap_or(0)
  }

  /// Drop a publisher's count once it no longer publishes the track.
  pub async fn forget(&self, publisher_connection_id: usize) {
    self.closed.write().await.remove(&publisher_connection_id);
  }

  /// Wait for `stream_count` of this publisher's data streams to finish, giving up
  /// after `timeout`. Returns how many had finished when the wait ended; a result
  /// below `stream_count` means it timed out.
  pub async fn wait_for(
    &self,
    publisher_connection_id: usize,
    stream_count: u64,
    timeout: Duration,
  ) -> u64 {
    let wait = async {
      loop {
        // Registered before the count is read, so a stream finishing between the
        // two still wakes this loop rather than leaving it to time out.
        let changed = self.changed.notified();
        tokio::pin!(changed);
        changed.as_mut().enable();

        let closed = self.closed_count(publisher_connection_id).await;
        if closed >= stream_count {
          return closed;
        }
        changed.await;
      }
    };

    match tokio::time::timeout(timeout, wait).await {
      Ok(closed) => closed,
      Err(_) => self.closed_count(publisher_connection_id).await,
    }
  }
}

/// When the last datagram for a track arrived, so a PUBLISH_DONE can wait for the
/// datagrams it would otherwise overtake. Datagrams are not counted in its Stream
/// Count, so the relay cannot tell when the last one is in; it waits for them to go
/// quiet instead.
#[derive(Debug, Default)]
pub struct DatagramArrivals {
  /// Microseconds from `utils::BASE_TIME` to the last arrival, plus one; 0 before any.
  last: AtomicU64,
}

impl DatagramArrivals {
  pub fn arrived(&self) {
    let micros = utils::BASE_TIME.elapsed().as_micros() as u64;
    self.last.store(micros + 1, Ordering::Relaxed);
  }

  /// Waits until no datagram has arrived for `quiet`, or until `deadline`. A track
  /// that has had no datagram does not wait at all.
  pub async fn wait_for_quiet(&self, quiet: Duration, deadline: Instant) {
    loop {
      let last = match self.last.load(Ordering::Relaxed) {
        0 => return,
        micros => Instant::from_std(*utils::BASE_TIME) + Duration::from_micros(micros - 1),
      };
      let until = (last + quiet).min(deadline);
      if Instant::now() >= until {
        return;
      }
      tokio::time::sleep_until(until).await;
    }
  }
}

/// Holds a PUBLISH_DONE back until `stream_count` of that publisher's data streams
/// for the track have finished arriving, and, for a track that has been receiving
/// datagrams, until none has arrived for `datagram_quiet`; all within `timeout`. A
/// publisher that names more streams than it opens must not stall the subscription's
/// end, so a short count is logged and the caller carries on.
pub async fn await_publisher_streams(
  track: &Arc<Track>,
  publisher_connection_id: usize,
  stream_count: u64,
  timeout: Duration,
  datagram_quiet: Duration,
) {
  let deadline = Instant::now() + timeout;

  let (progress, datagrams, full_track_name) = {
    (
      track.publisher_stream_progress.clone(),
      track.datagram_arrivals.clone(),
      track.full_track_name.clone(),
    )
  };

  if stream_count > 0 {
    await_streams(
      &progress,
      &full_track_name,
      publisher_connection_id,
      stream_count,
      timeout,
    )
    .await;
  }
  datagrams.wait_for_quiet(datagram_quiet, deadline).await;
}

async fn await_streams(
  progress: &PublisherStreamProgress,
  full_track_name: &FullTrackName,
  publisher_connection_id: usize,
  stream_count: u64,
  timeout: Duration,
) {
  let closed = progress
    .wait_for(publisher_connection_id, stream_count, timeout)
    .await;

  if stream_count == PublishDone::UNKNOWN_STREAM_COUNT {
    debug!(
      "PUBLISH_DONE from publisher {} for {:?} gave no exact stream count; {} finished within {:?}",
      publisher_connection_id, full_track_name, closed, timeout
    );
  } else if closed < stream_count {
    warn!(
      "PUBLISH_DONE from publisher {} for {:?} named {} streams, {} finished within {:?}; ending the subscription anyway",
      publisher_connection_id, full_track_name, stream_count, closed, timeout
    );
  } else {
    debug!(
      "PUBLISH_DONE from publisher {} for {:?}: all {} streams finished",
      publisher_connection_id, full_track_name, stream_count
    );
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  const PUBLISHER: usize = 7;
  const OTHER_PUBLISHER: usize = 9;

  #[tokio::test]
  async fn a_track_without_datagrams_does_not_wait_for_them() {
    let arrivals = DatagramArrivals::default();
    let started = Instant::now();
    arrivals
      .wait_for_quiet(Duration::from_secs(5), started + Duration::from_secs(5))
      .await;
    assert!(started.elapsed() < Duration::from_millis(50));
  }

  #[tokio::test]
  async fn datagrams_are_waited_for_until_they_go_quiet() {
    let arrivals = Arc::new(DatagramArrivals::default());
    arrivals.arrived();
    let started = Instant::now();
    // One more arrives 40 ms in, which pushes the quiet window on.
    let late = arrivals.clone();
    tokio::spawn(async move {
      tokio::time::sleep(Duration::from_millis(40)).await;
      late.arrived();
    });
    arrivals
      .wait_for_quiet(Duration::from_millis(60), started + Duration::from_secs(5))
      .await;
    let waited = started.elapsed();
    assert!(waited >= Duration::from_millis(100), "waited {waited:?}");
    assert!(waited < Duration::from_secs(1), "waited {waited:?}");
  }

  #[tokio::test]
  async fn datagrams_that_never_go_quiet_are_waited_for_only_until_the_deadline() {
    let arrivals = Arc::new(DatagramArrivals::default());
    arrivals.arrived();
    let steady = arrivals.clone();
    let feeder = tokio::spawn(async move {
      loop {
        tokio::time::sleep(Duration::from_millis(10)).await;
        steady.arrived();
      }
    });
    let started = Instant::now();
    arrivals
      .wait_for_quiet(
        Duration::from_millis(50),
        started + Duration::from_millis(150),
      )
      .await;
    feeder.abort();
    let waited = started.elapsed();
    assert!(waited >= Duration::from_millis(150), "waited {waited:?}");
    assert!(waited < Duration::from_secs(1), "waited {waited:?}");
  }

  #[tokio::test]
  async fn wait_returns_at_once_when_the_streams_already_finished() {
    let progress = PublisherStreamProgress::default();
    for _ in 0..3 {
      progress.stream_closed(PUBLISHER).await;
    }

    let closed = progress
      .wait_for(PUBLISHER, 3, Duration::from_millis(50))
      .await;

    assert_eq!(closed, 3);
  }

  #[tokio::test]
  async fn wait_is_released_by_a_stream_finishing_later() {
    let progress = Arc::new(PublisherStreamProgress::default());
    let writer = progress.clone();
    tokio::spawn(async move {
      for _ in 0..2 {
        tokio::time::sleep(Duration::from_millis(10)).await;
        writer.stream_closed(PUBLISHER).await;
      }
    });

    let closed = progress
      .wait_for(PUBLISHER, 2, Duration::from_secs(5))
      .await;

    assert_eq!(closed, 2);
  }

  #[tokio::test]
  async fn wait_gives_up_and_reports_the_short_count() {
    let progress = PublisherStreamProgress::default();
    progress.stream_closed(PUBLISHER).await;

    let closed = progress
      .wait_for(PUBLISHER, 4, Duration::from_millis(20))
      .await;

    assert_eq!(closed, 1);
  }

  #[tokio::test]
  async fn each_publisher_is_counted_on_its_own() {
    let progress = PublisherStreamProgress::default();
    progress.stream_closed(PUBLISHER).await;
    progress.stream_closed(PUBLISHER).await;
    progress.stream_closed(OTHER_PUBLISHER).await;

    assert_eq!(progress.closed_count(PUBLISHER).await, 2);
    assert_eq!(progress.closed_count(OTHER_PUBLISHER).await, 1);
    // One publisher's streams must not release another's PUBLISH_DONE.
    assert_eq!(
      progress
        .wait_for(OTHER_PUBLISHER, 2, Duration::from_millis(20))
        .await,
      1
    );
  }

  #[tokio::test]
  async fn forgetting_a_publisher_clears_its_count() {
    let progress = PublisherStreamProgress::default();
    progress.stream_closed(PUBLISHER).await;
    progress.forget(PUBLISHER).await;

    assert_eq!(progress.closed_count(PUBLISHER).await, 0);
  }
}
