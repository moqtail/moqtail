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

//! Objects arriving from a publisher: deduplicated, cached, counted in the Largest Location, then fanned out.

use super::{Track, TrackEvent};
use crate::server::stream_id::StreamId;
use crate::server::utils;
use anyhow::Result;
use moqtail::model::common::location::Location;
use moqtail::model::control::constant::PublishDoneStatusCode;
use moqtail::model::data::constant::ObjectStatus;
use moqtail::model::data::datagram::Datagram;
use moqtail::model::data::object::Object;
use moqtail::model::property::object_property::ObjectProperty;
use moqtail::transport::data_stream_handler::HeaderInfo;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use tracing::{debug, error, info, trace};

impl Track {
  /// Caches an Object an upstream FETCH brought in. It is not handed to subscribers,
  /// which get the live feed, nor checked against the Objects already seen: a group
  /// evicted from the cache has to be fetchable into it again.
  pub async fn cache_fetched_object(&self, object: &Object) {
    if let Ok(fetch_object) = object.clone().try_into_fetch() {
      self.cache.add_object(fetch_object).await;
    }
    self.record_arrival(object).await;
  }

  /// Logs an Object's arrival and moves the Largest Location on to it.
  pub(super) async fn record_arrival(&self, object: &Object) {
    if self.config.enable_object_logging {
      self
        .object_logger
        .log_track_object(
          self.relay_track_id,
          object,
          utils::passed_time_since_start(),
        )
        .await;
    }
    {
      let mut largest_location = self.largest_location.write().await;
      if object.location > *largest_location {
        *largest_location = object.location.clone();
      }
    }
    self.has_objects.store(true, Ordering::Relaxed);
  }

  /// Records what a live Object says does not exist, so the cache can serve a group
  /// once it accounts for all of it: what an End of Group or End of Track status
  /// rules out, what follows an Object that ends its group, and the IDs a Prior Group
  /// or Object ID Gap skips.
  pub(super) fn note_absent(&self, object: &Object, ends_its_group: bool) {
    let Location { group, object: id } = object.location.clone();
    let group_end = Location::new(group, u64::MAX);
    match object.status {
      ObjectStatus::EndOfGroup => self.cache.mark_absent(object.location.clone(), group_end),
      ObjectStatus::EndOfTrack => self
        .cache
        .mark_absent(object.location.clone(), Location::new(u64::MAX, u64::MAX)),
      ObjectStatus::Normal => {}
    }
    if ends_its_group {
      self.group_ended_after(&object.location);
    }
    for property in object.properties.iter().flatten() {
      match property {
        ObjectProperty::PriorObjectIdGap { gap } if *gap > 0 && *gap <= id => {
          self
            .cache
            .mark_absent(Location::new(group, id - gap), Location::new(group, id - 1));
        }
        ObjectProperty::PriorGroupIdGap { gap } if *gap > 0 && *gap <= group => {
          self.cache.mark_absent(
            Location::new(group - gap, 0),
            Location::new(group - 1, u64::MAX),
          )
        }
        _ => {}
      }
    }
  }

  /// Called when a subgroup stream that holds the end of its group closes with a FIN:
  /// nothing in the group comes after the last Object it carried.
  pub fn group_ended_after(&self, last: &Location) {
    if last.object < u64::MAX {
      self
        .cache
        .mark_absent(last.next().unwrap(), Location::new(last.group, u64::MAX));
    }
  }

  pub async fn new_subgroup_object(
    &self,
    stream_id: &StreamId,
    object: &Object,
    header_info: Option<&HeaderInfo>,
  ) -> Result<()> {
    let arrived = crate::server::inspect::arrival_time();
    trace!(
      "new_subgroup_object: relay_track_id={} location: {:?} stream_id={} diff_ms={}",
      self.relay_track_id,
      object.location,
      stream_id,
      utils::passed_time_since_start()
    );

    if self.is_duplicate(&object.location) {
      trace!(
        "new_subgroup_object: dropping duplicate | relay_track_id={} location: {:?}",
        self.relay_track_id, object.location
      );
      return Ok(());
    }

    if let Some(h) = header_info {
      debug!(
        "new group: relay_track_id={} location: {:?} stream_id={} time={}",
        self.relay_track_id,
        object.location,
        stream_id,
        utils::passed_time_since_start()
      );
      self
        .active_subgroup_headers
        .write()
        .await
        .insert(stream_id.clone(), h.clone());
    }

    // The cache cannot hold a status, so all that is kept of one is what it says does
    // not exist.
    if let Ok(fetch_object) = object.clone().try_into_fetch() {
      self.cache.add_object(fetch_object).await;
    }
    self.note_absent(object, false);
    // Before the fan-out: a subscriber joining now starts after the Largest Location,
    // so an Object it misses must already be counted in it.
    self.record_arrival(object).await;

    let event = TrackEvent::SubgroupObject {
      stream_id: stream_id.clone(),
      object: Arc::new(object.clone()),
      header_info: header_info.cloned(),
    };
    self
      .subscription_manager
      .send_object_event(event, arrived)
      .await?;
    Ok(())
  }

