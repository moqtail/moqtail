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

use moqtail::model::data::full_track_name::FullTrackName;
use std::collections::HashMap;

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct SwitchingSetMember {
  pub full_track_name: FullTrackName,
  pub relay_track_id: u64,
  pub throughput_threshold_kbps: u64,
  pub request_id: u64,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Default)]
pub struct SwitchingSet {
  pub id: u64,
  pub members: Vec<SwitchingSetMember>,
  pub fraction: u8,
  pub rank: u8,
  pub active: bool,
  pub selected_relay_track_id: Option<u64>,
}

#[derive(Debug, Default)]
pub struct SwitchingSetManager {
  pub sets: HashMap<u64, SwitchingSet>,
  pub track_to_set: HashMap<FullTrackName, u64>,
  // Reverse index: relay_track_id -> switching_set_id for fast per-set stream counting
  pub relay_track_to_set: HashMap<u64, u64>,
}

impl SwitchingSetManager {
  pub fn new() -> Self {
    Self::default()
  }

  #[allow(clippy::too_many_arguments)]
  pub fn assign(
    &mut self,
    full_track_name: FullTrackName,
    relay_track_id: u64,
    request_id: u64,
    switching_set_id: u64,
    throughput_threshold_kbps: u64,
    fraction: u8,
    rank: u8,
    activate: bool,
  ) -> Result<(), &'static str> {
    if let Some(&existing_set_id) = self.track_to_set.get(&full_track_name)
      && existing_set_id != switching_set_id
    {
      return Err("Track already assigned to a different switching set");
    }

    let member = SwitchingSetMember {
      full_track_name: full_track_name.clone(),
      relay_track_id,
      throughput_threshold_kbps,
      request_id,
    };

    let set = self
      .sets
      .entry(switching_set_id)
      .or_insert_with(|| SwitchingSet {
        id: switching_set_id,
        ..Default::default()
      });

    set.fraction = fraction;
    set.rank = rank;

    // Replace or insert member
    if let Some(existing) = set
      .members
      .iter_mut()
      .find(|m| m.full_track_name == full_track_name)
    {
      *existing = member;
    } else {
      set.members.push(member);
    }

    // Sort members by throughput ascending (lowest quality = index 0)
    set.members.sort_by_key(|m| m.throughput_threshold_kbps);

    self.track_to_set.insert(full_track_name, switching_set_id);
    self
      .relay_track_to_set
      .insert(relay_track_id, switching_set_id);

    if activate {
      set.active = true;
    }

    Ok(())
  }

  pub fn remove(&mut self, full_track_name: &FullTrackName) {
    if let Some(set_id) = self.track_to_set.remove(full_track_name)
      && let Some(set) = self.sets.get_mut(&set_id)
    {
      // Clean up relay_track_to_set for the removed member
      let removed_members: Vec<_> = set
        .members
        .iter()
        .filter(|m| m.full_track_name == *full_track_name)
        .map(|m| m.relay_track_id)
        .collect();
      for rtid in &removed_members {
        self.relay_track_to_set.remove(rtid);
      }

      set
        .members
        .retain(|m| m.full_track_name != *full_track_name);
      if set.members.is_empty() {
        self.sets.remove(&set_id);
      }
    }
  }

  /// Look up the switching set ID that a given relay_track_id belongs to.
  /// Returns `None` if the track is not part of any switching set.
  pub fn get_set_id_for_relay_track(&self, relay_track_id: u64) -> Option<u64> {
    self.relay_track_to_set.get(&relay_track_id).copied()
  }

  pub fn update_assignment(
    &mut self,
    full_track_name: &FullTrackName,
    fraction: Option<u8>,
    rank: Option<u8>,
    activate: Option<bool>,
  ) -> Result<(), &'static str> {
    let set_id = self
      .track_to_set
      .get(full_track_name)
      .copied()
      .ok_or("Track not found in any switching set")?;

    if let Some(set) = self.sets.get_mut(&set_id) {
      if let Some(f) = fraction {
        set.fraction = f;
      }
      if let Some(r) = rank {
        set.rank = r;
      }
      if let Some(a) = activate {
        set.active = a;
      }
      Ok(())
    } else {
      Err("Switching set not found")
    }
  }

  pub fn get_set_for_track(&self, full_track_name: &FullTrackName) -> Option<&SwitchingSet> {
    self
      .track_to_set
      .get(full_track_name)
      .and_then(|id| self.sets.get(id))
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use moqtail::model::common::tuple::{Tuple, TupleField};

  fn make_track(ns: &str, name: &str) -> FullTrackName {
    FullTrackName::new(Tuple::from_utf8_path(ns), TupleField::from_utf8(name))
      .expect("create track name")
  }

  #[tokio::test]
  async fn test_assign_and_remove() {
    let mut manager = SwitchingSetManager::new();
    let track1 = make_track("ns", "1080p");
    let track2 = make_track("ns", "480p");

    manager
      .assign(track1.clone(), 10, 1, 1, 3000, 6, 1, false)
      .unwrap();
    manager
      .assign(track2.clone(), 20, 2, 1, 800, 6, 1, true)
      .unwrap();

    let result = manager.assign(track1.clone(), 10, 1, 2, 3000, 4, 2, true);
    assert!(result.is_err());

    manager.remove(&track1);
    assert!(manager.get_set_for_track(&track1).is_none());

    let set = manager.get_set_for_track(&track2).unwrap();
    assert_eq!(set.members.len(), 1);
  }

  #[test]
  fn test_relay_track_to_set_lookup() {
    let mut manager = SwitchingSetManager::new();
    let track1 = make_track("ns", "1080p");
    let track2 = make_track("ns", "480p");

    manager
      .assign(track1.clone(), 100, 1, 5, 3000, 6, 1, true)
      .unwrap();
    manager
      .assign(track2.clone(), 200, 2, 5, 800, 6, 1, true)
      .unwrap();

    assert_eq!(manager.get_set_id_for_relay_track(100), Some(5));
    assert_eq!(manager.get_set_id_for_relay_track(200), Some(5));
    assert_eq!(manager.get_set_id_for_relay_track(999), None);

    manager.remove(&track1);
    assert_eq!(manager.get_set_id_for_relay_track(100), None);
    assert_eq!(manager.get_set_id_for_relay_track(200), Some(5));
  }
}
