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

//! A hook that sees every control message a [`ControlStreamHandler`] sends or
//! receives, for counting and matching requests to responses.
//!
//! Unlike qlog it builds nothing: the observer gets the parsed message itself. With
//! no observer installed, each message costs one check.
//!
//! [`ControlStreamHandler`]: super::control_stream_handler::ControlStreamHandler

use crate::model::control::control_message::ControlMessage;
use std::sync::{Arc, OnceLock};

pub trait ControlObserver: Send + Sync {
  /// A control message was sent (`sent`) or received on stream `stream_id` of
  /// connection `conn`.
  fn control_message(&self, conn: usize, stream_id: u64, sent: bool, message: &ControlMessage);

  /// Bytes received on stream `stream_id` of connection `conn` did not parse.
  fn parse_error(&self, _conn: usize, _stream_id: u64) {}
}

static OBSERVER: OnceLock<Arc<dyn ControlObserver>> = OnceLock::new();

/// Installs the observer for the rest of the process. Later calls are ignored.
pub fn set_control_observer(observer: Arc<dyn ControlObserver>) {
  let _ = OBSERVER.set(observer);
}

pub(crate) fn installed() -> bool {
  OBSERVER.get().is_some()
}

pub(crate) fn control_message(conn: usize, stream_id: u64, sent: bool, message: &ControlMessage) {
  if let Some(observer) = OBSERVER.get() {
    observer.control_message(conn, stream_id, sent, message);
  }
}

pub(crate) fn parse_error(conn: usize, stream_id: u64) {
  if let Some(observer) = OBSERVER.get() {
    observer.parse_error(conn, stream_id);
  }
}
