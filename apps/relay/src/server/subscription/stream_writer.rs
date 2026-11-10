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

//! Writing a subscription's Objects onto its QUIC streams: opening, writing, closing, resetting.

use super::{DEFAULT_PUBLISHER_PRIORITY, Subscription};
use crate::server::stream_id::{SendStreamKey, StreamId};
use crate::server::utils;
use anyhow::Result;
use bytes::Bytes;
use moqtail::model::data::object::Object;
use moqtail::transport::connection::TransportSendStream;
use moqtail::transport::data_stream_handler::HeaderInfo;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use tokio::sync::Mutex;
use tracing::{debug, error, trace, warn};

/// The header for a downstream stream that opens after the subgroup's first object
/// has gone by, from the header that arrived with that object. Upstream set its bits
/// for the object it came with, which this stream does not start with.
pub(super) fn for_mid_subgroup_stream(header_info: HeaderInfo) -> HeaderInfo {
  match header_info {
    HeaderInfo::Subgroup { header } => HeaderInfo::Subgroup {
      header: header.for_mid_subgroup_stream(),
    },
    other => other,
  }
}

impl Subscription {
  /// Reset every data stream this subscription has opened with the given code.
  /// Resets every open data stream, draining the map so `finish` does not then try to
  /// close a stream that has already been reset.
  pub(super) async fn reset_data_streams(&self, code: u64) {
    let stream_ids: Vec<StreamId> = {
      let mut map = self.send_stream_last_object_ids.write().await;
      map.drain().map(|(stream_id, _)| stream_id).collect()
    };
    for stream_id in stream_ids {
      self
        .subscriber
        .reset_stream(&self.new_send_stream_key(&stream_id), code)
        .await;
    }
  }

  /// Saves the header of a stream whose first object was skipped. `header_info`
  /// is set only on a stream's first object.
  pub(super) async fn save_skipped_header(
    &self,
    stream_id: &StreamId,
    header_info: Option<HeaderInfo>,
  ) {
    if let Some(header) = header_info {
      self
        .skipped_stream_headers
        .lock()
        .await
        .insert(stream_id.clone(), header);
    }
  }

  pub(super) async fn handle_header(
    &self,
    header_info: HeaderInfo,
  ) -> Result<(StreamId, Arc<Mutex<TransportSendStream>>)> {
    // Handle the header information
    trace!("Handling header: {:?}", header_info);
    let stream_id = self.get_stream_id(&header_info);

    if let Ok(header_payload) = self.get_header_payload(&header_info).await {
      // hex dump the header payload
      trace!(
        "subscription::handle_object | header payload: {:?}",
        utils::bytes_to_hex(&header_payload)
      );

      let (pub_prio, group_id) = match &header_info {
        HeaderInfo::Subgroup { header } => (
          header
            .publisher_priority
            .unwrap_or(DEFAULT_PUBLISHER_PRIORITY),
          header.group_id,
        ),
        HeaderInfo::Fetch { .. } => (DEFAULT_PUBLISHER_PRIORITY, 0u64),
      };
      let (sub_prio, group_order) = {
        let state = self.subscription_state.read().await;
        (state.subscriber_priority, state.group_order)
      };
      let priority = utils::compute_stream_priority(sub_prio, pub_prio, group_order, group_id);

      let send_stream = match self
        .subscriber
        .open_stream(
          &self.new_send_stream_key(&stream_id),
          header_payload,
          priority,
        )
        .await
      {
        Ok(send_stream) => send_stream,
        Err(e) => {
          error!(
            "Failed to open stream {}: {:?} subscriber={} relay_track_id={}",
            stream_id, e, self.client_connection_id, self.relay_track_id
          );
          return Err(e);
        }
      };

      debug!("Created stream: {}", stream_id.get_stream_id());

      // Count every data stream opened for this subscription (PUBLISH_DONE
      // Stream Count), including subgroups that end up carrying no objects.
      self.opened_stream_count.fetch_add(1, Ordering::Relaxed);

      Ok((stream_id, send_stream.clone()))
    } else {
      error!(
        "Failed to serialize header payload for stream {} subscriber={} relay_track_id={}",
        stream_id, self.client_connection_id, self.relay_track_id
      );
      Err(anyhow::anyhow!(
        "Failed to serialize header payload for stream {} subscriber={} relay_track_id={}",
        stream_id,
        self.client_connection_id,
        self.relay_track_id
      ))
    }
  }

