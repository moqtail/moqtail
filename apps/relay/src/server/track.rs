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

use super::seen_objects::SeenObjects;
use super::track_cache::TrackCache;
use crate::server::config::AppConfig;
use crate::server::object_logger::ObjectLogger;
use crate::server::stream_id::StreamId;
use crate::server::subscription::Subscription;
use crate::server::subscription_manager::{SubscriptionId, SubscriptionManager};
use crate::server::utils;
use crate::server::{client::MOQTClient, subscription::SubscriptionOrigin};
use anyhow::Result;
use moqtail::model::common::location::Location;
use moqtail::model::common::reason_phrase::ReasonPhrase;
use moqtail::model::control::constant::PublishDoneStatusCode;
use moqtail::model::control::publish_done::PublishDone;
use moqtail::model::data::constant::{DEFAULT_PUBLISHER_PRIORITY, ObjectStatus};
use moqtail::model::data::datagram::Datagram;
use moqtail::model::data::full_track_name::FullTrackName;
use moqtail::model::data::object::Object;
use moqtail::model::error::RequestErrorCode;
use moqtail::model::parameter::message_parameter::MessageParameter;
use moqtail::model::property::object_property::ObjectProperty;
use moqtail::model::property::track_property::TrackProperty;
use moqtail::transport::data_stream_handler::HeaderInfo;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;
use tokio::sync::{Mutex, MutexGuard, Notify, RwLock, oneshot};
use tokio::time::Instant;
use tracing::{debug, error, info, trace, warn};

pub type ActiveSubgroupHeaderMap = Arc<RwLock<HashMap<StreamId, HeaderInfo>>>;

/// What removing one of a track's publishers left behind.
///
/// `still_served` is the question callers actually have to answer: a track
/// several publishers serve is only finished when the last of them goes, and
/// until then it is merely thinner. `remove_publisher` already works this out
/// to decide whether to notify the subscribers, so it reports it rather than
/// leaving each caller to ask again — and the callers that forgot to ask are
/// how a track still being published ended up looking finished.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PublisherRemoval {
  /// The track alias the removed publisher was using.
  pub alias: u64,
  /// Another publisher is still serving the track, so it is not over.
  pub still_served: bool,
}

/// How many data streams each publisher has finished sending for one track.
///
/// A PUBLISH_DONE carries the number of data streams its sender opened, and it
/// travels on the request stream, which overtakes those streams. Ending the
/// downstream subscriptions the moment it lands would cut off objects still
/// arriving, so the relay waits here for the streams to be accounted for first.
#[derive(Debug, Default)]
pub struct PublisherStreamProgress {
  /// publisher_connection_id -> data streams finished for this track.
  closed: RwLock<HashMap<usize, u64>>,
  changed: Notify,
}

impl PublisherStreamProgress {
  /// Record one finished data stream from this publisher.
  pub async fn stream_closed(&self, publisher_connection_id: usize) {
    {
      let mut closed = self.closed.write().await;
      *closed.entry(publisher_connection_id).or_insert(0) += 1;
    }
    self.changed.notify_waiters();
  }

  pub async fn closed_count(&self, publisher_connection_id: usize) -> u64 {
    self
      .closed
      .read()
      .await
      .get(&publisher_connection_id)
      .copied()
      .unwrap_or(0)
  }

  /// Drop a publisher's count once it no longer publishes the track.
  pub async fn forget(&self, publisher_connection_id: usize) {
    self.closed.write().await.remove(&publisher_connection_id);
  }

  /// Wait for `stream_count` of this publisher's data streams to finish, giving up
  /// after `timeout`. Returns how many had finished when the wait ended; a result
  /// below `stream_count` means it timed out.
  pub async fn wait_for(
    &self,
    publisher_connection_id: usize,
    stream_count: u64,
    timeout: Duration,
  ) -> u64 {
    let wait = async {
      loop {
        // Registered before the count is read, so a stream finishing between the
        // two still wakes this loop rather than leaving it to time out.
        let changed = self.changed.notified();
        tokio::pin!(changed);
        changed.as_mut().enable();

        let closed = self.closed_count(publisher_connection_id).await;
        if closed >= stream_count {
          return closed;
        }
        changed.await;
      }
    };

    match tokio::time::timeout(timeout, wait).await {
      Ok(closed) => closed,
      Err(_) => self.closed_count(publisher_connection_id).await,
    }
  }
}

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
struct UpstreamRequests(Vec<UpstreamRequest>);

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

/// What a track leaves the track manager to clean up once it lets go of its upstream.
#[derive(Debug, PartialEq, Eq)]
pub enum Cleanup {
  /// Nothing: the track is still in use and unchanged.
  Nothing,
  /// The track stays, as a cache-only track serving what it has cached, but no
  /// publisher feeds it any more. These (connection ID, track alias) pairs no longer
  /// lead to it and are to be removed from the alias map.
  RemoveAliases(Vec<(usize, u64)>),
  /// The track holds nothing and is to be removed.
  RemoveTrack,
}

/// When the last datagram for a track arrived, so a PUBLISH_DONE can wait for the
/// datagrams it would otherwise overtake. Datagrams are not counted in its Stream
/// Count, so the relay cannot tell when the last one is in; it waits for them to go
/// quiet instead.
#[derive(Debug, Default)]
pub struct DatagramArrivals {
  /// Microseconds from `utils::BASE_TIME` to the last arrival, plus one; 0 before any.
  last: AtomicU64,
}

impl DatagramArrivals {
  pub fn arrived(&self) {
    let micros = utils::BASE_TIME.elapsed().as_micros() as u64;
    self.last.store(micros + 1, Ordering::Relaxed);
  }

  /// Waits until no datagram has arrived for `quiet`, or until `deadline`. A track
  /// that has had no datagram does not wait at all.
  pub async fn wait_for_quiet(&self, quiet: Duration, deadline: Instant) {
    loop {
      let last = match self.last.load(Ordering::Relaxed) {
        0 => return,
        micros => Instant::from_std(*utils::BASE_TIME) + Duration::from_micros(micros - 1),
      };
      let until = (last + quiet).min(deadline);
      if Instant::now() >= until {
        return;
      }
      tokio::time::sleep_until(until).await;
    }
  }
}

