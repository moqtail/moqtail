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

//! The relay's own requests upstream for a track, so they can be cancelled together.

use super::Track;
use std::sync::atomic::Ordering;
use tokio::sync::oneshot;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UpstreamRequestKind {
  Subscribe,
  Fetch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UpstreamRequestCancellation {
  Cancelled,
  MalformedTrack,
}

#[derive(Debug)]
struct UpstreamRequest {
  request_id: u64,
  publisher_connection_id: usize,
  kind: UpstreamRequestKind,
  cancel: oneshot::Sender<UpstreamRequestCancellation>,
}

#[derive(Debug, Default)]
pub(super) struct UpstreamRequests(Vec<UpstreamRequest>);

impl UpstreamRequests {
  fn register(
    &mut self,
    request_id: u64,
    publisher_connection_id: usize,
    kind: UpstreamRequestKind,
  ) -> oneshot::Receiver<UpstreamRequestCancellation> {
    self.0.retain(|request| !request.cancel.is_closed());
    self.unregister(request_id);
    let (cancel, receiver) = oneshot::channel();
    self.0.push(UpstreamRequest {
      request_id,
      publisher_connection_id,
      kind,
      cancel,
    });
    receiver
  }

  fn unregister(&mut self, request_id: u64) {
    self.0.retain(|request| request.request_id != request_id);
  }

  fn cancel_matching(
    &mut self,
    mut matches: impl FnMut(&UpstreamRequest) -> bool,
    reason: UpstreamRequestCancellation,
  ) {
    let mut remaining = Vec::with_capacity(self.0.len());
    for request in self.0.drain(..) {
      if request.cancel.is_closed() {
        continue;
      } else if matches(&request) {
        let _ = request.cancel.send(reason);
      } else {
        remaining.push(request);
      }
    }
    self.0 = remaining;
  }

  fn cancel_for_malformed_publisher(&mut self, publisher_connection_id: usize) {
    self.cancel_matching(
      |request| request.publisher_connection_id == publisher_connection_id,
      UpstreamRequestCancellation::MalformedTrack,
    );
    self.cancel_matching(
      |request| request.kind == UpstreamRequestKind::Fetch,
      UpstreamRequestCancellation::MalformedTrack,
    );
    self.cancel_matching(|_| true, UpstreamRequestCancellation::Cancelled);
  }
}

impl Track {
  pub(crate) async fn register_upstream_request(
    &self,
    request_id: u64,
    publisher_connection_id: usize,
    kind: UpstreamRequestKind,
  ) -> oneshot::Receiver<UpstreamRequestCancellation> {
    self
      .upstream_requests
      .lock()
      .await
      .register(request_id, publisher_connection_id, kind)
  }

  pub(crate) fn upstream_subscribe_answered(&self) -> usize {
    let mut count = self.pending_upstream_subscribe_count.load(Ordering::SeqCst);
    while count > 0 {
      match self.pending_upstream_subscribe_count.compare_exchange_weak(
        count,
        count - 1,
        Ordering::SeqCst,
        Ordering::SeqCst,
      ) {
        Ok(_) => return count - 1,
        Err(current) => count = current,
      }
    }
    0
  }

  pub(crate) async fn unregister_upstream_request(&self, request_id: u64) {
    self.upstream_requests.lock().await.unregister(request_id);
  }

  pub(crate) async fn cancel_upstream_request(
    &self,
    request_id: u64,
    reason: UpstreamRequestCancellation,
  ) {
    self
      .upstream_requests
      .lock()
      .await
      .cancel_matching(|request| request.request_id == request_id, reason);
  }

  pub(crate) async fn cancel_for_malformed_publisher(&self, publisher_connection_id: usize) {
    self
      .upstream_requests
      .lock()
      .await
      .cancel_for_malformed_publisher(publisher_connection_id);
  }

  pub(crate) async fn cancel_upstream_requests_of_kind(
    &self,
    kind: UpstreamRequestKind,
    reason: UpstreamRequestCancellation,
  ) {
    self
      .upstream_requests
      .lock()
      .await
      .cancel_matching(|request| request.kind == kind, reason);
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  const PUBLISHER: usize = 7;
  const OTHER_PUBLISHER: usize = 9;

  #[tokio::test]
  async fn upstream_request_cancellation_is_scoped_and_unregisters() {
    let mut requests = UpstreamRequests::default();
    let bad_subscribe = requests.register(1, PUBLISHER, UpstreamRequestKind::Subscribe);
    let bad_fetch = requests.register(3, PUBLISHER, UpstreamRequestKind::Fetch);
    let other_subscribe = requests.register(5, OTHER_PUBLISHER, UpstreamRequestKind::Subscribe);
    let other_fetch = requests.register(7, OTHER_PUBLISHER, UpstreamRequestKind::Fetch);

    requests.cancel_for_malformed_publisher(PUBLISHER);
    assert_eq!(
      bad_subscribe.await,
      Ok(UpstreamRequestCancellation::MalformedTrack)
    );
    assert_eq!(
      bad_fetch.await,
      Ok(UpstreamRequestCancellation::MalformedTrack)
    );
    assert_eq!(
      other_subscribe.await,
      Ok(UpstreamRequestCancellation::Cancelled)
    );
    assert_eq!(
      other_fetch.await,
      Ok(UpstreamRequestCancellation::MalformedTrack)
    );

    drop(requests.register(9, PUBLISHER, UpstreamRequestKind::Fetch));
    let live = requests.register(11, PUBLISHER, UpstreamRequestKind::Fetch);
    assert_eq!(requests.0.len(), 1);
    requests.unregister(11);
    assert!(live.await.is_err());
  }
}
