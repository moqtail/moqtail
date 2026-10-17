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

//! The default bandwidth allocation algorithm: id 0, the one every SSTS
//! implementation has to support.
//!
//! It allocates from the connection's bandwidth estimate. `rank` is a strict
//! priority: a set receives its full allocation before any lower-priority
//! (higher-rank) set receives anything. `weight` only splits a rank tier
//! between sets, and a saturated set (one that selected its top track) gives
//! its unused share back to the rest of its tier, repeatedly, until the tier
//! is fully claimed or every set in it selected its top track.
//!
//! The algorithm is stateless: the same input always produces the same
//! selection, which is what makes it a safe default for a relay that may serve
//! publishers with unrelated encodings.

mod allocate;

use crate::{AbrAlgorithm, AbrAlgorithmFactory, AbrInput, Selection};
use allocate::allocate;

#[cfg(test)]
mod tests;

/// Largest budget the allocation arithmetic runs with.
///
/// A connection with no estimate and no configured cap is treated as having
/// unlimited bandwidth, and the weighted split multiplies the budget before it
/// divides. Leaving headroom for that is cheaper than auditing every path for
/// saturation, and no real link is anywhere near a quarter of `u64` kbps.
const UNLIMITED_KBPS: u64 = u64::MAX / 4;

/// Algorithm 0: strict-priority, weighted bandwidth allocation.
pub struct DefaultAlgorithm;

/// Factory for [`DefaultAlgorithm`].
pub struct DefaultAlgorithmFactory;

impl AbrAlgorithmFactory for DefaultAlgorithmFactory {
  fn id(&self) -> u64 {
    0
  }

  fn name(&self) -> &'static str {
    "default"
  }

  fn create(&self) -> Box<dyn AbrAlgorithm> {
    Box::new(DefaultAlgorithm)
  }
}

impl AbrAlgorithm for DefaultAlgorithm {
  fn decide(&mut self, input: &AbrInput) -> Selection {
    allocate(input.budget_kbps().min(UNLIMITED_KBPS), input.sets)
  }
}