/// What feeds a track: what created it, until its upstream goes away and it becomes
/// `CacheOnly`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum TrackOrigin {
  /// Created by a downstream SUBSCRIBE. The relay owns the upstream subscription; with
  /// no subscribers left it is cancelled, and the track becomes `CacheOnly` if it holds
  /// anything, or is removed. Either way the next SUBSCRIBE subscribes upstream afresh.
  Subscribe,
  /// Created by an upstream PUBLISH. The publisher keeps pushing regardless of
  /// subscriber count, so the track outlives them.
  Publish,
  /// Fed by nothing live, so it only serves its cache: created by a downstream FETCH of
  /// a track the relay did not have, or kept after its upstream went away while it held
  /// cached Objects. The cache evicts by its own rules. The first SUBSCRIBE or PUBLISH
  /// to reach it takes it over (`take_over`).
  CacheOnly,
}

impl TrackOrigin {
  fn from_u8(value: u8) -> Self {
    match value {
      0 => Self::Subscribe,
      1 => Self::Publish,
      _ => Self::CacheOnly,
    }
  }
}

/// Lifecycle status of a track on the relay.
#[derive(Debug, Clone)]
pub enum TrackStatus {
  /// Track created, subscribe forwarded to publisher, awaiting response.
  Pending,
  /// Publisher confirmed with SubscribeOk, with the parameters it sent. They were
  /// addressed to the relay, so they are what a later SUBSCRIBE_OK is derived from,
  /// never what it forwards.
  Confirmed {
    upstream_parameters: Vec<MessageParameter>,
  },
  /// Publisher rejected with RequestError.
  Rejected {
    error_code: RequestErrorCode,
    reason_phrase: ReasonPhrase,
  },
}

#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
pub enum TrackEvent {
  // Shared by every subscriber of the track, so fanning one out copies nothing.
  SubgroupObject {
    stream_id: StreamId,
    object: Arc<Object>,
    header_info: Option<HeaderInfo>,
  },
  Datagram {
    object: Arc<Datagram>,
  },
  StreamClosed {
    stream_id: StreamId,
  },
  PublisherDisconnected {
    status_code: PublishDoneStatusCode,
    reason: String,
  },
}

#[derive(Debug, Clone)]
pub struct Track {
  /// Stable relay-assigned track identifier, independent of publisher aliases.
  pub relay_track_id: u64,
  pub full_track_name: FullTrackName,
  /// A `TrackOrigin`. It changes when a cache-only track is taken over or a track
  /// becomes cache-only.
  origin: Arc<AtomicU8>,
  /// Held while a SUBSCRIBE takes the track over and joins it, and while the track lets
  /// go of its upstream subscription, so neither can run in the middle of the other.
  handover: Arc<Mutex<()>>,
  pub subscription_manager: SubscriptionManager,
  /// Maps publisher_connection_id -> publisher_track_alias for all active publishers.
  pub publisher_aliases: Arc<RwLock<BTreeMap<usize, u64>>>,
  pub(crate) cache: TrackCache,
  pub largest_location: Arc<RwLock<Location>>,
  /// Object ids already ingested, to drop duplicates from multiple publishers.
  seen_objects: Arc<SeenObjects>,
  /// Set once at least one Object (subgroup or datagram) has been seen for this
  /// track, so `largest_location` becomes meaningful (it starts at {0,0}).
  has_objects: Arc<AtomicBool>,
  pub object_logger: ObjectLogger,
  config: &'static AppConfig,
  status: Arc<RwLock<TrackStatus>>,
  pub status_notify: Arc<Notify>,
  /// Subscribers waiting for track confirmation: (request_id, connection_id).
  pending_subscribers: Arc<RwLock<Vec<(u64, usize)>>>,
  /// Cached track properties from PUBLISH or SUBSCRIBE_OK (relays MUST cache).
  track_properties: Arc<RwLock<Vec<TrackProperty>>>,
  /// Original subgroup headers for open publisher streams, keyed by stream_id.
  /// Used so new mid-group subscribers can open a QUIC send stream
  /// Inserted when the first object of a subgroup arrives; removed when the
  /// publisher's unistream closes (stream_closed signal).
  pub active_subgroup_headers: ActiveSubgroupHeaderMap,
  /// Data streams each publisher has finished sending, so a PUBLISH_DONE can be
  /// held until the streams it accounts for have been taken in.
  pub publisher_stream_progress: Arc<PublisherStreamProgress>,
  /// When its datagrams arrive, which a PUBLISH_DONE waits on (see DatagramArrivals).
  pub datagram_arrivals: Arc<DatagramArrivals>,
  /// Relay-owned request streams that may need to be cancelled as a group.
  upstream_requests: Arc<Mutex<UpstreamRequests>>,
  /// Upstream SUBSCRIBEs still awaiting a SUBSCRIBE_OK or REQUEST_ERROR.
  pub pending_upstream_subscribe_count: Arc<AtomicUsize>,
  /// Forward State the relay last gave the upstream publisher of a
  /// PUBLISH-created track. A relay may answer PUBLISH with Forward State 0
  /// while nothing downstream wants the track; this records whether it has
  /// since been raised, so it is raised only once.
  pub upstream_forward: Arc<AtomicBool>,
}

// TODO: this track implementation should be static? At least
// its lifetime should be same as the server's lifetime
impl Track {
  /// SSTS: remove this track from the switching sets of all its subscribers
  /// (deleting emptied sets).
  pub async fn remove_from_subscriber_switching_sets(&self) {
    let subscriptions = self.subscription_manager.get_all_subscriptions().await;
    for sub in &subscriptions {
      let subscriber = sub.read().await.subscriber().clone();
      // A connection that never negotiated SSTS holds no switching sets, so
      // there is nothing here to take this track out of. Checking costs a
      // length comparison, where the removal it skips costs a write lock, so a
      // relay with the feature off takes no locks here at all.
      if !subscriber.ssts_enabled() {
        continue;
      }
      subscriber
        .ssts
        .switching_sets
        .write()
        .await
        .remove(&self.full_track_name);
    }
  }

