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

use super::Track;
use moqtail::model::property::track_property::TrackProperty;

/// A NEW_GROUP_REQUEST the relay sent upstream. It is outstanding until a group
/// past the largest one of the time arrives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct SentNewGroupRequest {
  value: u64,
  largest_group: Option<u64>,
}

impl Track {
  /// Whether a subscriber's NEW_GROUP_REQUEST of `value` is passed upstream,
  /// recording it as sent if so. It is when the track supports dynamic groups,
  /// `value` is 0 or past the largest group, and no request the relay sent is still
  /// outstanding with a value at least as large.
  pub(crate) async fn claim_new_group_request(&self, value: u64) -> bool {
    let dynamic = self
      .track_properties
      .read()
      .await
      .iter()
      .any(|p| matches!(p, TrackProperty::DynamicGroups { enabled: true }));
    if !dynamic {
      return false;
    }
    let largest_group = self.largest_object().await.map(|l| l.group);
    if value != 0 && largest_group.is_some_and(|group| value <= group) {
      return false;
    }
    let mut sent = self.sent_new_group_request.lock().unwrap();
    if let Some(prev) = *sent
      && prev.largest_group == largest_group
      && prev.value >= value
    {
      return false;
    }
    *sent = Some(SentNewGroupRequest {
      value,
      largest_group,
    });
    true
  }
}

#[cfg(test)]
mod tests {
  use super::super::{Track, TrackOrigin, TrackStatus};
  use crate::server::config::AppConfig;
  use moqtail::model::common::location::Location;
  use moqtail::model::data::full_track_name::FullTrackName;
  use moqtail::model::property::track_property::TrackProperty;
  use std::sync::atomic::Ordering;

  async fn track(dynamic_groups: bool) -> Track {
    let config = Box::leak(Box::new(AppConfig::default()));
    let track = Track::new(
      1,
      FullTrackName::try_new("meet/room1", "video").unwrap(),
      config,
      TrackStatus::Pending,
      TrackOrigin::Publish,
    );
    track
      .set_track_properties(vec![TrackProperty::DynamicGroups {
        enabled: dynamic_groups,
      }])
      .await;
    track
  }

  async fn reach_group(track: &Track, group: u64) {
    *track.largest_location.write().await = Location::new(group, 0);
    track.has_objects.store(true, Ordering::Relaxed);
  }

  #[tokio::test]
  async fn a_track_without_dynamic_groups_passes_nothing_upstream() {
    assert!(!track(false).await.claim_new_group_request(0).await);
  }

  #[tokio::test]
  async fn only_zero_or_a_value_past_the_largest_group_goes_upstream() {
    let track = track(true).await;
    reach_group(&track, 3).await;
    assert!(
      !track.claim_new_group_request(3).await,
      "group 3 already exists"
    );
    assert!(track.claim_new_group_request(4).await);
  }

  #[tokio::test]
  async fn an_outstanding_request_holds_back_smaller_or_equal_ones() {
    let track = track(true).await;
    reach_group(&track, 3).await;
    assert!(track.claim_new_group_request(5).await);
    assert!(!track.claim_new_group_request(5).await);
    assert!(!track.claim_new_group_request(0).await);
    assert!(
      track.claim_new_group_request(6).await,
      "a larger one still goes"
    );
  }

  #[tokio::test]
  async fn a_new_group_ends_the_outstanding_request() {
    let track = track(true).await;
    assert!(track.claim_new_group_request(0).await);
    assert!(!track.claim_new_group_request(0).await);
    reach_group(&track, 0).await;
    assert!(track.claim_new_group_request(0).await);
  }
}
