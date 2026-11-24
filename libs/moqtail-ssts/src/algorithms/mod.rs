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

//! The bandwidth allocation algorithms MOQtail ships with.
//!
//! Each algorithm is one directory: the implementation, its tests, and (where
//! the behaviour needs explaining) a README describing what it optimizes for
//! and what it does badly. Nothing here may depend on the host: the contract in
//! the crate root is the only input.

pub mod asd;
pub mod asd_v11;
pub mod default;
/// The perfect-knowledge oracle, run on the testbed: the ceiling the adaptive
/// algorithms are measured against.
pub mod oracle;
pub mod wasd;

use crate::{AbrInput, Selection};

/// The selection the tier-walking algorithms share: every set with a subscriber
/// serves the member at `tier`, clamped to its own ladder, and a set without
/// one forwards nothing.
///
/// A set shorter than the tier serves its own top track. That is what makes one
/// index shared across sets of different heights safe to reason about: the tall
/// set switches and the short one has nothing to switch to, rather than
/// forwarding a member it does not have.
pub(crate) fn serve_tier(input: &AbrInput, tier: usize) -> Selection {
  let mut selection = Selection::with_capacity(input.sets.len());
  for set in input.sets {
    let selected = if set.active && !set.members.is_empty() {
      set
        .members
        .get(tier.min(set.members.len() - 1))
        .map(|(_, track_id)| *track_id)
    } else {
      None
    };
    selection.insert(set.id, selected);
  }
  selection
}
