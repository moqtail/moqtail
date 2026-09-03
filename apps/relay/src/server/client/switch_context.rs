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

use moqtail::model::control::constant::SwitchMode;
use moqtail::model::data::full_track_name::FullTrackName;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::RwLock;

/// Shared between the two subscriptions of one switch: the activating side sets
/// it when it first delivers, the suspending side reads it to know its boundary
/// is real.
#[derive(Debug, Clone, Default)]
pub struct SwitchActivation(Arc<AtomicBool>);

impl SwitchActivation {
  pub fn mark(&self) {
    self.0.store(true, Ordering::Release);
  }

  pub fn is_set(&self) -> bool {
    self.0.load(Ordering::Acquire)
  }
}

/// What a SWITCH_FROM asked for.
///
/// Both subscriptions are driven from `boundary`, a group id on the shared grid
/// that group ids form across the tracks of one content: the activating
/// subscription delivers from it, the suspending one stops at it. Neither has to
/// look at the other's group ids, which do not have to line up.
#[derive(Debug, Clone)]
pub struct SwitchPlan {
  /// The subscription being suspended.
  pub suspending: FullTrackName,
  pub mode: SwitchMode,
  /// Whether stopping it also sends PUBLISH_DONE.
  pub publish_done: bool,
  /// The activating subscription's Start Group, when it asked for one. Without
  /// it there is nothing to schedule and the boundary is instead the first group
  /// that subscription delivers.
  pub boundary: Option<u64>,
  /// Set once the activating subscription has delivered something. Until then
  /// the suspending one keeps going past its boundary, so a switch to a track
  /// that never publishes leaves the subscriber with the track it already had
  /// rather than with nothing.
  pub activated: SwitchActivation,
}

impl SwitchPlan {
  /// The boundary is left unset: it is the activating subscription that knows
  /// its own Start Group, and it fills this in when the switch is scheduled.
  pub fn new(suspending: FullTrackName, mode: SwitchMode, publish_done: bool) -> Self {
    Self {
      suspending,
      mode,
      publish_done,
      boundary: None,
      activated: SwitchActivation::default(),
    }
  }
}

#[derive(Debug, Clone)]
pub struct SwitchContext {
  /// Keyed by the activating track: the switch it is the target of.
  plans: Arc<RwLock<HashMap<FullTrackName, SwitchPlan>>>,
}

impl SwitchContext {
  pub fn new() -> Self {
    Self {
      plans: Arc::new(RwLock::new(HashMap::new())),
    }
  }

  /// Records what to do to the suspending subscription once `activating` delivers.
  pub async fn set_plan(&self, activating: FullTrackName, plan: SwitchPlan) {
    self.plans.write().await.insert(activating, plan);
  }

  /// Takes the plan for `activating`, leaving none behind: a switch happens once.
  pub async fn take_plan(&self, activating: &FullTrackName) -> Option<SwitchPlan> {
    self.plans.write().await.remove(activating)
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn plan(suspending: &str, mode: SwitchMode) -> SwitchPlan {
    SwitchPlan::new(
      FullTrackName::try_new("ns", suspending).expect("create track name"),
      mode,
      false,
    )
  }

  #[test]
  fn activation_starts_unset_and_is_shared() {
    let plan = plan("low", SwitchMode::Soft);
    let held_by_the_suspending_side = plan.activated.clone();
    assert!(!held_by_the_suspending_side.is_set());

    plan.activated.mark();
    assert!(held_by_the_suspending_side.is_set());
  }

  #[test]
  fn a_new_plan_has_no_boundary_until_the_switch_is_scheduled() {
    assert_eq!(plan("low", SwitchMode::Hard).boundary, None);
  }

  #[tokio::test]
  async fn a_plan_is_taken_once() {
    let context = SwitchContext::new();
    let activating = FullTrackName::try_new("ns", "high").expect("create track name");

    context
      .set_plan(activating.clone(), plan("low", SwitchMode::Hard))
      .await;

    assert!(context.take_plan(&activating).await.is_some());
    assert!(context.take_plan(&activating).await.is_none());
  }

  #[tokio::test]
  async fn plans_are_kept_per_activating_track() {
    let context = SwitchContext::new();
    let high = FullTrackName::try_new("ns", "high").expect("create track name");
    let mid = FullTrackName::try_new("ns", "mid").expect("create track name");

    context
      .set_plan(high.clone(), plan("low", SwitchMode::Hard))
      .await;
    context
      .set_plan(mid.clone(), plan("high", SwitchMode::Soft))
      .await;

    assert_eq!(
      context.take_plan(&high).await.expect("plan for high").mode,
      SwitchMode::Hard
    );
    assert_eq!(
      context.take_plan(&mid).await.expect("plan for mid").mode,
      SwitchMode::Soft
    );
  }
}
