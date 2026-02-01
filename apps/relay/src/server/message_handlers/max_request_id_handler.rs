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

use crate::server::client::MOQTClient;
use crate::server::session_context::SessionContext;
use core::result::Result;
use moqtail::model::control::control_message::ControlMessage;
use moqtail::model::error::TerminationCode;
use moqtail::transport::control_stream_handler::ControlStreamHandler;
use std::sync::Arc;
use tracing::{info, warn};

pub async fn handle(
  _client: Arc<MOQTClient>,
  _control_stream_handler: &mut ControlStreamHandler,
  msg: ControlMessage,
  context: Arc<SessionContext>,
) -> Result<(), TerminationCode> {
  match msg {
    ControlMessage::MaxRequestId(m) => {
      // TODO: the namespace is already announced, return error
      info!("received MaxRequestId message");

      let new_max_request_id = m.request_id;

      loop {
        let current = context
          .max_request_id
          .load(std::sync::atomic::Ordering::Relaxed);
        if current >= new_max_request_id {
          warn!("received MaxRequestId message with lower request id than previously announced");
          return Err(TerminationCode::ProtocolViolation);
        }
        if context
          .max_request_id
          .compare_exchange(
            current,
            new_max_request_id,
            std::sync::atomic::Ordering::Relaxed,
            std::sync::atomic::Ordering::Relaxed,
          )
          .is_ok()
        {
          break;
        }
      }
      info!("setting new maxrequest id to {}", new_max_request_id);
      Ok(())
    }
    _ => {
      // no-op
      Ok(())
    }
  }
}
