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

use std::collections::HashMap;

use crate::model::error::ParseError;

use super::full_track_name::FullTrackName;

#[derive(Debug, Default, Clone)]
pub struct TrackAliasMap {
  alias_to_name: HashMap<u64, FullTrackName>,
  name_to_alias: HashMap<FullTrackName, u64>,
}

impl TrackAliasMap {
  pub fn new() -> Self {
    Default::default()
  }

  pub fn add_mapping(&mut self, alias: u64, name: FullTrackName) -> Result<(), ParseError> {
    if let Some(existing_name) = self.alias_to_name.get(&alias) {
      if *existing_name == name {
        return Ok(());
      } else {
        return Err(ParseError::TrackAliasError {
          context: "TrackAliasMap::add_mapping(existing_name)",
          details: format!("Full track name already exists for alias: {alias}"),
        });
      }
    }

    if let Some(existing_alias) = self.name_to_alias.get(&name) {
      if *existing_alias == alias {
        return Ok(());
      } else {
        return Err(ParseError::TrackAliasError {
          context: "TrackAliasMap::add_mapping(existing_alias)",
          details: format!("An alias already exists for full track name: {name:?}"),
        });
      }
    }

    self.alias_to_name.insert(alias, name.clone());
    self.name_to_alias.insert(name, alias);
    Ok(())
  }

  pub fn get_name_by_alias(&self, alias: u64) -> Result<&FullTrackName, ParseError> {
    self
      .alias_to_name
      .get(&alias)
      .ok_or(ParseError::TrackAliasError {
        context: "TrackAliasMap::get_name_by_alias(alias)",
        details: format!("Alias: {alias} doesn't exist"),
      })
  }

  pub fn get_alias_by_name(&self, name: &FullTrackName) -> Result<u64, ParseError> {
    self
      .name_to_alias
      .get(name)
      .copied()
      .ok_or(ParseError::TrackAliasError {
        context: "TrackAliasMap::get_alias_by_name(name)",
        details: format!("Name: {name:?} doesn't exist"),
      })
  }

  pub fn remove_mapping_by_alias(&mut self, alias: u64) -> Option<FullTrackName> {
    if let Some(name) = self.alias_to_name.remove(&alias) {
      self.name_to_alias.remove(&name);
      Some(name)
    } else {
      None
    }
  }

  pub fn remove_mapping_by_name(&mut self, name: &FullTrackName) -> Option<u64> {
    if let Some(alias) = self.name_to_alias.remove(name) {
      self.alias_to_name.remove(&alias);
      Some(alias)
    } else {
      None
    }
  }

  pub fn contains_alias(&self, alias: u64) -> bool {
    self.alias_to_name.contains_key(&alias)
  }

  pub fn contains_name(&self, name: &FullTrackName) -> bool {
    self.name_to_alias.contains_key(name)
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn make_name() -> FullTrackName {
    FullTrackName::try_new("namespace/test", "bamboozeled").unwrap()
  }
  fn make_name2() -> FullTrackName {
    FullTrackName::try_new("namespace/test/yeeahboii", "bamboozeled").unwrap()
  }

  #[test]
  fn test_add_and_get_mapping() {
    let mut map = TrackAliasMap::new();
    let alias = 42;
    let name = make_name();
    assert!(map.add_mapping(alias, name.clone()).is_ok());
    assert_eq!(map.get_name_by_alias(alias).unwrap(), &name);
    assert_eq!(map.get_alias_by_name(&name).unwrap(), alias);
  }

  #[test]
  fn test_add_duplicate_alias_error() {
    let mut map = TrackAliasMap::new();
    let alias = 1;
    let name1 = make_name();
    let name2 = make_name2();
    assert!(map.add_mapping(alias, name1.clone()).is_ok());
    let err = map.add_mapping(alias, name2.clone());
    assert!(err.is_err())
  }

  #[test]
  fn test_remove_mapping_by_alias_and_name() {
    let mut map = TrackAliasMap::new();
    let alias = 7;
    let name = make_name();
    map.add_mapping(alias, name.clone()).unwrap();
    assert!(map.remove_mapping_by_alias(alias).is_some());
    assert!(!map.contains_alias(alias));
    assert!(!map.contains_name(&name));
    // Add again and remove by name
    map.add_mapping(alias, name.clone()).unwrap();
    assert!(map.remove_mapping_by_name(&name).is_some());
    assert!(!map.contains_alias(alias));
    assert!(!map.contains_name(&name));
  }

  #[test]
  fn test_get_nonexistent_returns_error() {
    let map = TrackAliasMap::new();
    let alias = 99;
    let name = make_name();
    assert!(map.get_name_by_alias(alias).is_err());
    assert!(map.get_alias_by_name(&name).is_err());
  }
}
