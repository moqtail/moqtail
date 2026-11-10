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

//! Switching a subscriber from one track to another: suspending, draining, and taking over.

use super::Subscription;
use crate::server::message_handlers::fetch_handler::FetchStop;
use crate::server::session::client::switch_context::SwitchPlan;
use anyhow::Result;
use moqtail::model::common::location::Location;
use moqtail::model::control::constant::PublishDoneStatusCode;
use moqtail::model::control::constant::SwitchMode;
use moqtail::model::error::StreamResetCode;
use moqtail::model::parameter::location_filter::LocationFilter;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use tokio::sync::RwLock;
use tracing::{error, info, warn};

/// The first group of the track being switched to that the track being switched
/// away from no longer covers.
///
/// `resume_group` is the group the activating track has just started delivering,
/// `covered_to` the id where the suspending track's media runs out, and
/// `resume_gap` how far apart the activating track's own group ids run. A track
/// can only resume at one of its own groups, so this steps by that distance
/// rather than landing on an id it has no group at.
// TODO: Until switch trigger in subscription.rs solvedthis is dead code.
#[allow(dead_code)]
fn takeover_group(resume_group: u64, covered_to: u64, resume_gap: u64) -> u64 {
  let resume_gap = resume_gap.max(1);
  match covered_to.checked_sub(resume_group) {
    Some(ahead) if ahead > 0 => resume_group + resume_gap * ahead.div_ceil(resume_gap),
    _ => resume_group,
  }
}

impl Subscription {
  /// Schedules a switch on this, the subscription it activates.
  ///
  /// `filter` is applied here rather than only at creation, because a switch back
  /// to a track reuses the subscription that track already has, whose filter is
  /// the one from last time. A SUBSCRIBE describes a subscription in full and so
  /// always carries one; a REQUEST_UPDATE passes one only where it changes it.
  ///
  /// A Start Group is a boundary both sides can act on straight away: this
  /// subscription's filter drops everything before it, and the suspending one is
  /// told to stop there. Without one there is nothing to schedule, and the
  /// boundary becomes the first group this subscription delivers.
  pub async fn begin_switch(
    &self,
    mut plan: SwitchPlan,
    filter: Option<&LocationFilter>,
    largest: Option<Location>,
  ) {
    {
      let mut state = self.subscription_state.write().await;
      if let Some(filter) = filter {
        // A live filter says where to start only against the largest Object, and
        // that is the boundary the switch then happens at.
        state.set_location_filter(filter.clone(), largest);
      }
      // A switch activates the subscription it arrives on; SWITCH_FROM cannot be
      // combined with FORWARD, so nothing else decides this.
      state.forward = true;
      // Whatever a previous switch left behind is not this one's business.
      state.soft_suspended = false;
      state.publish_done_at_end = false;
      state.switch_activated = None;
      plan.boundary = state.start_location.as_ref().map(|start| start.group);
    }

    info!(
      "switch: {:?} activating for subscriber {} ({:?}, boundary {:?})",
      self.full_track_name, self.client_connection_id, plan.mode, plan.boundary
    );

    if let Some(boundary) = plan.boundary {
      self.suspend_at(&plan, boundary).await;
    }

    // A hard switch cuts on that first delivery, and a boundary nobody supplied
    // is discovered there too.
    self
      .subscriber
      .switch_context
      .set_plan(self.request_id, plan)
      .await;
    self.pending_switch.store(true, Ordering::Relaxed);
  }

  /// Tells the subscription this switch suspends where to stop.
  ///
  /// Soft leaves it delivering up to the group before the boundary, so the two
  /// tracks meet without a hole, and leaves its streams — fill fetch streams
  /// included — to finish. Hard is not scheduled: it is cut in
  /// [`Self::cut_suspending`] when the activating subscription first delivers,
  /// the earliest moment at which stopping the old track does not leave the
  /// subscriber with nothing. There is no group before group 0, so a boundary of
  /// 0 leaves nothing to drain and cuts as well.
  pub(super) async fn suspend_at(&self, plan: &SwitchPlan, boundary: u64) {
    if plan.mode != SwitchMode::Soft || boundary == 0 {
      return;
    }
    let Some(suspending) = self.find_suspending(plan).await else {
      return;
    };
    let suspending = suspending.read().await;
    let mut state = suspending.subscription_state.write().await;
    state.soft_suspended = true;
    // A switch can only bring the end forward: a subscription that already asked
    // to stop earlier still stops there.
    let end_group = match state.end_group {
      None => boundary - 1,
      Some(existing) => existing.min(boundary - 1),
    };
    // An End Object belongs to the group the filter named, not to an earlier one.
    if Some(end_group) != state.end_group {
      state.end_object = None;
    }
    state.end_group = Some(end_group);
    state.publish_done_at_end = plan.publish_done;
    state.switch_activated = Some(plan.activated.clone());
    info!(
      "switch: draining {:?} for subscriber {} up to group {}",
      plan.suspending, self.client_connection_id, end_group
    );
  }

