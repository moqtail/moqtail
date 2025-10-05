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

use crate::server::{
  stream_id::{StreamId, StreamType},
  utils,
};
use anyhow::Result;
#[allow(dead_code)]
use bytes::Bytes;
use moqtail::{
  model::{
    common::tuple::Tuple,
    control::{client_setup::ClientSetup, control_message::ControlMessage},
    data::full_track_name::FullTrackName,
  },
  transport::data_stream_handler::{FetchRequest, SubscribeRequest},
};

use std::{
  collections::{BTreeMap, HashMap, VecDeque},
  sync::Arc,
};
use tokio::sync::Notify;
use tokio::sync::{Mutex, RwLock};
use tracing::{debug, error, info, warn};
use wtransport::{Connection, SendStream, error::StreamWriteError};

/// Number of partitions for send stream management to reduce lock contention.
/// Each partition contains a separate HashMap protected by its own RwLock.
/// Higher values reduce contention but increase memory overhead.
/// Should be a power of 2 for optimal modulo performance.
pub const SEND_STREAM_PARTITION_COUNT: usize = 16;

pub type SendStreamMap = HashMap<String, Arc<Mutex<SendStream>>>;
pub type SendStreamLock = Arc<RwLock<SendStreamMap>>;
pub type SendStreamList = Vec<SendStreamLock>;

#[derive(Debug, Clone)]
pub(crate) struct MOQTClient {
  pub connection_id: usize,
  pub connection: Arc<Connection>,
  #[allow(dead_code)]
  pub client_setup: Arc<ClientSetup>,
  pub announced_track_namespaces: Arc<RwLock<Vec<Tuple>>>, // the track namespaces the publisher announced
  pub published_tracks: Arc<RwLock<Vec<FullTrackName>>>,   // the tracks the client is publishing
  pub subscribers: Arc<RwLock<Vec<usize>>>, // the subscribers the client is subscribed to

  pub message_queue: Arc<RwLock<VecDeque<ControlMessage>>>, // the control messages the client has sent
  pub message_notify: Arc<Notify>, // notify when a new message is available
  pub send_streams: Arc<SendStreamList>,

  // this contains the requests made by the client and the corresponding request
  pub fetch_requests: Arc<RwLock<BTreeMap<u64, FetchRequest>>>,

  // this contains the requests made by the client and the corresponding request.
  // The key value is the original request id.
  pub subscribe_requests: Arc<RwLock<BTreeMap<u64, SubscribeRequest>>>,
}

impl MOQTClient {
  pub(crate) fn new(
    connection_id: usize,
    connection: Arc<Connection>,
    client_setup: Arc<ClientSetup>,
  ) -> Self {
    let mut send_streams = Vec::with_capacity(SEND_STREAM_PARTITION_COUNT);
    for _ in 0..SEND_STREAM_PARTITION_COUNT {
      send_streams.push(Arc::new(RwLock::new(HashMap::new())));
    }

    MOQTClient {
      connection_id,
      connection,
      client_setup,
      announced_track_namespaces: Arc::new(RwLock::new(Vec::new())),
      published_tracks: Arc::new(RwLock::new(Vec::new())),
      subscribers: Arc::new(RwLock::new(Vec::new())),
      message_queue: Arc::new(RwLock::new(VecDeque::new())),
      message_notify: Arc::new(Notify::default()),
      send_streams: Arc::new(send_streams),
      fetch_requests: Arc::new(RwLock::new(BTreeMap::new())),
      subscribe_requests: Arc::new(RwLock::new(BTreeMap::new())),
    }
  }

  pub(crate) async fn add_announced_track_namespace(&self, track_namespace: Tuple) {
    let mut announced_track_namespaces = self.announced_track_namespaces.write().await;
    announced_track_namespaces.push(track_namespace);
  }

  pub(crate) async fn add_subscriber(&self, subscriber_id: usize) {
    let mut subscribers = self.subscribers.write().await;
    subscribers.push(subscriber_id);
  }

