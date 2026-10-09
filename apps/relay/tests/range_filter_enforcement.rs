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

//! Every request that may carry a Range Filter has to answer for one.
//!
//! `range_filter_rejection` is unit-tested where it lives; what is untestable
//! there is whether anything calls it. The handlers are async and reach deep
//! into session state, so there is no cheap harness that drives a SUBSCRIBE
//! through one -- and the failure this guards is silent in exactly the way a
//! missing test is: §5.1.4's filters parse and validate fine, the relay simply
//! never applies them, and the subscriber receives the Objects it asked not to
//! receive with no error anywhere.
//!
//! So the call site is held by the source. A handler added later for a request
//! that takes parameters belongs in this list.

use std::fs;
use std::path::Path;

/// Requests whose parameters §5.1.4 lets a Range Filter ride.
const HANDLERS_TAKING_FILTERS: &[&str] = &[
  "subscribe_handler.rs",
  "fetch_handler.rs",
  "subscribe_tracks_handler.rs",
];

#[test]
fn every_request_handler_that_can_receive_a_range_filter_rejects_one() {
  let handlers = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/server/message_handlers");
  for name in HANDLERS_TAKING_FILTERS {
    let path = handlers.join(name);
    let source = fs::read_to_string(&path).unwrap_or_else(|e| panic!("reading {path:?}: {e}"));
    assert!(
      source.contains("range_filter_rejection("),
      "{name} handles a request that may carry a Range Filter but never calls \
       range_filter_rejection; a filter it accepts is one it will not apply"
    );
  }
}
