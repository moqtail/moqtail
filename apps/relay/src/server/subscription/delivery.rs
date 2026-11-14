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

//! What a subscription does with each event from its track: the filters and forward state an Object passes before it is written.

use super::stream_writer::for_mid_subgroup_stream;
use super::subscription_manager::EventTimes;
use super::{DEFAULT_PUBLISHER_PRIORITY, Subscription};
use crate::server::inspect;
use crate::server::message_handlers::fetch_handler::FetchStop;
use crate::server::stream_id::StreamId;
use crate::server::track::TrackEvent;
use crate::server::utils;
use moqtail::model::common::location::Location;
use moqtail::model::control::constant::PublishDoneStatusCode;
use moqtail::model::data::datagram::Datagram;
use moqtail::model::data::object::Object;
use moqtail::model::error::StreamResetCode;
use moqtail::model::parameter::combined_filter::ObjectFields;
use moqtail::transport::data_stream_handler::HeaderInfo;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use tracing::{debug, error, info, trace, warn};

impl Subscription {
  /// Whether `group` is skipped for this subscriber: its Objects were dropped from
  /// the queue, or this one waited past the delivery timeout. A late Object skips
  /// the rest of its group, so delivery picks up again at the start of a later one.
  pub(super) fn skips_group(
    &self,
    group: u64,
    queued_at: Instant,
    timeout: Option<Duration>,
  ) -> bool {
    if self.queue.skips(group) {
      return true;
    }
    let Some(timeout) = timeout else {
      return false;
    };
    let waited = queued_at.elapsed();
    if waited <= timeout {
      return false;
    }
    if let Some(c) = inspect::counters() {
      c.dropped_delivery_timeout.add(1);
    }
    if self.queue.skip_group(group) {
      warn!(
        "Subscriber {} is {:?} behind on relay_track_id={}; skipping the rest of group {}",
        self.client_connection_id, waited, self.relay_track_id, group
      );
    }
    true
  }

  /// Counts a delivery and records its processing time, when the subscription is
  /// measured.
  fn record_written(
    &self,
    times: EventTimes,
    picked: Option<Instant>,
    location: &Location,
    payload_bytes: usize,
  ) {
    if let Some(c) = inspect::counters() {
      c.object_sent(payload_bytes);
    }
    if let Some(zone) = &self.zone {
      zone.object_out(payload_bytes);
    }
    if let (Some(stats), Some(picked)) = (&self.stats, picked) {
      stats.object_written(
        times.arrived,
        times.queued,
        picked,
        location.group,
        location.object,
      );
    }
  }

  pub(super) async fn handle_track_event(&self, times: EventTimes, event: TrackEvent) {
    // Read the clock only when the delivery is measured.
    let picked = self.stats.as_ref().map(|_| Instant::now());
    trace!(
      "Event received for subscriber={} relay_track_id={} event: {:?}",
      self.client_connection_id, self.relay_track_id, event
    );
    match event {
      TrackEvent::SubgroupObject {
        object,
        stream_id,
        header_info,
      } => {
        self
          .on_subgroup_object(times, picked, object, stream_id, header_info)
          .await
      }
      TrackEvent::Datagram { object } => self.on_datagram(times, picked, object).await,
      TrackEvent::StreamClosed { stream_id } => {
        debug!(
          "Received StreamClosed event: subscriber={} stream_id={} relay_track_id={}",
          self.client_connection_id, stream_id, self.relay_track_id
        );
        let _ = self.handle_stream_closed(&stream_id).await;
      }
      TrackEvent::PublisherDisconnected {
        status_code,
        reason,
      } => {
        info!(
          "Received PublisherDisconnected event: subscriber={}, status={:?} reason={} relay_track_id={}",
          self.client_connection_id, status_code, reason, self.relay_track_id
        );

        // Send PublishDone message and finish the subscription
        if let Err(e) = self.send_publish_done(status_code, &reason).await {
          error!(
            "Failed to send PublishDone for publisher disconnect: subscriber={} relay_track_id={} error: {:?}",
            self.client_connection_id, self.relay_track_id, e
          );
        }

        if status_code == PublishDoneStatusCode::MalformedTrack {
          self.stop_fill_streams(FetchStop::MalformedTrack).await;
        }

        // Finish the subscription since the publisher is gone
        self.finish().await;
      }
    }
  }

