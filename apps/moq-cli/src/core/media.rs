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

use anyhow::Result;
use bytes::Bytes;
use tokio::time::{Duration, sleep};

/// A source that generates frames.
/// In the future, this will be a trait implemented by FileSource and PipeSource.
pub struct MockSource {
  counter: usize,
}

impl MockSource {
  pub fn new() -> Self {
    Self { counter: 0 }
  }

  /// Generates a frame similar to the legacy client: "payload xxxxx..."
  pub async fn read_frame(&mut self) -> Result<Option<Bytes>> {
    // Simulate frame timing (10fps)
    sleep(Duration::from_millis(100)).await;

    if self.counter >= 100 {
      return Ok(None); // Stop after 100 frames
    }

    let pattern = "x".repeat(self.counter);
    let payload = format!("payload {} - seq {}", pattern, self.counter);

    self.counter += 1;

    Ok(Some(Bytes::from(payload)))
  }
}
