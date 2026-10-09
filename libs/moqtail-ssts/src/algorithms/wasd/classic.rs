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

//! WASD, the classic: downshift on depth, retroactive nothing.
//!
//! The baseline of the comparison. The depth is read at the tier being
//! delivered and sampled inside the group like in every version, but a
//! downshift is forward-looking only: it issues no supersede request, the
//! groups already in flight above the new rung finish on the member they
//! were carrying, and nothing is re-sent. What the dip cost those groups
//! the subscriber decides for itself, with the buffer it negotiated.
//!
//! Everything the version shares with the others — the signal, the
//! monitor, the streaks, the `wasd.reads` knob — is documented in
//! [`super`].

use super::Reach;
use super::WASD_ALGORITHM_ID;
use super::WasdAlgorithmFactory;

/// The classic: [`Reach::Nothing`], advertised as `wasd` at
/// [`WASD_ALGORITHM_ID`].
pub const FACTORY: WasdAlgorithmFactory = WasdAlgorithmFactory {
  id: WASD_ALGORITHM_ID,
  reach: Reach::Nothing,
};
