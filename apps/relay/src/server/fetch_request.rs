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

use moqtail::model::control::fetch::Fetch;
use tokio::sync::{RwLock, Arc};
use moqtail::client::MOQTClient;
use wtransport::SendStream;

pub struct FetchRequest {
  pub fetch_message: Fetch,
  pub client: Arc<RwLock<MOQTClient>>,
  pub send_stream: SendStream,
  pub finished: Arc<RwLock<bool>>,
}

impl FetchRequest {
  pub fn new(fetch_message: Fetch, client: Arc<RwLock<MOQTClient>>, send_stream: SendStream) -> Self {
    Self { fetch_message, client, send_stream, finished: Arc::new(RwLock::new(false)) }
  }
}