  pub(crate) async fn add_published_track(&self, full_track_name: FullTrackName) {
    let mut published_tracks = self.published_tracks.write().await;
    published_tracks.push(full_track_name);
  }

  pub(crate) async fn get_published_tracks(&self) -> Vec<FullTrackName> {
    let published_tracks = self.published_tracks.read().await;
    published_tracks.clone()
  }

  /// Get the next control message from the queue.
  /// This function will block until a message is available and will return the message.
  pub(crate) async fn wait_for_next_message(&self) -> ControlMessage {
    loop {
      // Acquire the lock and check if there's a message
      let mut message_queue = self.message_queue.write().await;
      if let Some(message) = message_queue.pop_front() {
        return message;
      }
      // Drop the lock before waiting
      drop(message_queue);
      // TODO: what happens when the client disconnects?
      self.message_notify.notified().await;
    }
  }

  // Also, update queue_message to notify:
  pub(crate) async fn queue_message(&self, control_message: ControlMessage) {
    let mut message_queue = self.message_queue.write().await;
    message_queue.push_back(control_message);
    self.message_notify.notify_one();
  }

  /// Calculate the partition index for stream distribution across buckets.
  /// This method implements a load balancing strategy to distribute streams
  /// across multiple stream buckets to improve performance and reduce contention.
  fn get_partition_index(&self, stream_id: &StreamId) -> usize {
    let value = match stream_id.stream_type {
      StreamType::Fetch => {
        // Use a simple hash combining track_alias and fetch_request_id
        stream_id
          .track_alias
          .wrapping_add(stream_id.fetch_request_id.unwrap_or(0).wrapping_mul(13))
      }
      StreamType::Subgroup => {
        // Better distribution using prime number multipliers
        stream_id
          .track_alias
          .wrapping_add(stream_id.group_id.unwrap_or(0).wrapping_mul(17))
          .wrapping_add(stream_id.subgroup_id.unwrap_or(0).wrapping_mul(31))
      }
    };

    // Convert to bytes for fnv_hash function
    let value_bytes = value.to_le_bytes();
    (utils::fnv_hash(&value_bytes) % SEND_STREAM_PARTITION_COUNT as u64) as usize
  }

  fn get_stream_map(
    &self,
    stream_id: &StreamId,
  ) -> Arc<RwLock<HashMap<String, Arc<Mutex<SendStream>>>>> {
    let partition_index = self.get_partition_index(stream_id);
    debug!(
      "get_stream_map | stream_id: {} partition_index: {}",
      stream_id, partition_index
    );
    self.send_streams[partition_index].clone()
  }

  pub async fn get_stream(&self, stream_id: &StreamId) -> Option<Arc<Mutex<SendStream>>> {
    let send_stream_map = self.get_stream_map(stream_id);
    let send_streams = send_stream_map.read().await;
    let send_stream = send_streams.get(stream_id.get_stream_id().as_str());
    send_stream.cloned()
  }