  pub fn new(
    relay_track_id: u64,
    full_track_name: FullTrackName,
    config: &'static AppConfig,
    initial_status: TrackStatus,
    origin: TrackOrigin,
  ) -> Self {
    Track {
      relay_track_id,
      full_track_name: full_track_name.clone(),
      origin: Arc::new(AtomicU8::new(origin as u8)),
      handover: Arc::new(Mutex::new(())),
      subscription_manager: SubscriptionManager::new(
        relay_track_id,
        full_track_name,
        config.log_folder.clone(),
        config,
      ),
      publisher_aliases: Arc::new(RwLock::new(BTreeMap::new())),
      cache: TrackCache::new(relay_track_id, config.cache_size.into(), config),
      largest_location: Arc::new(RwLock::new(Location::new(0, 0))),
      seen_objects: Arc::new(SeenObjects::new(config.dedup_retained_groups)),
      has_objects: Arc::new(AtomicBool::new(false)),
      object_logger: ObjectLogger::new(config.log_folder.clone()),
      config,
      status: Arc::new(RwLock::new(initial_status)),
      status_notify: Arc::new(Notify::new()),
      pending_subscribers: Arc::new(RwLock::new(Vec::new())),
      track_properties: Arc::new(RwLock::new(Vec::new())),
      active_subgroup_headers: Arc::new(RwLock::new(HashMap::new())),
      publisher_stream_progress: Arc::new(PublisherStreamProgress::default()),
      datagram_arrivals: Arc::new(DatagramArrivals::default()),
      upstream_requests: Arc::new(Mutex::new(UpstreamRequests::default())),
      pending_upstream_subscribe_count: Arc::new(AtomicUsize::new(0)),
      upstream_forward: Arc::new(AtomicBool::new(false)),
    }
  }

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

  pub fn origin(&self) -> TrackOrigin {
    TrackOrigin::from_u8(self.origin.load(Ordering::Acquire))
  }