  async fn on_subgroup_object(
    &self,
    times: EventTimes,
    picked: Option<Instant>,
    object: Arc<Object>,
    stream_id: StreamId,
    header_info: Option<HeaderInfo>,
  ) {
    // SSTS: when this track belongs to a switching set, only the track
    // the allocation picked for this group is forwarded. The answer holds
    // for the whole group, so the gate caches it and reads the shared
    // state once per group instead of once per Object. A connection
    // without SSTS pays one `Vec::is_empty`.
    //
    // The gate assumes a subscriber drives switching one way or the other,
    // never both: SSTS picks the track here, or the subscriber asks for a
    // SWITCH_FROM. The two do not compose as written. A dropped Object is
    // invisible to the checks below, so a soft drain waiting for the Object
    // past its end group never sees one the gate swallowed and never
    // finishes.
    //
    // TODO: decide how the two should interact before enabling SSTS on a
    // connection that also switches with SWITCH_FROM. Likely the drain has
    // to end on something other than an Object arriving, so that it does
    // not depend on objects the gate may drop.
    if self.subscriber.ssts_enabled()
      && !self
        .ssts_gate
        .allows(&self.subscriber.ssts, object.location.group)
        .await
    {
      return;
    }

    // update last received object location
    {
      let mut state = self.subscription_state.write().await;
      state.update_last_received_object_location(object.location.clone());
    }

    let object_received_time = utils::passed_time_since_start();

    {
      let state = self.subscription_state.read().await;
      if let Some(start) = &state.start_location
        && object.location < *start
      {
        trace!(
          "Object before start location for subscriber={} relay_track_id={} object location: {:?} start location: {:?}",
          self.client_connection_id, self.relay_track_id, object.location, start
        );
        // A later object of this stream may be past the start location.
        self.save_skipped_header(&stream_id, header_info).await;
        return;
      }

      if state.is_past_end(&object.location)
      // TODO: The following code is commented out. The suspend track is stopped at the
      // end group. However, if the SUBSCRIBE comes late (after the group boundary)
      // the first object (keyframe) would have been sent out and then the relay
      // stops sending this group. In that case awaiting switch activation seems
      // like a good idea but then it also does not work well.
      // For now, we commented it out and will figure out a new approach in the future.
      // && !state.awaiting_switch_activation()
      {
        trace!(
          "Object beyond end group for subscriber={} relay_track_id={} object location: {:?} end group: {:?}",
          self.client_connection_id, self.relay_track_id, object.location, state.end_group
        );
        let drained = state.soft_suspended;
        drop(state);
        // An object past the boundary is how a drain ends: the group before it
        // has nothing more to give.
        if drained {
          self.finish_soft_drain().await;
        }
        return;
      }

      if !state.forward {
        // Cache the subgroup header so we can open the stream immediately
        // if forward transitions to true mid-group (sub-update-forward method).
        if let Some(ref header) = header_info {
          let mut pending = self.pending_header.lock().await;
          *pending = Some((stream_id.clone(), header.clone()));
        }
        return;
      }

      // Checked last, so a drain still sees the object past its boundary.
      //
      // A filtered object opens no stream. If a later object of the stream
      // passes, the stream is opened then (see `skipped_stream_headers`).
      // Skipping objects keeps object ID encoding correct, because each ID is
      // encoded against the last object actually written to the stream. A Prior
      // Object ID Gap is forwarded as is: it counts IDs that were never
      // published, not ones filtered out.
      if !state.range_filter.matches_object(&object) {
        trace!(
          "Object filtered out for subscriber={} relay_track_id={} location: {:?} subgroup: {:?} priority: {}",
          self.client_connection_id,
          self.relay_track_id,
          object.location,
          object.subgroup_id,
          object.publisher_priority
        );
        // A later object of this stream may pass the filter.
        self.save_skipped_header(&stream_id, header_info).await;
        return;
      }

      let timeout = state.delivery_timeout(self.config.delivery_timeout);
      if self.skips_group(object.location.group, times.queued, timeout) {
        drop(state);
        // The stream's Objects so far went out, but the rest of the group won't.
        if self
          .send_stream_last_object_ids
          .write()
          .await
          .remove(&stream_id)
          .is_some()
        {
          self
            .subscriber
            .reset_stream(
              &self.new_send_stream_key(&stream_id),
              StreamResetCode::DeliveryTimeout.to_u64(),
            )
            .await;
        }
        self.skipped_stream_headers.lock().await.remove(&stream_id);
        return;
      }
    }

    /* TODO: Look at the comment about awaiting_switch_activation.
    // The following code block is commented out with the same reason.
    // This is the switch's first delivery. The track being switched away from
    // keeps sending past its boundary while it waits for exactly this, so the two
    // can now overlap, and the seam has to be decided here.
    if self.pending_switch.swap(false, Ordering::Relaxed)
      && let Some(plan) = self
        .subscriber
        .switch_context
        .get_plan(self.request_id)
        .await
      && let Some(suspending) = self.find_suspending(&plan).await
    {
      let (suspend_last_loc, suspend_group_gap) = {
        let state = suspending.read().await;
        let state = state.subscription_state.read().await;
        (state.last_sent_max_location.clone(), state.sent_group_gap())
      };

      let mut stop = false;

      if let Some(sus_last_loc) = suspend_last_loc
        && sus_last_loc >= object.location
      {
        // The suspending track got there first, so it keeps the seam and this one
        // takes over after it. Two distances decide where: what the suspending
        // track has covered runs to the next id it would use, not to the one
        // after the id it reached, and this track can only resume at one of its
        // own group ids. So the takeover is the first group of this track that
        // the suspending one no longer covers -- and whatever lies between the
        // two stays with the suspending track, which is the one with groups
        // there to fill it.
        let suspend_gap = suspend_group_gap.unwrap_or(1);
        let resume_gap = self
          .subscription_state
          .read()
          .await
          .received_group_gap()
          .unwrap_or(1);
        let covered_to = sus_last_loc.group + suspend_gap;
        let takeover = takeover_group(object.location.group, covered_to, resume_gap);

        info!(
          "switch: {:?} takes over from {:?} at group {} for subscriber {}; it had reached {:?} covering to {} ({} apart), resuming {} apart",
          self.full_track_name,
          plan.suspending,
          takeover,
          self.client_connection_id,
          sus_last_loc,
          covered_to,
          suspend_gap,
          resume_gap
        );

        self.subscription_state.write().await.start_location = Some(Location::new(takeover, 0));

        {
          let suspending = suspending.read().await;
          let mut state = suspending.subscription_state.write().await;
          state.end_group = Some(takeover - 1);
          state.end_object = None;
        }

        stop = true;
      }

      // The switch has happened: this releases the suspending track's boundary,
      // which was held until there was something to switch to.
      self.complete_switch(object.location.group).await;

      if stop {
        return;
      }
    }
    */

    // Entering forward=true: clear any stale pending header (group boundary case).
    // If forward was already true, pending_header is None and this is a no-op.
    {
      let mut pending = self.pending_header.lock().await;
      pending.take();
    }

    // Handle header info if this is the first object
    let send_stream = if let Some(header) = header_info {
      if let HeaderInfo::Subgroup { header: _ } = header {
        debug!(
          "Creating stream - subscriber={} relay_track_id={} now={} received time={} object: {:?} header: {:?}",
          self.client_connection_id,
          self.relay_track_id,
          utils::passed_time_since_start(),
          object_received_time,
          object.location,
          header
        );
        if let Ok((stream_id, send_stream)) = self.handle_header(header.clone()).await {
          {
            let mut send_stream_last_object_ids = self.send_stream_last_object_ids.write().await;
            send_stream_last_object_ids.insert(stream_id.clone(), None);
          }
          debug!(
            "Stream created - subscriber={} stream_id={} relay_track_id={} now={} received time={} object: {:?}",
            self.client_connection_id,
            stream_id,
            self.relay_track_id,
            utils::passed_time_since_start(),
            object_received_time,
            object.location
          );
          Some(send_stream)
        } else {
          // TODO: maybe log error here?
          None
        }
      } else {
        error!(
          "Received Object event with non-subgroup header: {:?}",
          header
        );
        None
      }
    } else {
      match self
        .subscriber
        .get_stream(&self.new_send_stream_key(&stream_id))
        .await
      {
        Some(s) => Some(s),
        None => {
          // No send stream yet, so open one with the stream's header. Either
          // this subscription skipped the stream's first object, or it
          // subscribed after the stream had started.
          let held = self.skipped_stream_headers.lock().await.remove(&stream_id);
          let cached = match held {
            Some(h) => Some(h),
            None => self
              .active_subgroup_headers
              .read()
              .await
              .get(&stream_id)
              .cloned(),
          };
          if let Some(h) = cached {
            trace!(
              "mid-subgroup join: opening stream from cached header for subscriber={} relay_track_id={} stream_id={}",
              self.client_connection_id, self.relay_track_id, stream_id
            );
            self
              .handle_header(for_mid_subgroup_stream(h))
              .await
              .ok()
              .map(|(_, send_stream)| send_stream)
          } else {
            None
          }
        }
      }
    };

    if let Some(send_stream) = send_stream {
      // Get the previous object ID for this stream
      let previous_object_id = {
        let send_stream_last_object_ids = self.send_stream_last_object_ids.read().await;
        send_stream_last_object_ids
          .get(&stream_id)
          .cloned()
          .flatten()
      };

      trace!(
        "Received Object event: subscriber={} stream_id={} relay_track_id={} previous_object_id: {:?} object: {:?} now={} received time={}",
        self.client_connection_id,
        stream_id,
        self.relay_track_id,
        previous_object_id,
        object,
        utils::passed_time_since_start(),
        object_received_time
      );

      // Log object properties with send status if enabled
      let write_result = self
        .handle_object(&object, previous_object_id, &stream_id, send_stream.clone())
        .await;
      let send_status = write_result.is_ok();

      // Update the last object ID for this stream if successful
      if send_status {
        let payload_bytes = object.payload.as_ref().map_or(0, |p| p.len());
        self.record_written(times, picked, &object.location, payload_bytes);
        let mut send_stream_last_object_ids = self.send_stream_last_object_ids.write().await;
        send_stream_last_object_ids.insert(stream_id.clone(), Some(object.location.object));
        drop(send_stream_last_object_ids); // Release the lock immediately

        // Update last sent max location
        self
          .subscription_state
          .write()
          .await
          .update_last_sent_max_location(object.location.clone());

        // The switch's first delivery: the switch has happened, so a hard one
        // cuts the subscription it suspends.
        if self.pending_switch.swap(false, Ordering::Relaxed) {
          self.complete_switch(object.location.group).await;
        }
      }

      if self.config.enable_object_logging {
        self
          .object_logger
          .log_subscription_object(
            self.relay_track_id,
            self.client_connection_id,
            &object,
            send_status,
            object_received_time,
          )
          .await;
      }
    } else {
      error!(
        "Received Object event without a valid send stream for subscriber={} stream_id={} relay_track_id={} object: {:?} now={} received time={}",
        self.client_connection_id,
        stream_id,
        self.relay_track_id,
        object.location,
        utils::passed_time_since_start(),
        object_received_time
      );
    }
  }

