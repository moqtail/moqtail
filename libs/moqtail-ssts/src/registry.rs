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

//! The algorithms a relay runs.
//!
//! This is the one place a new algorithm is announced. Everything downstream
//! of it is derived: the SETUP advertisement, the configuration validation,
//! and the instance the controller runs for a connection.

use std::sync::{Arc, OnceLock};

use crate::algorithms::default::DefaultAlgorithmFactory;
use crate::{AbrAlgorithm, AbrAlgorithmFactory};

/// First algorithm id an implementation may use for its own experiments.
///
/// SSTS algorithm ids are allocated by IETF review, and the specification
/// reserves the top 255 values for implementations to use without registration
/// so that experimenting does not have to wait on a standards action. Only one
/// id is assigned today: `0`, the default bandwidth allocation, which every
/// implementation must support. Our own algorithms therefore start at the
/// first value in the private range, where a later standards allocation cannot
/// collide with them.
pub const PRIVATE_ALGORITHM_ID_BASE: u64 = 0xff01;

/// The set of algorithms a relay runs, keyed by wire id.
pub struct Registry {
  factories: Vec<Arc<dyn AbrAlgorithmFactory>>,
}

impl Registry {
  fn new() -> Self {
    Self {
      factories: vec![Arc::new(DefaultAlgorithmFactory)],
    }
  }

  /// Every registered factory, in advertisement order.
  pub fn factories(&self) -> &[Arc<dyn AbrAlgorithmFactory>] {
    &self.factories
  }

  /// The ids to advertise in SETUP, in registration order.
  pub fn ids(&self) -> Vec<u64> {
    self.factories.iter().map(|f| f.id()).collect()
  }

  pub fn get(&self, id: u64) -> Option<&Arc<dyn AbrAlgorithmFactory>> {
    self.factories.iter().find(|f| f.id() == id)
  }

  /// A fresh, stateless-history instance of `id` for one connection.
  pub fn create(&self, id: u64) -> Option<Box<dyn AbrAlgorithm>> {
    self.get(id).map(|f| f.create())
  }
}

/// The process-wide registry.
pub fn registry() -> &'static Registry {
  static REGISTRY: OnceLock<Registry> = OnceLock::new();
  REGISTRY.get_or_init(Registry::new)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn registered_ids_are_unique() {
    let ids = registry().ids();
    let unique: std::collections::HashSet<u64> = ids.iter().copied().collect();
    assert_eq!(
      ids.len(),
      unique.len(),
      "two algorithms claim the same id: {ids:?}"
    );
  }

  #[test]
  fn ids_are_either_standard_or_private() {
    for id in registry().ids() {
      assert!(
        id == 0 || id >= PRIVATE_ALGORITHM_ID_BASE,
        "algorithm id {id} sits in the range reserved for future standard \
         allocation; use {PRIVATE_ALGORITHM_ID_BASE} or above"
      );
    }
  }

  #[test]
  fn the_default_algorithm_is_always_available() {
    assert!(registry().get(0).is_some(), "algorithm 0 is mandatory");
    assert!(registry().create(0).is_some());
  }

  #[test]
  fn every_factory_names_its_algorithm() {
    for factory in registry().factories() {
      assert!(!factory.name().is_empty());
      assert!(factory.id() == 0 || factory.id() >= PRIVATE_ALGORITHM_ID_BASE);
    }
  }

  #[test]
  fn unknown_ids_are_rejected() {
    assert!(registry().get(12345).is_none());
    assert!(registry().create(12345).is_none());
  }
}
