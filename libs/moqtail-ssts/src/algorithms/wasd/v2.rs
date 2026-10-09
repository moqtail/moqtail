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

//! WASD v2: the retroactive downshift of the newest overtaken group.
//!
//! A downshift cancels the one group closest to the new decision — the
//! newest still in flight above the new rung — and has it re-sent there.
//! 1080p down to 720p at group 4 means group 3, still in flight at 1080p,
//! is cancelled and re-sent at 720p. Groups older than that keep their
//! delivery: their re-send would chase a delivery window that is mostly
//! gone, and the bytes it would spend are better left to the groups ahead.
//!
//! The re-sent stream is excluded from the depth, so the repair does not
//! read as new congestion. What the repair cannot undo is documented in
//! the module README's shortcomings: a short dip leaves a queue behind,
//! and the drain's overlap with the next group can still read as a second
//! congestion. That is the accepted cost of reaching back, measured
//! against the classic on `PROFILE_JITTER`.
//!
//! Everything else is documented in [`super`].

use super::Reach;
use super::WASD_V2_ALGORITHM_ID;
use super::WasdAlgorithmFactory;

/// v2: [`Reach::Newest`], advertised as `wasd-v2` at
/// [`WASD_V2_ALGORITHM_ID`].
pub const FACTORY: WasdAlgorithmFactory = WasdAlgorithmFactory {
  id: WASD_V2_ALGORITHM_ID,
  reach: Reach::Newest,
};