  pub async fn open_stream(
    &self,
    stream_id: &StreamId,
    header_payload: Bytes,
    priority: i32, // Priority for the stream
  ) -> Result<Arc<Mutex<SendStream>>> {
    let send_stream = {
      let send_stream_map = self.get_stream_map(stream_id);
      let mut send_streams = send_stream_map.write().await;
      match send_streams.entry(stream_id.get_stream_id().to_string()) {
        std::collections::hash_map::Entry::Vacant(entry) => {
          let result = self
            .connection
            .open_uni()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to open send stream 1: {:?}", e))?;

          let send_stream = result
            .await
            .map_err(|e| anyhow::anyhow!("Failed to open send stream 2: {:?}", e))?;

          send_stream.set_priority(priority);
          let s = Arc::new(Mutex::new(send_stream));
          entry.insert(s.clone());
          info!(
            "open_stream | added send_stream to send streams ({}) connection_id: {}",
            stream_id, self.connection_id
          );
          s
        }
        std::collections::hash_map::Entry::Occupied(s) => {
          debug!(
            "open_stream | Send stream for {} already exists connection_id: {}",
            stream_id, self.connection_id
          );
          s.get().clone()
        }
      }
    };

    debug!(
      "open_stream |  writing to stream ({}) connection_id: {}",
      stream_id, self.connection_id
    );

    // Write the header payload to the stream
    match send_stream.lock().await.write_all(&header_payload).await {
      Ok(..) => {
        debug!(
          "open_stream |  wrote to stream ({}) connection_id: {}",
          stream_id, self.connection_id
        );
      }
      Err(e) => {
        error!(
          "open_stream |  Failed to write header payload to send stream ({}) connection_id: {}",
          stream_id, self.connection_id
        );

        // remove this from the streams
        let send_stream_map = self.get_stream_map(stream_id);
        let mut send_streams = send_stream_map.write().await;
        send_streams.remove(&stream_id.get_stream_id().to_string());

        return Err(anyhow::anyhow!(
          "Failed to write header payload to send stream ({}): {:?} connection_id: {}",
          stream_id,
          self.connection_id,
          e
        ));
      }
    };

    Ok(send_stream.clone())
  }

  // Remove the stream from the map and finish it
  pub async fn close_stream(&self, stream_id: &StreamId) -> Result<()> {
    let stream = self.remove_stream_by_stream_id(stream_id).await;

    if let Some(send_stream) = stream {
      // gracefully close the stream
      self.finish_stream(stream_id, send_stream).await
    } else {
      warn!(
        "close_stream | Send stream not found for {} connection_id: {}",
        stream_id, self.connection_id
      );
      Err(anyhow::anyhow!(
        "Send stream not found ({}) connection_id: {}",
        stream_id,
        self.connection_id
      ))
    }
  }

  // Just remove the stream from the stream_map
  // The caller finishes the stream and calls this to remove it from the map
  pub async fn remove_stream_by_stream_id(
    &self,
    stream_id: &StreamId,
  ) -> Option<Arc<Mutex<SendStream>>> {
    let send_stream_map = self.get_stream_map(stream_id);
    let mut send_streams = send_stream_map.write().await;
    send_streams.remove(stream_id.get_stream_id().as_str())
  }

  async fn finish_stream(
    &self,
    stream_id: &StreamId,
    send_stream: Arc<Mutex<SendStream>>,
  ) -> Result<()> {
    let mut stream = send_stream.lock().await;

    // gracefully close the stream
    stream.finish().await.map_err(|e| {
      error!(
        "close_stream | Failed to finish send stream ({}): {:?} connection_id: {}",
        stream_id, e, self.connection_id
      );
      anyhow::anyhow!("Failed to finish send stream ({}): {:?}", stream_id, e)
    })?;
    info!(
      "close_stream | Closed send stream ({}) connection_id: {}",
      stream_id, self.connection_id
    );
    Ok(())
  }

