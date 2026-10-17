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

use super::full_track_name::FullTrackName;
use super::object::Object;
use super::subgroup::Subgroup;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub enum GroupContent {
  Subgroups(BTreeMap<u64, Subgroup>),
  Objects(BTreeMap<u64, Object>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Group {
  pub full_track_name: FullTrackName,
  pub group_id: u64,
  pub content: GroupContent,
}