  /// See `handover`. A SUBSCRIBE holds it from `take_over` until it has joined.
  pub async fn lock_handover(&self) -> MutexGuard<'_, ()> {
    self.handover.lock().await
  }

  /// Hands a cache-only track over to a SUBSCRIBE or PUBLISH, keeping what is cached.
  /// Returns true only for the caller that took it, which must then do what creating
  /// the track would have: nothing upstream has been asked for it yet.
  pub async fn take_over(&self, origin: TrackOrigin, status: TrackStatus) -> bool {
    let taken = self
      .origin
      .compare_exchange(
        TrackOrigin::CacheOnly as u8,
        origin as u8,
        Ordering::AcqRel,
        Ordering::Acquire,
      )
      .is_ok();
    if taken {
      *self.status.write().await = status;
    }
    taken
  }

  /// Removes a subscription and, once no subscriber is left, cancels the upstream
  /// subscriptions and lets go of them. All under `handover`, so a SUBSCRIBE cannot join
  /// after the cancel and be left on an upstream that is gone.
  pub async fn remove_subscriber(&self, leaving: &SubscriptionId) -> Cleanup {
    let _handover = self.lock_handover().await;
    self.remove_subscription(leaving).await;
    if self.subscriber_count().await > 0 {
      return Cleanup::Nothing;
    }
    // One per publisher serving the track, so each observes the cancellation.
    self
      .cancel_upstream_requests_of_kind(
        UpstreamRequestKind::Subscribe,
        UpstreamRequestCancellation::Cancelled,
      )
      .await;
    if self.origin() != TrackOrigin::Subscribe {
      return Cleanup::Nothing;
    }
    self.become_cache_only().await
  }

  /// Lets go of a track nothing upstream serves any more: its last publisher left, or
  /// its upstream SUBSCRIBE was refused. Its subscribers have been told the track
  /// ended and finish on their own; they are detached, so whatever takes the track
  /// over next cannot feed them.
  pub async fn retire(&self) -> Cleanup {
    let _handover = self.lock_handover().await;
    let cleanup = self.become_cache_only().await;
    // The track stays, so whatever takes it over next must not find these subscribers.
    if matches!(cleanup, Cleanup::RemoveAliases(_)) {
      self.subscription_manager.detach_all().await;
    }
    cleanup
  }

  /// Makes the track cache-only if its cache holds anything, so the next SUBSCRIBE or
  /// PUBLISH starts upstream afresh. A track holding nothing is left as it is, for the
  /// caller to remove. The caller holds `handover`.
  async fn become_cache_only(&self) -> Cleanup {
    if self.cache.is_empty().await {
      return Cleanup::RemoveTrack;
    }
    *self.status.write().await = TrackStatus::Pending;
    let aliases: Vec<(usize, u64)> = std::mem::take(&mut *self.publisher_aliases.write().await)
      .into_iter()
      .collect();
    for (connection_id, _) in &aliases {
      self.publisher_stream_progress.forget(*connection_id).await;
    }
    self
      .cancel_upstream_requests_of_kind(
        UpstreamRequestKind::Subscribe,
        UpstreamRequestCancellation::Cancelled,
      )
      .await;
    self.active_subgroup_headers.write().await.clear();
    self
      .pending_upstream_subscribe_count
      .store(0, Ordering::SeqCst);
    self.upstream_forward.store(false, Ordering::Relaxed);
    self
      .origin
      .store(TrackOrigin::CacheOnly as u8, Ordering::Release);
    Cleanup::RemoveAliases(aliases)
  }

  /// Whether any subscription on this track currently wants Objects forwarded.
  pub async fn has_forwarding_subscriber(&self) -> bool {
    for sub in self.subscription_manager.get_all_subscriptions().await {
      if sub.read().await.is_forwarding().await {
        return true;
      }
    }
    false
  }

  /// Add a publisher (connection_id -> track_alias) to this track.
  pub async fn add_publisher(&self, connection_id: usize, track_alias: u64) {
    let mut aliases = self.publisher_aliases.write().await;
    aliases.insert(connection_id, track_alias);
    info!(
      "Added publisher {}@alias={} to relay_track_id={}",
      connection_id, track_alias, self.relay_track_id
    );
  }

  /// Take `connection_id` out of this track's publishers, reporting the alias
  /// it held when it was one of them, and whether any publisher still serves
  /// the track.
  ///
  /// Both answers come from one acquisition of the lock, so they cannot
  /// disagree: a publisher arriving or leaving between two reads would
  /// otherwise let a removal report the track as finished while someone else
  /// was already serving it.
  async fn take_publisher(&self, connection_id: usize) -> (Option<u64>, bool) {
    let mut aliases = self.publisher_aliases.write().await;
    let alias = aliases.remove(&connection_id);
    (alias, !aliases.is_empty())
  }

  /// Remove a publisher by connection_id, reporting its alias and whether the
  /// track is still served, or `None` when it was not publishing this track.
  /// If no publishers remain after removal, sends PublisherDisconnected to all subscribers.
  #[must_use = "clear the alias with TrackManager::remove_publisher_alias"]
  pub async fn remove_publisher(&self, connection_id: usize) -> Option<PublisherRemoval> {
    let (alias, still_served) = self.take_publisher(connection_id).await;
    let alias = alias?;

    self.publisher_stream_progress.forget(connection_id).await;
    info!(
      "Removed publisher {}@alias={} from relay_track_id={} | still_served={}",
      connection_id, alias, self.relay_track_id, still_served
    );

    if !still_served && let Err(e) = self.notify_publisher_disconnected().await {
      error!(
        "Failed to notify subscribers after last publisher removed for relay_track_id={}: {:?}",
        self.relay_track_id, e
      );
    }

    Some(PublisherRemoval {
      alias,
      still_served,
    })
  }

  /// Take `connection_id` out of this track's publishers without telling the
  /// subscribers anything, and report whether any publisher still serves it.
  ///
  /// The caller owns the downstream message, which is the whole difference from
  /// `remove_publisher`: a PUBLISH_DONE carries its own status and reason, and
  /// the generic "track ended" that `remove_publisher` sends would either
  /// replace it or arrive alongside it.
  ///
  /// The answer is about the track rather than about this removal, so a
  /// connection that was never one of its publishers still learns truthfully
  /// whether the others are there.
  ///
  /// Deliberately leaves `publisher_stream_progress` alone: a caller that goes
  /// on to wait for this publisher's Stream Count still needs the closures
  /// already recorded for it.
  pub async fn remove_publisher_quietly(&self, connection_id: usize) -> bool {
    let (_, still_served) = self.take_publisher(connection_id).await;
    info!(
      "Removed publisher {} of relay_track_id={} quietly | still_served={}",
      connection_id, self.relay_track_id, still_served
    );
    still_served
  }

  /// Whether the given connection is one of this track's publishers.
  pub async fn is_published_by(&self, connection_id: usize) -> bool {
    self
      .publisher_aliases
      .read()
      .await
      .contains_key(&connection_id)
  }

  /// The Largest Object seen for this track, or None if no Objects have been
  /// published yet (so `largest_location`'s default {0,0} is not mistaken for
  /// a real Object).
  pub async fn largest_object(&self) -> Option<Location> {
    if self.has_objects.load(Ordering::Relaxed) {
      Some(self.largest_location.read().await.clone())
    } else {
      None
    }
  }

  /// Records an Object's location and reports whether it had already been seen.
  /// Only the Object's own group is locked, so this does not serialise the track.
  fn is_duplicate(&self, location: &Location) -> bool {
    self.seen_objects.is_duplicate(location)
  }

  /// Registers an accepting publisher. Returns true only for the one that took the
  /// track out of Pending, since only it has a downstream response to send.
  pub async fn confirm(
    &self,
    publisher_connection_id: usize,
    publisher_track_alias: u64,
    upstream_parameters: Vec<MessageParameter>,
    properties: Vec<TrackProperty>,
  ) -> bool {
    {
      let mut aliases = self.publisher_aliases.write().await;
      aliases.insert(publisher_connection_id, publisher_track_alias);
    }
    *self.track_properties.write().await = properties;

    let mut status = self.status.write().await;
    let was_pending = matches!(*status, TrackStatus::Pending);
    if was_pending {
      *status = TrackStatus::Confirmed {
        upstream_parameters,
      };
    }
    drop(status);

    if was_pending {
      self.status_notify.notify_waiters();
    }

    info!(
      "Track publisher accepted: relay_track_id={} publisher_connection_id={} publisher_alias={} confirmed_now={}",
      self.relay_track_id, publisher_connection_id, publisher_track_alias, was_pending
    );
    was_pending
  }

  /// Updates the cached track properties (per spec: most recent set replaces any previous).
  pub async fn set_track_properties(&self, properties: Vec<TrackProperty>) {
    *self.track_properties.write().await = properties;
  }

  /// The status to answer a joining subscriber with, queuing it while Pending. The
  /// queuing happens under the status lock, which confirm and reject take to change
  /// the status, so no subscriber that saw Pending is missed.
  pub async fn join_status(&self, request_id: u64, connection_id: usize) -> TrackStatus {
    let status = self.status.read().await;
    if matches!(*status, TrackStatus::Pending) {
      self
        .pending_subscribers
        .write()
        .await
        .push((request_id, connection_id));
    }
    status.clone()
  }

  pub async fn status(&self) -> TrackStatus {
    self.status.read().await.clone()
  }

  /// The subscribers that joined while Pending, leaving none behind.
  pub async fn take_pending_subscribers(&self) -> Vec<(u64, usize)> {
    std::mem::take(&mut *self.pending_subscribers.write().await)
  }

  pub async fn track_properties(&self) -> Vec<TrackProperty> {
    self.track_properties.read().await.clone()
  }

  /// Transition from Pending to Rejected. Notifies waiters.
  pub async fn reject(&self, error_code: RequestErrorCode, reason_phrase: ReasonPhrase) {
    let mut status = self.status.write().await;
    *status = TrackStatus::Rejected {
      error_code,
      reason_phrase,
    };
    drop(status);
    self.status_notify.notify_waiters();
  }

  pub async fn add_subscription(
    &self,
    subscriber: Arc<MOQTClient>,
    origin_message: impl Into<SubscriptionOrigin>,
  ) -> Result<Arc<RwLock<Subscription>>> {
    let origin_enum = origin_message.into();
    let subscription = self
      .subscription_manager
      .add_subscription(
        subscriber,
        origin_enum,
        self.cache.clone(),
        Arc::clone(&self.active_subgroup_headers),
      )
      .await?;

    // Where a live filter starts is stated against the largest Object, which only
    // the track knows.
    subscription
      .read()
      .await
      .resolve_live_start(self.largest_object().await)
      .await;

    Ok(subscription)
  }

  pub async fn has_any_subscription(&self, subscriber_id: usize) -> bool {
    self
      .subscription_manager
      .has_any_subscription(subscriber_id)
      .await
  }

  pub async fn has_subscription(&self, subscription_id: &SubscriptionId) -> bool {
    self.subscription_manager.exists(subscription_id).await
  }

  // return the subscription for the client
  // subscriber_id is the connection id of the client
  pub async fn get_subscription(
    &self,
    subscription_id: &SubscriptionId,
  ) -> Option<Arc<RwLock<Subscription>>> {
    self
      .subscription_manager
      .get_subscription(subscription_id)
      .await
  }

  pub async fn remove_subscription(&self, subscription_id: &SubscriptionId) {
    self
      .subscription_manager
      .remove_subscription(subscription_id)
      .await
  }

  pub async fn remove_subscriptions(&self, subscriber_id: usize) {
    self
      .subscription_manager
      .remove_subscriptions(subscriber_id)
      .await
  }

  pub async fn subscriber_count(&self) -> usize {
    self.subscription_manager.subscriber_count().await
  }

  /// Caches an Object an upstream FETCH brought in. It is not handed to subscribers,
  /// which get the live feed, nor checked against the Objects already seen: a group
  /// evicted from the cache has to be fetchable into it again.
  pub async fn cache_fetched_object(&self, object: &Object) {
    if let Ok(fetch_object) = object.clone().try_into_fetch() {
      self.cache.add_object(fetch_object).await;
    }
    self.record_arrival(object).await;
  }

  /// Logs an Object's arrival and moves the Largest Location on to it.
  async fn record_arrival(&self, object: &Object) {
    if self.config.enable_object_logging {
      self
        .object_logger
        .log_track_object(
          self.relay_track_id,
          object,
          utils::passed_time_since_start(),
        )
        .await;
    }
    {
      let mut largest_location = self.largest_location.write().await;
      if object.location > *largest_location {
        *largest_location = object.location.clone();
      }
    }
    self.has_objects.store(true, Ordering::Relaxed);
  }

  /// Records what a live Object says does not exist, so the cache can serve a group
  /// once it accounts for all of it: what an End of Group or End of Track status
  /// rules out, what follows an Object that ends its group, and the IDs a Prior Group
  /// or Object ID Gap skips.
  fn note_absent(&self, object: &Object, ends_its_group: bool) {
    let Location { group, object: id } = object.location.clone();
    let group_end = Location::new(group, u64::MAX);
    match object.status {
      ObjectStatus::EndOfGroup => self.cache.mark_absent(object.location.clone(), group_end),
      ObjectStatus::EndOfTrack => self
        .cache
        .mark_absent(object.location.clone(), Location::new(u64::MAX, u64::MAX)),
      ObjectStatus::Normal => {}
    }
    if ends_its_group {
      self.group_ended_after(&object.location);
    }
    for property in object.properties.iter().flatten() {
      match property {
        ObjectProperty::PriorObjectIdGap { gap } if *gap > 0 && *gap <= id => {
          self
            .cache
            .mark_absent(Location::new(group, id - gap), Location::new(group, id - 1));
        }
        ObjectProperty::PriorGroupIdGap { gap } if *gap > 0 && *gap <= group => {
          self.cache.mark_absent(
            Location::new(group - gap, 0),
            Location::new(group - 1, u64::MAX),
          )
        }
        _ => {}
      }
    }
  }

  /// Called when a subgroup stream that holds the end of its group closes with a FIN:
  /// nothing in the group comes after the last Object it carried.
  pub fn group_ended_after(&self, last: &Location) {
    if last.object < u64::MAX {
      self
        .cache
        .mark_absent(last.next().unwrap(), Location::new(last.group, u64::MAX));
    }
  }

  pub async fn new_subgroup_object(
    &self,
    stream_id: &StreamId,
    object: &Object,
    header_info: Option<&HeaderInfo>,
  ) -> Result<()> {
    trace!(
      "new_subgroup_object: relay_track_id={} location: {:?} stream_id={} diff_ms={}",
      self.relay_track_id,
      object.location,
      stream_id,
      utils::passed_time_since_start()
    );

    if self.is_duplicate(&object.location) {
      trace!(
        "new_subgroup_object: dropping duplicate | relay_track_id={} location: {:?}",
        self.relay_track_id, object.location
      );
      return Ok(());
    }

    if let Some(h) = header_info {
      debug!(
        "new group: relay_track_id={} location: {:?} stream_id={} time={}",
        self.relay_track_id,
        object.location,
        stream_id,
        utils::passed_time_since_start()
      );
      self
        .active_subgroup_headers
        .write()
        .await
        .insert(stream_id.clone(), h.clone());
    }

    // Send single Object event with optional header info
    let event = TrackEvent::SubgroupObject {
      stream_id: stream_id.clone(),
      object: Arc::new(object.clone()),
      header_info: header_info.cloned(),
    };

    self
      .subscription_manager
      .send_event_to_subscribers(event)
      .await?;

    // The cache cannot hold a status, so all that is kept of one is what it says does
    // not exist.
    if let Ok(fetch_object) = object.clone().try_into_fetch() {
      self.cache.add_object(fetch_object).await;
    }
    self.note_absent(object, false);
    self.record_arrival(object).await;
    Ok(())
  }

  /// The Publisher Priority to assume for this track's objects whose header omits
  /// the field. A publisher may name it with the DEFAULT_PUBLISHER_PRIORITY track
  /// property; without one, the protocol's own default stands.
  pub async fn default_publisher_priority(&self) -> u8 {
    self
      .track_properties
      .read()
      .await
      .iter()
      .find_map(|p| match p {
        TrackProperty::DefaultPublisherPriority { priority } => Some(*priority),
        _ => None,
      })
      .unwrap_or(DEFAULT_PUBLISHER_PRIORITY)
  }

  pub async fn new_datagram(&self, datagram: &Datagram) -> Result<()> {
    self.datagram_arrivals.arrived();
    trace!(
      "new_datagram: relay_track_id={} group: {:?} object_id={} diff_ms={}",
      self.relay_track_id,
      datagram.group_id,
      datagram.object_id,
      utils::passed_time_since_start()
    );

    match Object::try_from_datagram(datagram.clone(), self.default_publisher_priority().await) {
      Ok((object, end_of_group)) => {
        if self.is_duplicate(&object.location) {
          trace!(
            "new_datagram: dropping duplicate | relay_track_id={} location: {:?}",
            self.relay_track_id, object.location
          );
          return Ok(());
        }

        if end_of_group {
          trace!(
            "new_datagram: end_of_group received for track: {:?} group: {:?} object_id: {}",
            datagram.track_alias, datagram.group_id, datagram.object_id
          );
        }

        // The cache cannot hold a status, so all that is kept of one is what it says
        // does not exist.
        if let Ok(fetch_object) = object.clone().try_into_fetch() {
          self.cache.add_object(fetch_object).await;
        }
        self.note_absent(&object, end_of_group);
        self.record_arrival(&object).await;
      }
      Err(e) => {
        error!(
          "Failed to convert datagram to object for logging: group: {:?} object_id={} error={}",
          datagram.group_id, datagram.object_id, e
        );
      }
    }

    let event = TrackEvent::Datagram {
      object: Arc::new(datagram.clone()),
    };

    self
      .subscription_manager
      .send_event_to_subscribers(event)
      .await?;

    Ok(())
  }

  pub async fn stream_closed(&self, stream_id: &StreamId) -> Result<()> {
    self.active_subgroup_headers.write().await.remove(stream_id);

    let event = TrackEvent::StreamClosed {
      stream_id: stream_id.clone(),
    };

    self
      .subscription_manager
      .send_event_to_subscribers(event)
      .await?;

    Ok(())
  }

  /// Send PublisherDisconnected event to all subscribers.
  /// Called internally by remove_publisher() when the last publisher leaves.
  pub async fn notify_publisher_disconnected(&self) -> Result<()> {
    info!(
      "All publishers gone for relay_track_id={} - notifying all subscribers",
      self.relay_track_id
    );

    self
      .notify_publish_done(
        PublishDoneStatusCode::TrackEnded,
        "Publisher disconnected".to_string(),
      )
      .await
  }

  /// Fan a PUBLISH_DONE out to all subscribers with the given status/reason.
  /// Used when an upstream publisher signals it is done for the track, so the
  /// upstream's status code is relayed downstream verbatim rather than being
  /// flattened to a generic disconnect.
  pub async fn notify_publish_done(
    &self,
    status_code: PublishDoneStatusCode,
    reason: String,
  ) -> Result<()> {
    let event = TrackEvent::PublisherDisconnected {
      status_code,
      reason,
    };

    self
      .subscription_manager
      .send_event_to_subscribers(event)
      .await?;

    Ok(())
  }
}

