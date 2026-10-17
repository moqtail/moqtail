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

//! The boundary between the mechanism and the algorithms, held by the source
//! rather than by review.
//!
//! The crate compiles without the relay, which already keeps host types out of
//! the algorithms. What this guards is the other direction, which the compiler
//! cannot see: an algorithm creeping back into the host, where it would be
//! reached through the relay's own types instead of through the contract, and
//! the contract quietly becoming decorative.

use std::fs;
use std::path::{Path, PathBuf};

fn walk(dir: &Path, files: &mut Vec<PathBuf>) {
  let entries = match fs::read_dir(dir) {
    Ok(entries) => entries,
    Err(_) => return,
  };
  for entry in entries.flatten() {
    let path = entry.path();
    if path.is_dir() {
      walk(&path, files);
    } else if path.extension().is_some_and(|ext| ext == "rs") {
      files.push(path);
    }
  }
}

fn source_files(dir: &Path) -> Vec<PathBuf> {
  let mut files = Vec::new();
  walk(dir, &mut files);
  assert!(!files.is_empty(), "no sources found under {dir:?}");
  files
}

/// The relay must not implement algorithms: it runs the ones the registry
/// hands it. A hit here means an algorithm moved back into the host, where it
/// would read connection state directly instead of through `AbrInput`.
#[test]
fn the_host_implements_no_algorithms() {
  // <workspace>/libs/moqtail-ssts -> <workspace>
  let relay_src = Path::new(env!("CARGO_MANIFEST_DIR"))
    .parent()
    .and_then(Path::parent)
    .expect("workspace root")
    .join("apps/relay/src");
  let offenders: Vec<PathBuf> = source_files(&relay_src)
    .into_iter()
    .filter(|path| {
      fs::read_to_string(path)
        .map(|text| text.contains("impl AbrAlgorithm for"))
        .unwrap_or(false)
    })
    .collect();
  assert!(
    offenders.is_empty(),
    "SSTS algorithms must live in this crate; found implementations in {offenders:?}"
  );
}

/// And the algorithms here must not reach for the host. The dependency graph
/// already forbids a real import, so what this catches is the shape a
/// regression takes in practice: someone reintroducing a `&MOQTClient`, a
/// process-wide config read or an async contract so an algorithm can go and
/// look at the connection itself.
#[test]
fn the_algorithms_use_nothing_but_the_contract() {
  let algorithms = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/algorithms");
  let forbidden = [
    "MOQTClient",
    "AppConfig",
    "crate::server",
    "super::super::server",
    "wtransport",
    "moqtail::",
    "Pin<Box<dyn Future",
  ];
  let offenders: Vec<String> = source_files(&algorithms)
    .iter()
    .flat_map(|path| {
      let text = fs::read_to_string(path).unwrap_or_default();
      forbidden
        .iter()
        .filter(move |needle| text.contains(**needle))
        .map(|needle| format!("{} mentions {needle}", path.display()))
    })
    .collect();
  assert!(
    offenders.is_empty(),
    "algorithms may only use AbrInput and its own state:\n{}",
    offenders.join("\n")
  );
}