  pub(super) async fn handle_object(
    &self,
    object: &Object,
    previous_object_id: Option<u64>,
    stream_id: &StreamId,
    send_stream: Arc<Mutex<TransportSendStream>>,
  ) -> Result<()> {
    trace!(
      "Handling object relay_track_id={} location: {:?} stream_id={} diff_ms={}",
      self.relay_track_id,
      object.location,
      stream_id,
      utils::passed_time_since_start()
    );

    let object_location = object.location.clone();

    let has_properties = object.properties.is_some();
    let chunks = match object.serialize_subgroup_parts(previous_object_id, has_properties) {
      Ok((header, payload)) => [Some(header), payload].into_iter().flatten().collect(),
      Err(e) => {
        error!(
          "Error in serializing object before writing to stream for subscriber={} relay_track_id={}, location: {:?}, previous_object_id: {:?}, error: {:?}",
          self.client_connection_id, self.relay_track_id, object_location, previous_object_id, e
        );
        return Err(e.into());
      }
    };

    self
      .subscriber
      .write_stream_object(
        &self.new_send_stream_key(stream_id),
        object.location.object,
        chunks,
        Some(send_stream.clone()),
      )
      .await
      .map_err(|open_stream_err| {
        error!(
          "Error writing object to stream for subscriber={} relay_track_id={}, error: {:?}",
          self.client_connection_id, self.relay_track_id, open_stream_err
        );
        open_stream_err
      })
  }

  pub(super) async fn handle_stream_closed(&self, stream_id: &StreamId) -> Result<()> {
    // Handle the stream closed event
    trace!("Stream closed: {}", stream_id.get_stream_id());

    // remove the stream id from send_stream_last_object_ids immediately
    let mut send_stream_last_object_ids = self.send_stream_last_object_ids.write().await;
    send_stream_last_object_ids.remove(stream_id);
    drop(send_stream_last_object_ids); // Release the lock immediately

    // The header is still saved only if no object of this stream was sent.
    self.skipped_stream_headers.lock().await.remove(stream_id);

    // Perform graceful stream closure in a separate task to avoid blocking
    // the main subscription event loop. This is critical for real-time media streaming
    // where blocking operations can disrupt video flow timing (25fps = ~40ms intervals)
    let subscriber = self.subscriber.clone();
    let stream_id = self.new_send_stream_key(stream_id);
    let connection_id = self.client_connection_id;
    let relay_track_id = self.relay_track_id;

    tokio::spawn(async move {
      trace!(
        "Starting graceful stream closure in background: subscriber={} stream_id={} relay_track_id={}",
        connection_id, stream_id, relay_track_id
      );

      let res = subscriber.close_stream(&stream_id).await;
      if let Err(e) = res {
        warn!(
          "handle_stream_closed | error for subscriber={} stream_id={} relay_track_id={} error: {:?}",
          connection_id, stream_id, relay_track_id, e
        );
      } else if let Ok(closed) = res {
        if closed {
          trace!(
            "handle_stream_closed | successful for subscriber={} stream_id={} relay_track_id={}",
            connection_id, stream_id, relay_track_id
          );
        } else {
          trace!(
            "handle_stream_closed | stream not found for subscriber={} stream_id={} relay_track_id={}",
            connection_id, stream_id, relay_track_id
          );
        }
      }
    });

    // Return immediately to avoid blocking the event loop
    Ok(())
  }

  pub(super) async fn get_header_payload(&self, header_info: &HeaderInfo) -> Result<Bytes> {
    let connection_id = self.client_connection_id;
    match header_info {
      HeaderInfo::Subgroup { header } => header.serialize(Some(self.track_alias)).map_err(|e| {
        error!(
          "Error serializing subgroup header: {:?} subscriber={} relay_track_id={}",
          e, connection_id, self.relay_track_id
        );
        e.into()
      }),
      HeaderInfo::Fetch {
        header,
        fetch_request: _,
      } => header.serialize().map_err(|e| {
        error!(
          "Error serializing fetch header: {:?} subscriber={} relay_track_id={}",
          e, connection_id, self.relay_track_id
        );
        e.into()
      }),
    }
  }

  pub(super) fn get_stream_id(&self, header_info: &HeaderInfo) -> StreamId {
    utils::build_stream_id(self.relay_track_id, header_info)
  }

  /// This subscription's send streams are keyed by using request_id
  /// and stream_id to avoid stream naming collisions in the client
  pub(super) fn new_send_stream_key(&self, stream_id: &StreamId) -> SendStreamKey {
    SendStreamKey::new(self.request_id, stream_id.clone())
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use moqtail::model::data::subgroup_header::SubgroupHeader;

  #[test]
  fn a_stream_opened_mid_subgroup_states_the_subgroup_id_and_drops_first_object() {
    // What upstream sent with object 5: ID taken from that object, FIRST_OBJECT set.
    let mut upstream = SubgroupHeader::new_first_object_id(1, 9, Some(7), false, false, true);
    upstream.subgroup_id = Some(5);

    let HeaderInfo::Subgroup { header } =
      for_mid_subgroup_stream(HeaderInfo::Subgroup { header: upstream })
    else {
      panic!("a subgroup header stays a subgroup header");
    };
    assert!(!header.header_type.is_first_object());
    assert!(header.header_type.has_explicit_subgroup_id());

    // The downstream reader sees subgroup 5 although its first object is, say, 8.
    let mut buf = header.serialize(None).unwrap();
    assert_eq!(
      SubgroupHeader::deserialize(&mut buf).unwrap().subgroup_id,
      Some(5)
    );
  }
}