/// Holds a PUBLISH_DONE back until `stream_count` of that publisher's data streams
/// for the track have finished arriving, and, for a track that has been receiving
/// datagrams, until none has arrived for `datagram_quiet`; all within `timeout`. A
/// publisher that names more streams than it opens must not stall the subscription's
/// end, so a short count is logged and the caller carries on.
pub async fn await_publisher_streams(
  track: &Arc<Track>,
  publisher_connection_id: usize,
  stream_count: u64,
  timeout: Duration,
  datagram_quiet: Duration,
) {
  let deadline = Instant::now() + timeout;

  let (progress, datagrams, full_track_name) = {
    (
      track.publisher_stream_progress.clone(),
      track.datagram_arrivals.clone(),
      track.full_track_name.clone(),
    )
  };

  if stream_count > 0 {
    await_streams(
      &progress,
      &full_track_name,
      publisher_connection_id,
      stream_count,
      timeout,
    )
    .await;
  }
  datagrams.wait_for_quiet(datagram_quiet, deadline).await;
}

async fn await_streams(
  progress: &PublisherStreamProgress,
  full_track_name: &FullTrackName,
  publisher_connection_id: usize,
  stream_count: u64,
  timeout: Duration,
) {
  let closed = progress
    .wait_for(publisher_connection_id, stream_count, timeout)
    .await;

  if stream_count == PublishDone::UNKNOWN_STREAM_COUNT {
    debug!(
      "PUBLISH_DONE from publisher {} for {:?} gave no exact stream count; {} finished within {:?}",
      publisher_connection_id, full_track_name, closed, timeout
    );
  } else if closed < stream_count {
    warn!(
      "PUBLISH_DONE from publisher {} for {:?} named {} streams, {} finished within {:?}; ending the subscription anyway",
      publisher_connection_id, full_track_name, stream_count, closed, timeout
    );
  } else {
    debug!(
      "PUBLISH_DONE from publisher {} for {:?}: all {} streams finished",
      publisher_connection_id, full_track_name, stream_count
    );
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  const PUBLISHER: usize = 7;
  const OTHER_PUBLISHER: usize = 9;

  #[tokio::test]
  async fn a_track_without_datagrams_does_not_wait_for_them() {
    let arrivals = DatagramArrivals::default();
    let started = Instant::now();
    arrivals
      .wait_for_quiet(Duration::from_secs(5), started + Duration::from_secs(5))
      .await;
    assert!(started.elapsed() < Duration::from_millis(50));
  }

  #[tokio::test]
  async fn datagrams_are_waited_for_until_they_go_quiet() {
    let arrivals = Arc::new(DatagramArrivals::default());
    arrivals.arrived();
    let started = Instant::now();
    // One more arrives 40 ms in, which pushes the quiet window on.
    let late = arrivals.clone();
    tokio::spawn(async move {
      tokio::time::sleep(Duration::from_millis(40)).await;
      late.arrived();
    });
    arrivals
      .wait_for_quiet(Duration::from_millis(60), started + Duration::from_secs(5))
      .await;
    let waited = started.elapsed();
    assert!(waited >= Duration::from_millis(100), "waited {waited:?}");
    assert!(waited < Duration::from_secs(1), "waited {waited:?}");
  }

  #[tokio::test]
  async fn datagrams_that_never_go_quiet_are_waited_for_only_until_the_deadline() {
    let arrivals = Arc::new(DatagramArrivals::default());
    arrivals.arrived();
    let steady = arrivals.clone();
    let feeder = tokio::spawn(async move {
      loop {
        tokio::time::sleep(Duration::from_millis(10)).await;
        steady.arrived();
      }
    });
    let started = Instant::now();
    arrivals
      .wait_for_quiet(
        Duration::from_millis(50),
        started + Duration::from_millis(150),
      )
      .await;
    feeder.abort();
    let waited = started.elapsed();
    assert!(waited >= Duration::from_millis(150), "waited {waited:?}");
    assert!(waited < Duration::from_secs(1), "waited {waited:?}");
  }

  #[tokio::test]
  async fn wait_returns_at_once_when_the_streams_already_finished() {
    let progress = PublisherStreamProgress::default();
    for _ in 0..3 {
      progress.stream_closed(PUBLISHER).await;
    }

    let closed = progress
      .wait_for(PUBLISHER, 3, Duration::from_millis(50))
      .await;

    assert_eq!(closed, 3);
  }

  #[tokio::test]
  async fn wait_is_released_by_a_stream_finishing_later() {
    let progress = Arc::new(PublisherStreamProgress::default());
    let writer = progress.clone();
    tokio::spawn(async move {
      for _ in 0..2 {
        tokio::time::sleep(Duration::from_millis(10)).await;
        writer.stream_closed(PUBLISHER).await;
      }
    });

    let closed = progress
      .wait_for(PUBLISHER, 2, Duration::from_secs(5))
      .await;

    assert_eq!(closed, 2);
  }

  #[tokio::test]
  async fn wait_gives_up_and_reports_the_short_count() {
    let progress = PublisherStreamProgress::default();
    progress.stream_closed(PUBLISHER).await;

    let closed = progress
      .wait_for(PUBLISHER, 4, Duration::from_millis(20))
      .await;

    assert_eq!(closed, 1);
  }

  #[tokio::test]
  async fn each_publisher_is_counted_on_its_own() {
    let progress = PublisherStreamProgress::default();
    progress.stream_closed(PUBLISHER).await;
    progress.stream_closed(PUBLISHER).await;
    progress.stream_closed(OTHER_PUBLISHER).await;

    assert_eq!(progress.closed_count(PUBLISHER).await, 2);
    assert_eq!(progress.closed_count(OTHER_PUBLISHER).await, 1);
    // One publisher's streams must not release another's PUBLISH_DONE.
    assert_eq!(
      progress
        .wait_for(OTHER_PUBLISHER, 2, Duration::from_millis(20))
        .await,
      1
    );
  }

  #[tokio::test]
  async fn forgetting_a_publisher_clears_its_count() {
    let progress = PublisherStreamProgress::default();
    progress.stream_closed(PUBLISHER).await;
    progress.forget(PUBLISHER).await;

    assert_eq!(progress.closed_count(PUBLISHER).await, 0);
  }

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

  #[tokio::test]
  async fn a_subscriber_joining_a_pending_track_is_queued_and_a_later_one_is_not() {
    let track = cleanup::track(TrackOrigin::Subscribe);
    assert!(matches!(
      track.join_status(1, 10).await,
      TrackStatus::Pending
    ));
    track.confirm(7, 3, vec![], vec![]).await;
    assert!(matches!(
      track.join_status(2, 11).await,
      TrackStatus::Confirmed { .. }
    ));
    assert_eq!(*track.pending_subscribers.read().await, vec![(1, 10)]);
  }

  mod cleanup {
    use super::super::*;
    use super::PUBLISHER;
    use crate::server::config::Cli;
    use bytes::Bytes;
    use clap::Parser;
    use moqtail::model::data::constant::ObjectForwardingPreference;
    use moqtail::model::data::fetch_object::FetchObjectPayload;

    pub(super) fn track(origin: TrackOrigin) -> Track {
      let config = Box::leak(Box::new(AppConfig::from_cli(Cli::parse_from(["relay"]))));
      Track::new(
        1,
        FullTrackName::try_new("meet/room1", "video").unwrap(),
        config,
        TrackStatus::Pending,
        origin,
      )
    }

    async fn cache_an_object(track: &Track) {
      track
        .cache
        .add_object(FetchObjectPayload {
          group_id: 4,
          subgroup_id: 0,
          object_id: 0,
          publisher_priority: 0,
          forwarding_preference: ObjectForwardingPreference::Subgroup,
          properties: None,
          payload: Bytes::from_static(b"x"),
        })
        .await;
    }

    #[tokio::test]
    async fn letting_go_of_the_upstream_keeps_the_cache() {
      let track = track(TrackOrigin::Subscribe);
      track.publisher_aliases.write().await.insert(3, 11);
      cache_an_object(&track).await;

      assert_eq!(
        track.remove_subscriber(&(0, 0)).await,
        Cleanup::RemoveAliases(vec![(3, 11)])
      );
      assert_eq!(track.origin(), TrackOrigin::CacheOnly);
      assert!(track.publisher_aliases.read().await.is_empty());
      assert!(track.cache.get_group(4).await.is_some(), "the cache stays");

      // The next SUBSCRIBE takes it over and subscribes upstream afresh.
      assert!(
        track
          .take_over(TrackOrigin::Subscribe, TrackStatus::Pending)
          .await
      );
    }

    #[tokio::test]
    async fn the_last_subscriber_leaving_cancels_the_upstream_subscriptions() {
      for origin in [TrackOrigin::Subscribe, TrackOrigin::Publish] {
        let track = track(origin);
        let cancelled = track
          .register_upstream_request(1, PUBLISHER, UpstreamRequestKind::Subscribe)
          .await;
        track.remove_subscriber(&(0, 0)).await;
        assert_eq!(
          cancelled.await,
          Ok(UpstreamRequestCancellation::Cancelled),
          "{origin:?}"
        );
      }
    }

    #[tokio::test]
    async fn only_a_subscribe_created_track_lets_go_of_its_upstream() {
      for origin in [TrackOrigin::Publish, TrackOrigin::CacheOnly] {
        let track = track(origin);
        cache_an_object(&track).await;
        assert_eq!(track.remove_subscriber(&(0, 0)).await, Cleanup::Nothing);
        assert_eq!(track.origin(), origin);
      }
    }

    #[tokio::test]
    async fn a_track_holding_nothing_is_left_for_removal() {
      let track = track(TrackOrigin::Subscribe);
      assert_eq!(track.remove_subscriber(&(0, 0)).await, Cleanup::RemoveTrack);
      assert_eq!(track.retire().await, Cleanup::RemoveTrack);
      assert_eq!(track.origin(), TrackOrigin::Subscribe, "left as it was");
    }

    #[tokio::test]
    async fn a_track_whose_publisher_left_stays_for_what_it_cached() {
      let track = track(TrackOrigin::Publish);
      track.upstream_forward.store(true, Ordering::Relaxed);
      cache_an_object(&track).await;

      assert_eq!(track.retire().await, Cleanup::RemoveAliases(vec![]));
      assert_eq!(track.origin(), TrackOrigin::CacheOnly);
      assert!(
        !track.upstream_forward.load(Ordering::Relaxed),
        "a new publisher starts from Forward State 0 again"
      );
      assert!(
        track
          .take_over(
            TrackOrigin::Publish,
            TrackStatus::Confirmed {
              upstream_parameters: vec![]
            }
          )
          .await
      );
    }

    #[tokio::test]
    async fn answers_after_retirement_do_not_wrap_the_pending_count() {
      let track = track(TrackOrigin::Subscribe);
      track
        .pending_upstream_subscribe_count
        .store(2, Ordering::SeqCst);
      assert_eq!(track.upstream_subscribe_answered(), 1);
      cache_an_object(&track).await;
      track.retire().await;
      assert_eq!(track.upstream_subscribe_answered(), 0);
      assert_eq!(
        track
          .pending_upstream_subscribe_count
          .load(Ordering::SeqCst),
        0
      );
    }
  }

  mod live_evidence {
    use super::super::*;
    use bytes::Bytes;
    use moqtail::model::data::constant::ObjectForwardingPreference;

    fn track() -> Track {
      super::cleanup::track(TrackOrigin::Subscribe)
    }

    fn object(
      group: u64,
      id: u64,
      status: ObjectStatus,
      properties: Vec<ObjectProperty>,
    ) -> Object {
      Object {
        track_alias: 1,
        location: Location::new(group, id),
        publisher_priority: 128,
        forwarding_preference: ObjectForwardingPreference::Subgroup,
        subgroup_id: Some(0),
        status,
        properties: Some(properties),
        payload: (status == ObjectStatus::Normal).then(|| Bytes::from_static(b"x")),
      }
    }

    /// Caches a live Object and records what it says, as new_subgroup_object does.
    async fn arrive(track: &Track, object: &Object, ends_its_group: bool) {
      if let Ok(payload) = object.clone().try_into_fetch() {
        track.cache.add_object(payload).await;
      }
      track.note_absent(object, ends_its_group);
    }

    async fn complete(track: &Track, group: u64) -> bool {
      track
        .cache
        .objects_if_complete(group, 0, u64::MAX)
        .await
        .is_some()
    }

    #[tokio::test]
    async fn a_group_with_every_object_and_an_end_of_group_is_complete() {
      let track = track();
      for id in 0..3 {
        arrive(&track, &object(5, id, ObjectStatus::Normal, vec![]), false).await;
      }
      assert!(
        !complete(&track, 5).await,
        "where the group ends is not known yet"
      );
      arrive(
        &track,
        &object(5, 3, ObjectStatus::EndOfGroup, vec![]),
        false,
      )
      .await;
      assert!(complete(&track, 5).await);
    }

    #[tokio::test]
    async fn a_group_missing_an_object_is_not_complete() {
      let track = track();
      for id in [0, 2] {
        arrive(&track, &object(5, id, ObjectStatus::Normal, vec![]), false).await;
      }
      arrive(
        &track,
        &object(5, 3, ObjectStatus::EndOfGroup, vec![]),
        false,
      )
      .await;
      assert!(!complete(&track, 5).await, "object 1 never arrived");
    }

    #[tokio::test]
    async fn an_object_that_ends_its_group_ends_it() {
      let track = track();
      arrive(&track, &object(5, 0, ObjectStatus::Normal, vec![]), false).await;
      arrive(&track, &object(5, 1, ObjectStatus::Normal, vec![]), true).await;
      assert!(complete(&track, 5).await);
    }

    #[tokio::test]
    async fn a_stream_holding_the_end_of_its_group_ends_it() {
      let track = track();
      arrive(&track, &object(5, 0, ObjectStatus::Normal, vec![]), false).await;
      track.group_ended_after(&Location::new(5, 0));
      assert!(complete(&track, 5).await);
    }

    #[tokio::test]
    async fn an_end_of_track_rules_out_everything_after_it() {
      let track = track();
      arrive(&track, &object(5, 0, ObjectStatus::Normal, vec![]), false).await;
      arrive(
        &track,
        &object(5, 1, ObjectStatus::EndOfTrack, vec![]),
        false,
      )
      .await;
      assert!(complete(&track, 5).await);
      assert!(
        complete(&track, 9).await,
        "no group after the end of the track exists"
      );
    }

    #[tokio::test]
    async fn gap_properties_account_for_the_ids_they_skip() {
      let track = track();
      arrive(&track, &object(6, 0, ObjectStatus::Normal, vec![]), false).await;
      // Object 3 says 1 and 2 do not exist; the first object of group 6 says
      // groups 4 and 5 do not.
      let after_gap = object(
        6,
        3,
        ObjectStatus::Normal,
        vec![ObjectProperty::PriorObjectIdGap { gap: 2 }],
      );
      arrive(&track, &after_gap, true).await;
      assert!(complete(&track, 6).await);

      let first = object(
        6,
        0,
        ObjectStatus::Normal,
        vec![ObjectProperty::PriorGroupIdGap { gap: 2 }],
      );
      arrive(&track, &first, false).await;
      assert!(complete(&track, 4).await);
      assert!(complete(&track, 5).await);
      assert!(!complete(&track, 3).await);
    }
  }
}
