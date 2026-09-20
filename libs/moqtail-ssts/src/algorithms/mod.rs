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

pub mod default;
