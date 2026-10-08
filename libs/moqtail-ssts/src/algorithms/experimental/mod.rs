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

//! Algorithms that are kept, compiled, and tested, but are not in the
//! [`crate::registry`]: experimental steps that were not carried forward to
//! the comparison.
//!
//! * [`bnb`](crate::algorithms::experimental::bnb) (Budget-Negotiated
//!   Backpressure, wire id `0xff03`) is the
//!   per-set backlog/budget-ratio branch. It explored an orthogonal design to
//!   the ASD family; it is not part of the current ASD / ASD v1.1 / WASD
//!   comparison, so it is not advertised and cannot be negotiated. Its id is
//!   left unused so nothing else can claim it by accident.

pub mod bnb;
