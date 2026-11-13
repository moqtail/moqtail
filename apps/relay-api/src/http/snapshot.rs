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

//! A snapshot of relay state shared by the requests of one second, so polling
//! clients cost the relay one copy per second however many there are.

use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

const MAX_AGE: Duration = Duration::from_secs(1);

pub(crate) struct Cached<T> {
  slot: Mutex<Option<(Instant, Arc<T>)>>,
}

impl<T> Cached<T> {
  pub fn new() -> Self {
    Cached {
      slot: Mutex::new(None),
    }
  }

  /// The cached value if it is younger than a second, otherwise a fresh one from
  /// `load`. Concurrent callers wait for the one load in progress instead of each
  /// starting their own.
  pub async fn get<F: Future<Output = T>>(&self, load: impl FnOnce() -> F) -> Arc<T> {
    let mut slot = self.slot.lock().await;
    if let Some((taken, value)) = slot.as_ref()
      && taken.elapsed() < MAX_AGE
    {
      return value.clone();
    }
    let value = Arc::new(load().await);
    *slot = Some((Instant::now(), value.clone()));
    value
  }
}
