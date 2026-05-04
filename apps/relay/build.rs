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

use std::process::Command;

fn main() {
  let commit = Command::new("git")
    .args(["rev-parse", "--short", "HEAD"])
    .output()
    .ok()
    .and_then(|output| String::from_utf8(output.stdout).ok())
    .map(|s| s.trim().to_string())
    .unwrap_or_else(|| "unknown".to_string());

  let version = env!("CARGO_PKG_VERSION");
  let moqtail_version = format!("moqtail/{version}+{commit}");

  println!("cargo:rustc-env=MOQTAIL_VERSION={}", moqtail_version);
}