  /// The Publisher Priority to assume for this track's objects whose header omits
  /// the field. A publisher may name it with the DEFAULT_PUBLISHER_PRIORITY track
  /// property; without one, the protocol's own default stands.
  pub fn default_publisher_priority(&self) -> u8 {
    self.default_publisher_priority.load(Ordering::Relaxed)
  }

  pub async fn new_datagram(&self, datagram: &Datagram) -> Result<()> {
    let arrived = crate::server::inspect::arrival_time();
    self.datagram_arrivals.arrived();
    trace!(
      "new_datagram: relay_track_id={} group: {:?} object_id={} diff_ms={}",
      self.relay_track_id,
      datagram.group_id,
      datagram.object_id,
      utils::passed_time_since_start()
    );

    match Object::try_from_datagram(datagram.clone(), self.default_publisher_priority()) {
      Ok((object, end_of_group)) => {
        if self.is_duplicate(&object.location) {
          trace!(
            "new_datagram: dropping duplicate | relay_track_id={} location: {:?}",
            self.relay_track_id, object.location
          );
          return Ok(());
        }

        if end_of_group {
          trace!(
            "new_datagram: end_of_group received for track: {:?} group: {:?} object_id: {}",
            datagram.track_alias, datagram.group_id, datagram.object_id
          );
        }

        // The cache cannot hold a status, so all that is kept of one is what it says
        // does not exist.
        if let Ok(fetch_object) = object.clone().try_into_fetch() {
          self.cache.add_object(fetch_object).await;
        }
        self.note_absent(&object, end_of_group);
        self.record_arrival(&object).await;
      }
      Err(e) => {
        error!(
          "Failed to convert datagram to object for logging: group: {:?} object_id={} error={}",
          datagram.group_id, datagram.object_id, e
        );
      }
    }

    let event = TrackEvent::Datagram {
      object: Arc::new(datagram.clone()),
    };

    self
      .subscription_manager
      .send_object_event(event, arrived)
      .await?;

    Ok(())
  }

  pub async fn stream_closed(&self, stream_id: &StreamId) -> Result<()> {
    self.active_subgroup_headers.write().await.remove(stream_id);

    let event = TrackEvent::StreamClosed {
      stream_id: stream_id.clone(),
    };

    self
      .subscription_manager
      .send_event_to_subscribers(event)
      .await?;

    Ok(())
  }

  /// Send PublisherDisconnected event to all subscribers.
  /// Called internally by remove_publisher() when the last publisher leaves.
  pub async fn notify_publisher_disconnected(&self) -> Result<()> {
    info!(
      "All publishers gone for relay_track_id={} - notifying all subscribers",
      self.relay_track_id
    );

    self
      .notify_publish_done(
        PublishDoneStatusCode::TrackEnded,
        "Publisher disconnected".to_string(),
      )
      .await
  }

  /// Fan a PUBLISH_DONE out to all subscribers with the given status/reason.
  /// Used when an upstream publisher signals it is done for the track, so the
  /// upstream's status code is relayed downstream verbatim rather than being
  /// flattened to a generic disconnect.
  pub async fn notify_publish_done(
    &self,
    status_code: PublishDoneStatusCode,
    reason: String,
  ) -> Result<()> {
    let event = TrackEvent::PublisherDisconnected {
      status_code,
      reason,
    };

    self
      .subscription_manager
      .send_event_to_subscribers(event)
      .await?;

    Ok(())
  }
}
