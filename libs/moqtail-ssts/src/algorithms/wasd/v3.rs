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

//! WASD v3: the retroactive double downshift.
//!
//! A downshift reaches every overtaken group, staged by age: the newest
//! re-sent on the rung the decision landed on, every older one a rung
//! below that (the ladder's bottom being the floor). 1080p down to 720p
//! means the newest overtaken group continues at 720p and everything
//! older at 480p, because the older a group is, the less of its delivery
//! window is left to a re-send — the stage is the repair paying for itself
//! with the quality it gives up.
//!
//! The full repair is also the most bytes a downshift can spend, and this
//! version is where the accepted shortcoming of reaching back shows
//! strongest: on a short dip the repair bytes join the queue the dip left
//! behind, the next group's stream stays open into the one after that, and
//! the overlap reads as a second congestion — the double downshift the
//! module README's shortcomings section measures against the classic.
//!
//! Everything else is documented in [`super`].

use super::Reach;
use super::WASD_V3_ALGORITHM_ID;
use super::WasdAlgorithmFactory;

/// v3: [`Reach::AllStaged`], advertised as `wasd-v3` at
/// [`WASD_V3_ALGORITHM_ID`].
pub const FACTORY: WasdAlgorithmFactory = WasdAlgorithmFactory {
  id: WASD_V3_ALGORITHM_ID,
  reach: Reach::AllStaged,
};
