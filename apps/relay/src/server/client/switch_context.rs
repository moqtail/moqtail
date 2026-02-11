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

use moqtail::model::data::full_track_name::FullTrackName;
use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::info;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[repr(u8)]
pub enum SwitchStatus {
  None,
  Current,
  Next,
}

pub type SwitchContextItems = Arc<RwLock<HashMap<FullTrackName, SwitchStatus>>>;

#[derive(Debug, Clone)]
pub struct SwitchContext {
  pub items: SwitchContextItems,
}

impl SwitchContext {
  pub fn new() -> Self {
    Self {
      items: Arc::new(RwLock::new(HashMap::new())),
    }
  }

  // Add or update a switch item
  // If the status is Current or Next, ensure no other item has that status
  pub async fn add_or_update_switch_item(&self, track_name: FullTrackName, status: SwitchStatus) {
    let mut items = self.items.write().await;
    items.insert(track_name.clone(), status);

    // there can be only one Current and one Next
    if status == SwitchStatus::Current || status == SwitchStatus::Next {
      let target_status = status;
      for (other_track_name, other_status) in items.iter_mut() {
        if *other_status == target_status && *other_track_name != track_name {
          info!(
            "Switching status of track {:?} from {:?} to None",
            other_track_name, target_status
          );
          *other_status = SwitchStatus::None;
          break;
        }
      }
    }
  }

  pub async fn get_switch_status(&self, track_name: &FullTrackName) -> Option<SwitchStatus> {
    let items = self.items.read().await;
    items.get(track_name).copied()
  }

  pub async fn get_current(&self) -> Option<FullTrackName> {
    let items = self.items.read().await;
    for (track_name, status) in items.iter() {
      if *status == SwitchStatus::Current {
        return Some(track_name.clone());
      }
    }
    None
  }
}

impl fmt::Display for SwitchStatus {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    let status_str = match self {
      SwitchStatus::None => "None",
      SwitchStatus::Current => "Current",
      SwitchStatus::Next => "Next",
    };
    write!(f, "{}", status_str)
  }
}

impl fmt::Display for SwitchContext {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    let items = self.items.blocking_read();
    for (track_name, status) in items.iter() {
      writeln!(f, "{:?}: {}", track_name, status)?;
    }
    Ok(())
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[tokio::test]
  async fn add_and_update_current_ensures_unique() {
    let context = SwitchContext::new();

    let t1 = FullTrackName::try_new("ns", "t1").expect("create t1");
    let t2 = FullTrackName::try_new("ns", "t2").expect("create t2");

    // set t1 as Current
    context
      .add_or_update_switch_item(t1.clone(), SwitchStatus::Current)
      .await;
    assert_eq!(context.get_current().await.unwrap(), t1);
    assert_eq!(
      context.get_switch_status(&t1).await,
      Some(SwitchStatus::Current)
    );

    // set t2 as Current -> t1 should become None
    context
      .add_or_update_switch_item(t2.clone(), SwitchStatus::Current)
      .await;
    assert_eq!(context.get_current().await.unwrap(), t2);
    assert_eq!(
      context.get_switch_status(&t1).await,
      Some(SwitchStatus::None)
    );
  }

  #[tokio::test]
  async fn next_status_is_unique() {
    let context = SwitchContext::new();

    let a = FullTrackName::try_new("ns", "a").expect("create a");
    let b = FullTrackName::try_new("ns", "b").expect("create b");

    context
      .add_or_update_switch_item(a.clone(), SwitchStatus::Next)
      .await;

    context
      .add_or_update_switch_item(b.clone(), SwitchStatus::Next)
      .await;
    assert_eq!(
      context.get_switch_status(&a).await,
      Some(SwitchStatus::None)
    );
  }

  #[test]
  fn display_strings() {
    assert_eq!(format!("{}", SwitchStatus::None), "None");
    assert_eq!(format!("{}", SwitchStatus::Current), "Current");
    assert_eq!(format!("{}", SwitchStatus::Next), "Next");
  }
}
