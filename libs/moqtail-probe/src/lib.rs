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

//! The interface between the relay core and its observability layer.
//!
//! The core implements [`Inspect`] so the observability layer can read its state as
//! plain copies, without touching the core's locks. Everything that serves or
//! exports that state lives in the observability crate, which the core never
//! imports.

use serde::Serialize;
use std::future::Future;
use std::pin::Pin;

/// A boxed future, so [`Inspect`] can be used as a trait object.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Read-only access to the relay's state.
///
/// Each method copies what it needs while holding the core's locks only for the
/// copy, then returns plain data. Callers never hold a core lock, so a slow reader
/// cannot stall delivery.
pub trait Inspect: Send + Sync {
  /// The relay's identity and lifecycle state.
  fn status(&self) -> RelayStatus;

  /// The effective configuration, with every secret left out.
  fn config(&self) -> serde_json::Value;
}

/// The relay's identity and lifecycle state.
#[derive(Debug, Clone, Serialize)]
pub struct RelayStatus {
  /// Build version, for example `moqtail/0.18.0+9e8bf05f`.
  pub version: String,
  /// Process id.
  pub pid: u32,
  /// When the relay started, in milliseconds since the Unix epoch.
  pub started_at_ms: u64,
  /// True once the QUIC listeners accept connections.
  pub accepting: bool,
  /// True after GOAWAY was broadcast; new requests are rejected from then on.
  pub draining: bool,
}