  async fn on_datagram(&self, times: EventTimes, picked: Option<Instant>, object: Arc<Datagram>) {
    // Handle datagram - serialize full MOQT datagram format
    // Must include type, track_alias, group_id, object_id, publisher_priority, and payload

    let location = Location::new(object.group_id, object.object_id);
    {
      let mut state = self.subscription_state.write().await;
      state.update_last_received_object_location(location.clone());
    }

    // A datagram is subject to the same filter and forward state as any other
    // object. There is no stream to hold open, so nothing is cached for a later
    // forward transition: a datagram missed while paused is simply missed.
    {
      let state = self.subscription_state.read().await;
      if let Some(start) = &state.start_location
        && location < *start
      {
        trace!(
          "Datagram before start location for subscriber={} relay_track_id={} object location: {:?} start location: {:?}",
          self.client_connection_id, self.relay_track_id, location, start
        );
        return;
      }

      if state.is_past_end(&location) && !state.awaiting_switch_activation() {
        trace!(
          "Datagram beyond end group for subscriber={} relay_track_id={} object location: {:?} end group: {:?}",
          self.client_connection_id, self.relay_track_id, location, state.end_group
        );
        return;
      }

      if !state.forward {
        trace!(
          "Not forwarding datagram for subscriber={} relay_track_id={}: forward state is 0",
          self.client_connection_id, self.relay_track_id
        );
        return;
      }

      // A datagram has no subgroup, so it passes no subgroup filter.
      let fields = ObjectFields {
        subgroup_id: None,
        object_id: object.object_id,
        publisher_priority: object
          .publisher_priority
          .unwrap_or(DEFAULT_PUBLISHER_PRIORITY),
        properties: object.properties.as_deref(),
      };
      if !state.range_filter.matches(fields) {
        trace!(
          "Datagram filtered out for subscriber={} relay_track_id={} location: {:?}",
          self.client_connection_id, self.relay_track_id, location
        );
        return;
      }

      let timeout = state.delivery_timeout(self.config.delivery_timeout);
      if self.skips_group(location.group, times.queued, timeout) {
        return;
      }
    }

    let mut norm_object = (*object).clone();
    norm_object.track_alias = self.track_alias;

    match norm_object.serialize() {
      Ok(serialized_bytes) => {
        if let Err(e) = self
          .subscriber
          .write_datagram_object(serialized_bytes)
          .await
        {
          error!("Failed to write datagram: {:?}", e);
        } else {
          let payload_bytes = object.payload.as_ref().map_or(0, |p| p.len());
          self.record_written(times, picked, &location, payload_bytes);
          self
            .subscription_state
            .write()
            .await
            .update_last_sent_max_location(location.clone());
          if self.pending_switch.swap(false, Ordering::Relaxed) {
            self.complete_switch(location.group).await;
          }
        }
      }
      Err(e) => {
        error!("Failed to serialize datagram: {:?}", e);
      }
    }
  }
}