  pub async fn write_object_to_stream(
    &self,
    stream_id: &StreamId,
    object_id: u64,
    object: Bytes,
    the_stream: Option<Arc<Mutex<SendStream>>>,
  ) -> Result<(), anyhow::Error> {
    debug!(
      "write_object_to_stream | Writing object to stream ({} - {}) connection_id: {}",
      object_id, stream_id, self.connection_id
    );

    let send_stream = {
      if let Some(send_stream) = the_stream {
        Some(send_stream)
      } else {
        let stream_map = self.get_stream_map(stream_id);
        let send_streams = stream_map.read().await;
        send_streams
          .get(stream_id.get_stream_id().as_str())
          .cloned()
      }
    };

    // flow control

    if let Some(s) = send_stream {
      let mut stream = s.lock().await;
      match stream.write_all(&object).await {
        Ok(..) => {}
        Err(e) => {
          match &e {
            StreamWriteError::Closed | StreamWriteError::Stopped(_) => {
              warn!(
                "write_object_to_stream | Send stream is closed or stopped ({})",
                stream_id.get_stream_id()
              );
              drop(stream);
              // remove this from the streams
              let stream_map = self.get_stream_map(stream_id);
              let mut send_streams = stream_map.write().await;
              send_streams.remove(stream_id.get_stream_id().as_str());
            }
            _ => {}
          }
        }
      };
      Ok(())
    } else {
      warn!(
        "write_object_to_stream | Send stream not found for {} connection_id: {}",
        stream_id, self.connection_id
      );
      // This is not an error. The stream already started, wait for the next group...
      // Err(anyhow::anyhow!("Send stream not found ({})", stream_id))
      Ok(())
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::server::stream_id::{StreamId, StreamType};

  /// Test helper struct that exposes the partition logic for testing
  struct PartitionTester;

  impl PartitionTester {
    /// Expose the partition index calculation logic for testing
    /// This replicates the logic from MOQTClient::get_partition_index
    fn get_partition_index(stream_id: &StreamId) -> usize {
      let value = match stream_id.stream_type {
        StreamType::Fetch => {
          // Use a simple hash combining track_alias and fetch_request_id
          stream_id
            .track_alias
            .wrapping_add(stream_id.fetch_request_id.unwrap_or(0).wrapping_mul(13))
        }
        StreamType::Subgroup => {
          // Better distribution using prime number multipliers
          stream_id
            .track_alias
            .wrapping_add(stream_id.group_id.unwrap_or(0).wrapping_mul(17))
            .wrapping_add(stream_id.subgroup_id.unwrap_or(0).wrapping_mul(31))
        }
      };

      // Convert to bytes for fnv_hash function
      let value_bytes = value.to_le_bytes();
      (utils::fnv_hash(&value_bytes) % SEND_STREAM_PARTITION_COUNT as u64) as usize
    }
  }

  /// Helper function to create a Fetch stream ID
  fn create_fetch_stream_id(track_alias: u64, fetch_request_id: u64) -> StreamId {
    StreamId {
      stream_type: StreamType::Fetch,
      track_alias,
      group_id: None,
      subgroup_id: None,
      fetch_request_id: Some(fetch_request_id),
    }
  }

  /// Helper function to create a Subgroup stream ID
  fn create_subgroup_stream_id(
    track_alias: u64,
    group_id: Option<u64>,
    subgroup_id: Option<u64>,
  ) -> StreamId {
    StreamId {
      stream_type: StreamType::Subgroup,
      track_alias,
      group_id,
      subgroup_id,
      fetch_request_id: None,
    }
  }

  #[test]
  fn test_get_partition_index_fetch_streams() {
    // Test basic fetch stream partitioning
    let stream_id1 = create_fetch_stream_id(100, 1);
    let partition1 = PartitionTester::get_partition_index(&stream_id1);
    assert!(
      partition1 < SEND_STREAM_PARTITION_COUNT,
      "Partition index should be within bounds"
    );

    let stream_id2 = create_fetch_stream_id(100, 2);
    let partition2 = PartitionTester::get_partition_index(&stream_id2);
    assert!(
      partition2 < SEND_STREAM_PARTITION_COUNT,
      "Partition index should be within bounds"
    );

    // Different fetch request IDs should potentially give different partitions
    // (though not guaranteed due to hash collisions)
    let stream_id3 = create_fetch_stream_id(100, 1000);
    let partition3 = PartitionTester::get_partition_index(&stream_id3);
    assert!(
      partition3 < SEND_STREAM_PARTITION_COUNT,
      "Partition index should be within bounds"
    );
  }

  #[test]
  fn test_get_partition_index_fetch_streams_consistency() {
    // Test that the same inputs always produce the same output
    let stream_id = create_fetch_stream_id(42, 123);
    let partition1 = PartitionTester::get_partition_index(&stream_id);
    let partition2 = PartitionTester::get_partition_index(&stream_id);
    assert_eq!(
      partition1, partition2,
      "Same input should always produce same partition"
    );
  }

  #[test]
  fn test_get_partition_index_fetch_streams_edge_cases() {
    // Test with fetch_request_id = None (should default to 0)
    let mut stream_id = create_fetch_stream_id(100, 1);
    stream_id.fetch_request_id = None;
    let partition = PartitionTester::get_partition_index(&stream_id);
    assert!(
      partition < SEND_STREAM_PARTITION_COUNT,
      "Partition index should be within bounds"
    );

    // Test with large values
    let stream_id_large = create_fetch_stream_id(u64::MAX, u64::MAX);
    let partition_large = PartitionTester::get_partition_index(&stream_id_large);
    assert!(
      partition_large < SEND_STREAM_PARTITION_COUNT,
      "Partition index should be within bounds for large values"
    );

    // Test with zero values
    let stream_id_zero = create_fetch_stream_id(0, 0);
    let partition_zero = PartitionTester::get_partition_index(&stream_id_zero);
    assert!(
      partition_zero < SEND_STREAM_PARTITION_COUNT,
      "Partition index should be within bounds for zero values"
    );
  }

  #[test]
  fn test_get_partition_index_subgroup_streams() {
    // Test basic subgroup stream partitioning
    let stream_id1 = create_subgroup_stream_id(100, Some(1), Some(1));
    let partition1 = PartitionTester::get_partition_index(&stream_id1);
    assert!(
      partition1 < SEND_STREAM_PARTITION_COUNT,
      "Partition index should be within bounds"
    );

    let stream_id2 = create_subgroup_stream_id(100, Some(2), Some(1));
    let partition2 = PartitionTester::get_partition_index(&stream_id2);
    assert!(
      partition2 < SEND_STREAM_PARTITION_COUNT,
      "Partition index should be within bounds"
    );

    let stream_id3 = create_subgroup_stream_id(200, Some(1), Some(1));
    let partition3 = PartitionTester::get_partition_index(&stream_id3);
    assert!(
      partition3 < SEND_STREAM_PARTITION_COUNT,
      "Partition index should be within bounds"
    );
  }

  #[test]
  fn test_get_partition_index_subgroup_streams_consistency() {
    // Test that the same inputs always produce the same output
    let stream_id = create_subgroup_stream_id(42, Some(7), Some(13));
    let partition1 = PartitionTester::get_partition_index(&stream_id);
    let partition2 = PartitionTester::get_partition_index(&stream_id);
    assert_eq!(
      partition1, partition2,
      "Same input should always produce same partition"
    );
  }

  #[test]
  fn test_get_partition_index_subgroup_streams_none_values() {
    // Test with None group_id and subgroup_id (should default to 0)
    let stream_id1 = create_subgroup_stream_id(100, None, None);
    let partition1 = PartitionTester::get_partition_index(&stream_id1);
    assert!(
      partition1 < SEND_STREAM_PARTITION_COUNT,
      "Partition index should be within bounds"
    );

    // Test with Some group_id and None subgroup_id
    let stream_id2 = create_subgroup_stream_id(100, Some(5), None);
    let partition2 = PartitionTester::get_partition_index(&stream_id2);
    assert!(
      partition2 < SEND_STREAM_PARTITION_COUNT,
      "Partition index should be within bounds"
    );

    // Test with None group_id and Some subgroup_id
    let stream_id3 = create_subgroup_stream_id(100, None, Some(3));
    let partition3 = PartitionTester::get_partition_index(&stream_id3);
    assert!(
      partition3 < SEND_STREAM_PARTITION_COUNT,
      "Partition index should be within bounds"
    );
  }

  #[test]
  fn test_get_partition_index_subgroup_streams_large_values() {
    // Test with large values to ensure no overflow
    let stream_id_large = create_subgroup_stream_id(u64::MAX, Some(u64::MAX), Some(u64::MAX));
    let partition_large = PartitionTester::get_partition_index(&stream_id_large);
    assert!(
      partition_large < SEND_STREAM_PARTITION_COUNT,
      "Partition index should be within bounds for large values"
    );

    // Test with values that might cause overflow in naive implementations
    let stream_id_overflow =
      create_subgroup_stream_id(u64::MAX / 2, Some(u64::MAX / 3), Some(u64::MAX / 5));
    let partition_overflow = PartitionTester::get_partition_index(&stream_id_overflow);
    assert!(
      partition_overflow < SEND_STREAM_PARTITION_COUNT,
      "Partition index should be within bounds for overflow-prone values"
    );
  }

  #[test]
  fn test_get_partition_index_distribution_quality() {
    let mut distribution = [0; SEND_STREAM_PARTITION_COUNT];
    let test_count = 1000usize;

    // Test distribution quality for fetch streams
    for i in 0..test_count {
      let stream_id = create_fetch_stream_id((i % 10) as u64, i as u64);
      let partition = PartitionTester::get_partition_index(&stream_id);
      distribution[partition] += 1;
    }

    // Check that distribution is reasonably even (no partition should be empty or overly full)
    let expected_per_partition = test_count / SEND_STREAM_PARTITION_COUNT;
    let tolerance = expected_per_partition / 2; // Allow 50% deviation

    for (i, &count) in distribution.iter().enumerate() {
      assert!(
        count > 0,
        "Partition {} should have at least one entry for good distribution",
        i
      );
      assert!(
        count < expected_per_partition + tolerance,
        "Partition {} has too many entries ({}), expected around {}",
        i,
        count,
        expected_per_partition
      );
      // print the distribution
      println!("Partition {}: {}", i, count);
    }
  }

  #[test]
  fn test_get_partition_index_subgroup_distribution_quality() {
    let mut distribution = [0; SEND_STREAM_PARTITION_COUNT];
    let test_count = 1000usize;

    // Test distribution quality for subgroup streams
    for i in 0..test_count {
      let stream_id = create_subgroup_stream_id(
        (i % 10) as u64,       // track_alias
        Some((i / 10) as u64), // group_id
        Some((i % 5) as u64),  // subgroup_id
      );
      let partition = PartitionTester::get_partition_index(&stream_id);
      distribution[partition] += 1;
    }

    // Check that distribution is reasonably even
    let expected_per_partition = test_count / SEND_STREAM_PARTITION_COUNT;
    let tolerance = expected_per_partition / 2; // Allow 50% deviation

    for (i, &count) in distribution.iter().enumerate() {
      assert!(
        count > 0,
        "Partition {} should have at least one entry for good distribution",
        i
      );
      assert!(
        count < expected_per_partition + tolerance,
        "Partition {} has too many entries ({}), expected around {}",
        i,
        count,
        expected_per_partition
      );
      // print out the distribution
      println!("Partition {}: {}", i, count);
    }
  }

  #[test]
  fn test_get_partition_index_different_stream_types() {
    // Test that fetch and subgroup streams with similar parameters can have different partitions
    let fetch_stream = create_fetch_stream_id(100, 50);
    let subgroup_stream = create_subgroup_stream_id(100, Some(50), Some(0));

    let fetch_partition = PartitionTester::get_partition_index(&fetch_stream);
    let subgroup_partition = PartitionTester::get_partition_index(&subgroup_stream);

    // Both should be within bounds
    assert!(fetch_partition < SEND_STREAM_PARTITION_COUNT);
    assert!(subgroup_partition < SEND_STREAM_PARTITION_COUNT);

    // They should potentially be different (though not guaranteed due to hashing)
    // This test mainly ensures both algorithms work correctly
  }

  #[test]
  fn test_get_partition_index_deterministic() {
    let test_cases = vec![
      create_fetch_stream_id(42, 123),
      create_subgroup_stream_id(42, Some(123), Some(456)),
      create_subgroup_stream_id(0, None, None),
      create_fetch_stream_id(u64::MAX, 0),
    ];

    for stream_id in test_cases {
      let partition1 = PartitionTester::get_partition_index(&stream_id);
      let partition2 = PartitionTester::get_partition_index(&stream_id);
      assert_eq!(
        partition1, partition2,
        "Multiple calls should produce same partition for same stream_id"
      );
    }
  }
}