  /// Stops the subscription a hard switch suspends: Forward State 0 and every
  /// stream it still has open is reset, fill fetch streams included. Objects
  /// already in flight can still arrive. PUBLISH_DONE follows only if the switch
  /// asked for it.
  pub(super) async fn cut_suspending(&self, plan: &SwitchPlan) {
    let Some(suspending) = self.find_suspending(plan).await else {
      return;
    };
    let suspending = suspending.read().await;

    info!(
      "switch: suspending {:?} for subscriber {} ({:?})",
      plan.suspending, self.client_connection_id, plan.mode
    );

    suspending.subscription_state.write().await.forward = false;
    suspending.stop_fill_streams(FetchStop::Cancelled).await;
    suspending
      .reset_data_streams(StreamResetCode::Cancelled.to_u64())
      .await;

    if plan.publish_done
      && let Err(e) = suspending.send_switched_away().await
    {
      error!(
        "switch: failed to send PUBLISH_DONE to the suspended subscription: {:?}",
        e
      );
    }
  }

  pub(super) async fn find_suspending(
    &self,
    plan: &SwitchPlan,
  ) -> Option<Arc<RwLock<Subscription>>> {
    let Some(weak) = self
      .subscriber
      .subscriptions
      .get(plan.suspending_request_id)
      .await
    else {
      warn!(
        "switch: no subscription left to suspend for {:?} (request {}) on subscriber {}",
        plan.suspending, plan.suspending_request_id, self.client_connection_id
      );
      return None;
    };
    weak.upgrade()
  }

  /// The activating subscription has delivered, so the switch has happened: the
  /// suspending one can be stopped for good, and where no Start Group set a
  /// boundary, the group just delivered is it.
  pub(super) async fn complete_switch(&self, delivered_group: u64) {
    let Some(plan) = self
      .subscriber
      .switch_context
      .take_plan(self.request_id)
      .await
    else {
      return;
    };

    // Releases the suspending subscription's boundary, which was held until the
    // switch was known to be real.
    plan.activated.mark();

    info!(
      "switch: {:?} delivering from group {} for subscriber {}",
      self.full_track_name, delivered_group, self.client_connection_id
    );

    match plan.mode {
      // Scheduled in begin_switch when a Start Group gave a boundary; otherwise
      // this delivery is the boundary.
      SwitchMode::Soft if plan.boundary.is_none() => {
        self.suspend_at(&plan, delivered_group).await;
      }
      SwitchMode::Soft => {}
      SwitchMode::Hard => self.cut_suspending(&plan).await,
    }
  }

  /// Ends a soft switch's drain: the suspending subscription has delivered
  /// everything up to its boundary, so its streams are closed and PUBLISH_DONE
  /// goes out if the switch asked for it.
  pub(super) async fn finish_soft_drain(&self) {
    let publish_done = {
      let mut state = self.subscription_state.write().await;
      if !state.soft_suspended {
        return;
      }
      state.soft_suspended = false;
      state.forward = false;
      state.switch_activated = None;
      let wanted = state.publish_done_at_end;
      state.publish_done_at_end = false;
      wanted
    };

    info!(
      "switch: drain complete for subscriber={} relay_track_id={}",
      self.client_connection_id, self.relay_track_id
    );

    if publish_done {
      if let Err(e) = self.send_switched_away().await {
        error!(
          "switch: failed to send PUBLISH_DONE after the drain: {:?}",
          e
        );
      }
      self.finish().await;
    }
  }

  /// The PUBLISH_DONE a switch sends on the subscription it suspends.
  pub(super) async fn send_switched_away(&self) -> Result<(), anyhow::Error> {
    self
      .send_publish_done(PublishDoneStatusCode::TrackEnded, "switched away from")
      .await
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn a_dense_track_resumes_where_the_other_stops() {
    // 500ms groups on both sides: the next id is the takeover.
    assert_eq!(takeover_group(293, 294, 1), 294);
    // A long group on the track being left covers to 300, and this one has an id there.
    assert_eq!(takeover_group(293, 300, 1), 300);
  }

  #[test]
  fn a_track_with_longer_groups_resumes_at_one_of_its_own() {
    // 2s groups, four ids apart: 294 is not one of them, 297 is. The 1.5s between
    // stays with the track being switched away from, which has groups there.
    assert_eq!(takeover_group(293, 294, 4), 297);
    // Already aligned: the group after this one is exactly where the other stops.
    assert_eq!(takeover_group(292, 296, 4), 296);
  }

  #[test]
  fn a_track_the_other_never_caught_up_with_keeps_the_group_it_started() {
    assert_eq!(takeover_group(300, 300, 4), 300);
    assert_eq!(takeover_group(300, 296, 4), 300);
  }
}
