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

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use moqtail::model::data::full_track_name::FullTrackName;
use moqtail::transport::data_stream_handler::{FetchRequest, SubscribeRequest};
use tokio::sync::{RwLock, watch};

use crate::server::message_handlers::fetch_handler::FetchStop;

/// One request a client or downstream relay sent.
#[allow(dead_code)]
// TODO: Remove the dead_code attribute after merging all the migration changes.
// These changes are required for message unification and will be useful in other branches.
#[derive(Debug, Clone)]
pub enum DownstreamRequest {
  Fetch(FetchRequest),
  Subscribe(SubscribeRequest),
  PublishNamespace {
    client_connection_id: usize,
    original_request_id: u64,
    message: moqtail::model::control::publish_namespace::PublishNamespace,
  },
  SubscribeNamespace {
    client_connection_id: usize,
    original_request_id: u64,
    message: moqtail::model::control::subscribe_namespace::SubscribeNamespace,
  },
  SubscribeTracks {
    client_connection_id: usize,
    original_request_id: u64,
    message: moqtail::model::control::subscribe_tracks::SubscribeTracks,
  },
  Publish {
    publisher_connection_id: usize,
    original_request_id: u64,
    message: moqtail::model::control::publish::Publish,
  },
  RequestUpdate {
    client_connection_id: usize,
    original_request_id: u64,
    message: moqtail::model::control::request_update::RequestUpdate,
  },
}

/// The requests a client or downstream relay has sent, by request id, with a
/// fetch's stop signal kept beside it so the two are removed together.
#[derive(Clone, Debug, Default)]
pub struct DownstreamRequests {
  inner: Arc<RwLock<Requests>>,
}

#[derive(Debug, Default)]
struct Requests {
  requests: BTreeMap<u64, DownstreamRequest>,
  fetch_stops: HashMap<u64, watch::Sender<FetchStop>>,
}

impl Requests {
  fn stop_fetch(&mut self, request_id: u64, reason: FetchStop) {
    self
      .fetch_stops
      .entry(request_id)
      .or_insert_with(|| watch::channel(reason).0)
      .send_replace(reason);
  }

  fn remove(&mut self, request_id: u64) -> Option<DownstreamRequest> {
    self.fetch_stops.remove(&request_id);
    self.requests.remove(&request_id)
  }
}

impl DownstreamRequests {
  pub async fn get(&self, request_id: u64) -> Option<DownstreamRequest> {
    self.inner.read().await.requests.get(&request_id).cloned()
  }

  pub async fn get_subscribe(&self, request_id: u64) -> Option<SubscribeRequest> {
    match self.inner.read().await.requests.get(&request_id) {
      Some(DownstreamRequest::Subscribe(request)) => Some(request.clone()),
      _ => None,
    }
  }

  pub async fn insert(&self, request_id: u64, request: DownstreamRequest) {
    self
      .inner
      .write()
      .await
      .requests
      .insert(request_id, request);
  }

  /// Changes the request in place. `change` returns None for a request it does not
  /// apply to, and so does this when there is no request.
  pub async fn update<R>(
    &self,
    request_id: u64,
    change: impl FnOnce(&mut DownstreamRequest) -> Option<R>,
  ) -> Option<R> {
    self
      .inner
      .write()
      .await
      .requests
      .get_mut(&request_id)
      .and_then(change)
  }

  /// Removes the request and its stop signal.
  pub async fn remove(&self, request_id: u64) -> Option<DownstreamRequest> {
    self.inner.write().await.remove(request_id)
  }

  /// The fetch's stop signal, Running unless it was stopped before delivery began.
  pub async fn watch_fetch(&self, request_id: u64) -> watch::Receiver<FetchStop> {
    self
      .inner
      .write()
      .await
      .fetch_stops
      .entry(request_id)
      .or_insert_with(|| watch::channel(FetchStop::Running).0)
      .subscribe()
  }

  /// Tells a fetch's delivery to stop. The signal stays until the request is
  /// removed: a delivery whose signal is dropped can no longer wait on it.
  pub async fn stop_fetch(&self, request_id: u64, reason: FetchStop) {
    self.inner.write().await.stop_fetch(request_id, reason);
  }

  /// Stops the fetch and removes it, under one lock.
  pub async fn cancel_fetch(&self, request_id: u64) -> Option<DownstreamRequest> {
    let mut inner = self.inner.write().await;
    inner.stop_fetch(request_id, FetchStop::Cancelled);
    inner.remove(request_id)
  }

  /// Stops every fetch of the track.
  pub async fn stop_fetches_of(&self, track: &FullTrackName, reason: FetchStop) {
    let mut inner = self.inner.write().await;
    let ids: Vec<u64> = inner
      .requests
      .iter()
      .filter_map(|(id, request)| match request {
        DownstreamRequest::Fetch(fetch) if fetch.fetch_request.get_full_track_name() == *track => {
          Some(*id)
        }
        _ => None,
      })
      .collect();
    for id in ids {
      inner.stop_fetch(id, reason);
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[tokio::test]
  async fn removing_a_request_drops_its_stop_signal_and_a_stop_outlives_the_signal_it_sent() {
    let table = DownstreamRequests::default();
    let rx = table.watch_fetch(1).await;
    table.stop_fetch(1, FetchStop::UpdateFailed).await;
    assert_eq!(*rx.borrow(), FetchStop::UpdateFailed);
    assert!(table.inner.read().await.fetch_stops.contains_key(&1));
    table.remove(1).await;
    assert!(table.inner.read().await.fetch_stops.is_empty());
    assert_eq!(*table.watch_fetch(1).await.borrow(), FetchStop::Running);
  }
}
