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

//! SSTS switching sets.
//!
//! A switching set is a collection of tracks representing the same content
//! at different throughput levels; the ABR selects exactly one track per
//! active set to forward. `throughput threshold` is a per-member property,
//! while `weight`, `activate` and `rank` are set properties: when several
//! subscriptions in the same set specify different values, the most recently
//! received message wins.

use crate::server::stream_id::StreamId;
use moqtail::model::data::full_track_name::FullTrackName;
use moqtail_ssts::{SetSnapshot, StreamSnapshot};
use std::cmp::Reverse;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// The one place mechanism state becomes algorithm state: everything, and only
/// everything, an algorithm is allowed to know about a set.
impl From<&SwitchingSet> for SetSnapshot {
  fn from(set: &SwitchingSet) -> Self {
    Self {
      id: set.id,
      algorithm_id: set.algorithm_id,
      rank: set.rank,
      weight: set.weight,
      active: set.is_active(),
      members: set
        .members
        .iter()
        .map(|m| (m.throughput_threshold_kbps, m.relay_track_id))
        .collect(),
      subscriber_buffer_ms: set.subscriber_buffer_ms,
      algorithm_params: set.algorithm_params.clone(),
      // Measured against the stream history rather than stored on the set, so
      // the manager fills this in when it builds a snapshot.
      group_span_ms: 0,
    }
  }
}

#[derive(Debug, Clone)]
pub enum SwitchingSetError {
  /// The track is already assigned to a different switching set; the
  /// subscription MUST be rejected with a Parameter Error.
  TrackInDifferentSet,
  /// The track does not belong to any switching set.
  TrackNotInSet,
}

impl fmt::Display for SwitchingSetError {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    match self {
      Self::TrackInDifferentSet => {
        write!(f, "track is already assigned to a different switching set")
      }
      Self::TrackNotInSet => write!(f, "track is not in any switching set"),
    }
  }
}

/// A track's membership in a switching set.
#[derive(Debug, Clone)]
pub struct SwitchingSetMember {
  pub full_track_name: FullTrackName,
  pub relay_track_id: u64,
  /// Minimum throughput (kbps) required to select this track. Property of
  /// the subscription.
  pub throughput_threshold_kbps: u64,
  /// The request that established this track's subscription (SUBSCRIBE request
  /// id, or the PUBLISH request id for pushed tracks). Kept so a later
  /// SWITCHING_SET_ASSIGNMENT that contradicts it can name who set the value.
  pub request_id: u64,
  /// Every subscription that assigned this track (request ids).
  pub holders: BTreeSet<u64>,
}

/// A switching set: tracks representing the same content at different
/// throughput levels.
#[derive(Debug, Clone)]
pub struct SwitchingSet {
  pub id: u64,
  pub algorithm_id: u64,
  /// Sorted by throughput threshold, ascending (index 0 = lowest quality).
  pub members: Vec<SwitchingSetMember>,
  /// Relative weight for bandwidth allocation among sets that share the same
  /// rank; 1..=10.
  pub weight: u64,
  /// 0 pauses SSTS for this set; switching activates once the number of
  /// assigned tracks is >= this value.
  pub activate: u64,
  /// Degradation priority; lower values are protected first.
  pub rank: u8,
  /// The subscriber's reorder buffer in ms, from the assignment's extension;
  /// 0 means the subscriber offered no latency budget and the relay must not
  /// discard anything.
  pub subscriber_buffer_ms: u64,
  /// The non-default algorithm's own negotiated knobs, carried verbatim from
  /// the assignment's extension. The mechanism hands these to the algorithm
  /// and never interprets them.
  pub algorithm_params: Vec<u64>,
}

impl SwitchingSet {
  /// Whether the allocation runs for this set: enabled and enough tracks.
  pub fn is_active(&self) -> bool {
    self.activate > 0 && self.members.len() as u64 >= self.activate
  }

  /// How long a forwarding stream of this set may linger after its group
  /// ended before the relay resets it, in ms; 0 closes every stream
  /// gracefully.
  ///
  /// The default algorithm gets `None`, because the draft fixes its
  /// serialization without this extension and gives it no discard behaviour.
  /// For every other algorithm the negotiated subscriber buffer is the whole
  /// answer, and `Some(0)` is an answer: the subscriber that sent zero asked
  /// for no delivery timeout at all, so the relay must not substitute its own
  /// configured timeout.
  pub fn negotiated_discard_timeout_ms(&self) -> Option<u64> {
    if self.algorithm_id != 0 {
      Some(self.subscriber_buffer_ms)
    } else {
      None
    }
  }
}

#[derive(Debug, Default)]
pub struct SwitchingSetManager {
  pub sets: HashMap<u64, SwitchingSet>,
  /// Track -> switching set id. A track MUST only be in one set at a time.
  pub track_to_set: HashMap<FullTrackName, u64>,
  /// Relay track id -> switching set id, so a forwarding stream can be
  /// attributed to the set it carries.
  pub relay_track_to_set: HashMap<u64, u64>,
  /// The live forwarding streams, keyed by stream, and owned here next to the
  /// sets because they only mean something while the set exists: a set that is
  /// torn down takes its streams with it, so a publisher that later reuses the
  /// id does not inherit a queue depth it never created.
  ///
  /// Streams rather than a count per set, because the questions an algorithm
  /// asks are about streams: how long the oldest one has been waiting, which
  /// groups are in flight, which quality each is carrying.
  open_streams: HashMap<StreamId, OpenStream>,
  /// Observed group cadence per set, so an algorithm can tell a stream that is
  /// merely in flight from one that is behind.
  group_spans: HashMap<u64, GroupSpan>,
  /// Bumped on every change to set membership or set properties. The forward
  /// gate caches its verdict against this counter, which it reads without
  /// taking this lock, so a change here invalidates what the gate cached.
  epoch: Arc<AtomicU64>,
  /// The groups whose delivery an algorithm overtook: the new streams opened for
  /// them re-send media that was already partly delivered at another quality,
  /// so they are marked and an algorithm that measures congestion can leave them
  /// out. The value is the deadline the decision asked for on the re-send, or
  /// `None` when the request said nothing about deadlines and the set's
  /// negotiated buffer stays in charge. Pruned with the group's last stream.
  resent_groups: HashMap<(u64, u64), Option<u64>>,
}

/// One live forwarding stream of a switching set.
#[derive(Debug, Clone)]
pub struct OpenStream {
  pub set_id: u64,
  pub relay_track_id: u64,
  pub group_id: u64,
  /// `now_ms` as the host counted it when the stream opened.
  pub opened_at_ms: u64,
  /// Opened to re-send a group an earlier decision overtook, rather than to
  /// carry new media.
  pub resent: bool,
}

/// The cadence a set's groups are arriving at, in ms.
///
/// A group's forwarding stream lives about one group span under normal
/// pacing, so an age of one span is a healthy pipeline and only the excess
/// over it is a backlog. Without this, a long group period looks like
/// congestion to an algorithm that reads stream ages.
#[derive(Debug, Default, Clone)]
struct GroupSpan {
  last_group: u64,
  last_open_ms: u64,
  span_ms: f64,
  /// Whether `span_ms` holds a measurement; the first gap has nothing to be
  /// averaged with, and guessing would report a cadence the host never saw.
  measured: bool,
}

/// Weight of the newest gap in the cadence average. Small enough that a few
/// irregular groups do not move the answer, large enough that a real change of
/// group period is followed within a couple of groups.
const SPAN_SMOOTHING: f64 = 0.3;

impl SwitchingSetManager {
  pub fn new() -> Self {
    Self::default()
  }

  /// The full track name behind a relay track id, when it belongs to a set.
  pub fn full_track_name_of_relay_track(&self, relay_track_id: u64) -> Option<FullTrackName> {
    self
      .set_of_relay_track(relay_track_id)?
      .members
      .iter()
      .find(|member| member.relay_track_id == relay_track_id)
      .map(|member| member.full_track_name.clone())
  }

  /// The set carrying this relay track, if any.
  pub fn set_of_relay_track(&self, relay_track_id: u64) -> Option<&SwitchingSet> {
    self
      .relay_track_to_set
      .get(&relay_track_id)
      .and_then(|set_id| self.sets.get(set_id))
  }

  /// A handle on the membership counter, for the gate to read without taking
  /// this lock. Held by `SstsState` alongside the manager itself.
  pub fn epoch_handle(&self) -> Arc<AtomicU64> {
    self.epoch.clone()
  }

  /// Invalidate whatever the forward gate cached about this connection's sets.
  /// Called by every mutation that can change which set a track belongs to, or
  /// whether its set is active.
  fn bump_epoch(&self) {
    self.epoch.fetch_add(1, Ordering::Release);
  }

  /// A forwarding stream opened for the set this relay track belongs to, at
  /// `now_ms` on the host's monotonic clock. A track in no set is not tracked:
  /// there is no switching decision for it to be evidence about.
  pub fn note_stream_opened(&mut self, stream_id: &StreamId, now_ms: u64) {
    let Some(&set_id) = self.relay_track_to_set.get(&stream_id.relay_track_id) else {
      return;
    };
    let Some(group_id) = stream_id.group_id else {
      return;
    };
    let resent = self.resent_groups.contains_key(&(set_id, group_id));
    self.open_streams.insert(
      stream_id.clone(),
      OpenStream {
        set_id,
        relay_track_id: stream_id.relay_track_id,
        group_id,
        opened_at_ms: now_ms,
        resent,
      },
    );

    let span = self.group_spans.entry(set_id).or_default();
    // Only consecutive groups bracket one group period. A gap in the group
    // numbers, or the first group of a set, says nothing about cadence.
    if span.measured && span.last_group + 1 == group_id {
      let gap = now_ms.saturating_sub(span.last_open_ms) as f64;
      // The first gap is the seed rather than an average with zero, which
      // would report a cadence a third of the one observed.
      span.span_ms = if span.span_ms > 0.0 {
        (1.0 - SPAN_SMOOTHING) * span.span_ms + SPAN_SMOOTHING * gap
      } else {
        gap
      };
    }
    span.last_group = group_id;
    span.last_open_ms = now_ms;
    span.measured = true;
  }

  /// A forwarding stream ended, by finishing, being reset or being stopped by
  /// the peer.
  pub fn note_stream_closed(&mut self, stream_id: &StreamId) {
    let Some(live) = self.open_streams.remove(stream_id) else {
      return;
    };
    // The mark is wiped by the close of its last re-send stream, and only then.
    // An overtaken delivery closing must not wipe it: the re-send's own stream
    // is usually not open yet at that moment - the decision resets the old
    // delivery and the new one opens a tick later - and a group with no live
    // stream at that instant is not "re-send over", it is "re-send not
    // started". Wiping then would leave the re-send's stream opening without
    // the mark: no head replay for the subscriber, and the algorithm reading a
    // fresh delivery where one was ordered.
    if live.resent
      && !self
        .open_streams
        .values()
        .any(|other| other.set_id == live.set_id && other.group_id == live.group_id && other.resent)
    {
      self.resent_groups.remove(&(live.set_id, live.group_id));
    }
  }

  /// Say that this group's delivery was overtaken by a decision, so the streams
  /// opened for it from now on re-send rather than deliver.
  pub fn mark_group_resent(&mut self, set_id: u64, group_id: u64, deadline_ms: Option<u64>) {
    self.resent_groups.insert((set_id, group_id), deadline_ms);
  }

  /// Whether this group's delivery was overtaken, so the streams opened for it
  /// now re-send media rather than deliver new media.
  pub fn is_resent_group(&self, set_id: u64, group_id: u64) -> bool {
    self.resent_groups.contains_key(&(set_id, group_id))
  }

  /// The deadline the decision asked for on this re-sent group's streams, or
  /// `None` when the group is not a re-send or the request left the deadline
  /// alone.
  pub fn resent_deadline_ms(&self, set_id: u64, group_id: u64) -> Option<u64> {
    self
      .resent_groups
      .get(&(set_id, group_id))
      .copied()
      .flatten()
  }

  /// The live streams carrying one group of one set on a member other than
  /// `keep`, which is what an overtaken delivery is made of.
  /// The member one rung below `track_id` in `set_id`'s ladder — the rung an
  /// age-staged re-send gives to the groups older than the newest one it
  /// overtook. One rung below the ladder's bottom is the bottom itself, which
  /// is the floor staging can fall to.
  pub fn member_one_rung_below(&self, set_id: u64, track_id: u64) -> Option<u64> {
    let set = self.sets.get(&set_id)?;
    let index = set
      .members
      .iter()
      .position(|member| member.relay_track_id == track_id)?;
    set
      .members
      .get(index.saturating_sub(1))
      .map(|member| member.relay_track_id)
  }

  pub fn streams_above(&self, set_id: u64, keep_track: u64) -> Vec<(u64, StreamId)> {
    let ladder = match self.sets.get(&set_id) {
      Some(set) => set,
      None => return Vec::new(),
    };
    let keep_at = match ladder
      .members
      .iter()
      .position(|m| m.relay_track_id == keep_track)
    {
      Some(at) => at,
      None => return Vec::new(),
    };
    let rung_of = |track: u64| {
      ladder
        .members
        .iter()
        .position(|m| m.relay_track_id == track)
    };

    let mut overtaken: Vec<(u64, StreamId)> = self
      .open_streams
      .iter()
      .filter(|(_, live)| {
        live.set_id == set_id && rung_of(live.relay_track_id).is_some_and(|at| at > keep_at)
      })
      .map(|(stream_id, live)| (live.group_id, stream_id.clone()))
      .collect();
    overtaken.sort_by_key(|(group_id, _)| *group_id);
    overtaken
  }

  /// The live streams of every set that still exists, as the algorithms see
  /// them, with ages measured against `now_ms`.
  pub fn stream_table(&self, now_ms: u64) -> Vec<StreamSnapshot> {
    let mut table: Vec<StreamSnapshot> = self
      .open_streams
      .values()
      .filter(|stream| self.sets.contains_key(&stream.set_id))
      .map(|stream| StreamSnapshot {
        set_id: stream.set_id,
        relay_track_id: stream.relay_track_id,
        group_id: stream.group_id,
        age_ms: now_ms.saturating_sub(stream.opened_at_ms),
        resent: stream.resent,
      })
      .collect();
    // Oldest first: the table is short, and a stable order makes a logged
    // decision reproducible.
    table.sort_by_key(|stream| (stream.set_id, Reverse(stream.age_ms)));
    table
  }

  /// The observed group cadence of a set in ms, 0 until two consecutive
  /// groups have been seen.
  pub fn group_span_ms(&self, set_id: u64) -> u64 {
    self
      .group_spans
      .get(&set_id)
      .filter(|span| span.measured)
      .map(|span| span.span_ms as u64)
      .unwrap_or(0)
  }

  #[allow(clippy::too_many_arguments)]
  pub fn assign(
    &mut self,
    full_track_name: FullTrackName,
    relay_track_id: u64,
    request_id: u64,
    switching_set_id: u64,
    algorithm_id: u64,
    throughput_threshold_kbps: u64,
    weight: u64,
    activate: u64,
    rank: u8,
    subscriber_buffer_ms: u64,
    algorithm_params: Vec<u64>,
  ) -> Result<(), SwitchingSetError> {
    if let Some(&existing_set_id) = self.track_to_set.get(&full_track_name)
      && existing_set_id != switching_set_id
    {
      return Err(SwitchingSetError::TrackInDifferentSet);
    }

    let mut member = SwitchingSetMember {
      full_track_name: full_track_name.clone(),
      relay_track_id,
      throughput_threshold_kbps,
      request_id,
      holders: BTreeSet::from([request_id]),
    };

    let set = self
      .sets
      .entry(switching_set_id)
      .or_insert_with(|| SwitchingSet {
        id: switching_set_id,
        algorithm_id,
        members: Vec::new(),
        weight,
        activate,
        rank,
        subscriber_buffer_ms,
        algorithm_params: algorithm_params.clone(),
      });

    // Set properties: the most recently received message wins.
    set.algorithm_id = algorithm_id;
    set.weight = weight;
    set.activate = activate;
    set.rank = rank;
    set.subscriber_buffer_ms = subscriber_buffer_ms;
    set.algorithm_params = algorithm_params;

    // Replace or insert the member.
    let mut previous_relay_track_id = None;
    if let Some(existing) = set
      .members
      .iter_mut()
      .find(|m| m.full_track_name == full_track_name)
    {
      previous_relay_track_id = Some(existing.relay_track_id);
      member.holders.append(&mut existing.holders);
      *existing = member;
    } else {
      set.members.push(member);
    }

    set.members.sort_by_key(|m| m.throughput_threshold_kbps);

    self.track_to_set.insert(full_track_name, switching_set_id);
    // A member re-assigned under a new relay track id must not leave the old
    // one pointing at this set: streams on a track that is no longer a member
    // would keep counting towards this set's depth forever.
    if let Some(previous) = previous_relay_track_id
      && previous != relay_track_id
    {
      self.relay_track_to_set.remove(&previous);
    }
    self
      .relay_track_to_set
      .insert(relay_track_id, switching_set_id);
    self.bump_epoch();
    Ok(())
  }

  /// Release happens when a subscription that assigned this track has ended
  pub fn release(&mut self, full_track_name: &FullTrackName, request_id: u64) {
    let Some(set_id) = self.track_to_set.get(full_track_name) else {
      return;
    };
    let Some(member) = self.sets.get_mut(set_id).and_then(|set| {
      set
        .members
        .iter_mut()
        .find(|m| m.full_track_name == *full_track_name)
    }) else {
      return;
    };
    if member.holders.remove(&request_id) && member.holders.is_empty() {
      self.remove(full_track_name);
    }
  }

  /// Remove a track (its last subscription went, or PUBLISH_DONE) and delete the set once its
  /// last track leaves. Removing a subscription also decrements `activate`,
  /// as base SSTS requires; the floor is zero.
  pub fn remove(&mut self, full_track_name: &FullTrackName) {
    let Some(set_id) = self.track_to_set.remove(full_track_name) else {
      return;
    };
    // The track is out of `track_to_set` from here on, whatever follows, so
    // the gate's cached membership is already stale.
    self.bump_epoch();
    let Some(set) = self.sets.get_mut(&set_id) else {
      return;
    };
    let relay_track_id = set
      .members
      .iter()
      .find(|m| m.full_track_name == *full_track_name)
      .map(|m| m.relay_track_id);
    set
      .members
      .retain(|m| m.full_track_name != *full_track_name);
    if let Some(relay_track_id) = relay_track_id {
      self.relay_track_to_set.remove(&relay_track_id);
    }
    // Base SSTS: removing a subscription from a set decrements the activation
    // threshold, floor zero. The threshold is a count of tracks the subscriber
    // promised, and one of them just left; keeping the old count would
    // deactivate a set that still has every remaining member it expected.
    set.activate = set.activate.saturating_sub(1);
    if set.members.is_empty() {
      self.sets.remove(&set_id);
      // The set is gone, and with it the meaning of its streams and cadence.
      self.open_streams.retain(|_, live| live.set_id != set_id);
      self.group_spans.remove(&set_id);
      self.resent_groups.retain(|(set, _), _| *set != set_id);
    }
  }

  /// Apply a SWITCHING_SET_ASSIGNMENT that arrives as an update to an existing
  /// subscription (REQUEST_UPDATE).
  ///
  /// `weight`, `activate` and `rank` are set properties and the most recently
  /// received message wins. `throughput_threshold_kbps` belongs to the member,
  /// so it moves that one track and re-sorts the ladder. `algorithm_id` cannot
  /// be honoured: the sets already have decisions keyed to the algorithm that
  /// was agreed when they were created, and re-keying them mid-flight would
  /// leave those decisions referring to a different algorithm. It is reported
  /// rather than dropped in silence, which is what the returned warnings are
  /// for.
  #[allow(clippy::too_many_arguments)]
  pub fn update_assignment(
    &mut self,
    full_track_name: &FullTrackName,
    algorithm_id: Option<u64>,
    throughput_threshold_kbps: Option<u64>,
    weight: Option<u64>,
    activate: Option<u64>,
    rank: Option<u8>,
    subscriber_buffer_ms: Option<u64>,
    algorithm_params: Option<Vec<u64>>,
  ) -> Result<Vec<String>, SwitchingSetError> {
    let set_id = self
      .track_to_set
      .get(full_track_name)
      .copied()
      .ok_or(SwitchingSetError::TrackNotInSet)?;

    let Some(set) = self.sets.get_mut(&set_id) else {
      return Err(SwitchingSetError::TrackNotInSet);
    };

    let mut warnings = Vec::new();
    if let Some(algorithm_id) = algorithm_id
      && algorithm_id != set.algorithm_id
    {
      let established_by = set
        .members
        .iter()
        .find(|m| m.full_track_name == *full_track_name)
        .map(|m| m.request_id);
      warnings.push(format!(
        "switching set {} runs algorithm {}; the update asking for {algorithm_id} was ignored{}",
        set.id,
        set.algorithm_id,
        match established_by {
          Some(request_id) => format!(" (set by request {request_id})"),
          None => String::new(),
        }
      ));
    }
    if let Some(weight) = weight {
      set.weight = weight;
    }
    if let Some(activate) = activate {
      set.activate = activate;
    }
    if let Some(rank) = rank {
      set.rank = rank;
    }
    if let Some(buffer_ms) = subscriber_buffer_ms {
      set.subscriber_buffer_ms = buffer_ms;
    }
    if let Some(params) = algorithm_params {
      set.algorithm_params = params;
    }
    if let Some(threshold) = throughput_threshold_kbps
      && let Some(member) = set
        .members
        .iter_mut()
        .find(|m| m.full_track_name == *full_track_name)
      && member.throughput_threshold_kbps != threshold
    {
      member.throughput_threshold_kbps = threshold;
      set.members.sort_by_key(|m| m.throughput_threshold_kbps);
    }
    // `activate` can flip the set between dormant and active, so the gate's
    // cached verdict no longer follows from what it saw.
    self.bump_epoch();
    Ok(warnings)
  }

  pub fn get_set_for_track(&self, full_track_name: &FullTrackName) -> Option<&SwitchingSet> {
    self
      .track_to_set
      .get(full_track_name)
      .and_then(|id| self.sets.get(id))
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use moqtail::model::common::tuple::{Tuple, TupleField};

  fn make_track(ns: &str, name: &str) -> FullTrackName {
    FullTrackName::new(Tuple::from_utf8_path(ns), TupleField::from_utf8(name))
      .expect("create track name")
  }

  fn assign(
    manager: &mut SwitchingSetManager,
    track: &FullTrackName,
    relay_track_id: u64,
    set_id: u64,
    threshold: u64,
  ) {
    assign_with(
      manager,
      track,
      relay_track_id,
      set_id,
      0,
      threshold,
      0,
      vec![],
    );
  }

  #[allow(clippy::too_many_arguments)]
  fn assign_with(
    manager: &mut SwitchingSetManager,
    track: &FullTrackName,
    relay_track_id: u64,
    set_id: u64,
    algorithm_id: u64,
    threshold: u64,
    subscriber_buffer_ms: u64,
    algorithm_params: Vec<u64>,
  ) {
    manager
      .assign(
        track.clone(),
        relay_track_id,
        relay_track_id,
        set_id,
        algorithm_id,
        threshold,
        5,
        2,
        0,
        subscriber_buffer_ms,
        algorithm_params,
      )
      .unwrap();
  }

  /// The negotiated buffer is the subscriber's permission to discard its own
  /// media; nothing else may grant it. A set on the default algorithm and a
  /// track outside every set leave the relay's configured timeout in charge;
  /// a non-default set that negotiated zero explicitly disables it.
  #[test]
  fn only_a_non_default_set_can_authorize_discarding() {
    let private = 0xff01;
    let buffer = make_track("ns", "buf");
    let unbuffered = make_track("ns", "nobuf");
    let default_algo = make_track("ns", "default");
    let outside = make_track("ns", "outside");

    let mut manager = SwitchingSetManager::new();
    assign_with(&mut manager, &buffer, 11, 1, private, 100, 250, vec![3, 70]);
    assign_with(&mut manager, &unbuffered, 12, 2, private, 100, 0, vec![]);
    assign_with(&mut manager, &default_algo, 13, 3, 0, 100, 900, vec![]);

    assert_eq!(
      manager
        .set_of_relay_track(11)
        .unwrap()
        .negotiated_discard_timeout_ms(),
      Some(250)
    );
    assert_eq!(
      manager
        .set_of_relay_track(12)
        .unwrap()
        .negotiated_discard_timeout_ms(),
      Some(0),
      "a zero buffer means never discard, not 'fall back to the default'"
    );
    assert_eq!(
      manager
        .set_of_relay_track(13)
        .unwrap()
        .negotiated_discard_timeout_ms(),
      None,
      "algorithm 0 has no discard behaviour in the draft"
    );
    assert!(manager.set_of_relay_track(99).is_none());
    let _ = outside;
  }

  /// The snapshot must hand the algorithm the same knobs the subscriber sent,
  /// in order: the algorithm interprets them, and a reordering here would be
  /// silently wrong.
  #[test]
  fn the_snapshot_carries_the_negotiated_knobs_in_order() {
    let mut manager = SwitchingSetManager::new();
    let track = make_track("ns", "k");
    assign_with(&mut manager, &track, 21, 5, 0xff01, 100, 300, vec![1, 2, 3]);

    let set = manager.get_set_for_track(&track).unwrap();
    let snapshot = SetSnapshot::from(set);
    assert_eq!(snapshot.subscriber_buffer_ms, 300);
    assert_eq!(snapshot.algorithm_params, vec![1, 2, 3]);
  }

  #[test]
  fn test_assign_and_remove() {
    let mut manager = SwitchingSetManager::new();
    let track1 = make_track("ns", "1080p");
    let track2 = make_track("ns", "480p");

    assign(&mut manager, &track1, 10, 1, 3000);
    assign(&mut manager, &track2, 20, 1, 800);

    // Assigning the same track to a different set must fail.
    assert!(matches!(
      manager.assign(track1.clone(), 10, 1, 2, 0, 3000, 5, 2, 0, 0, vec![]),
      Err(SwitchingSetError::TrackInDifferentSet)
    ));

    manager.remove(&track1);
    assert!(manager.get_set_for_track(&track1).is_none());

    let set = manager.get_set_for_track(&track2).unwrap();
    assert_eq!(set.members.len(), 1);
    // Base SSTS: one promised track left, so the threshold follows it down and
    // the remaining member keeps the set active.
    assert_eq!(set.activate, 1);
    assert!(set.is_active());
  }

  #[test]
  fn test_remove_deletes_emptied_set() {
    let mut manager = SwitchingSetManager::new();
    let track = make_track("ns", "t");
    assign(&mut manager, &track, 1, 7, 100);

    manager.remove(&track);
    // The last member removed the set entirely.
    assert!(!manager.sets.contains_key(&7));
    assert!(manager.get_set_for_track(&track).is_none());
  }

  #[test]
  fn test_a_track_stays_while_another_subscription_holds_it() {
    let mut manager = SwitchingSetManager::new();
    let track = make_track("ns", "t");
    // Two subscriptions to the same track, both assigning it to set 7.
    manager
      .assign(track.clone(), 42, 2, 7, 0, 100, 5, 2, 0)
      .unwrap();
    manager
      .assign(track.clone(), 42, 4, 7, 0, 100, 5, 2, 0)
      .unwrap();

    manager.release(&track, 2);
    assert!(manager.get_set_for_track(&track).is_some());
    assert_eq!(manager.relay_track_to_set.get(&42), Some(&7));

    manager.release(&track, 4);
    assert!(manager.get_set_for_track(&track).is_none());
    assert!(!manager.sets.contains_key(&7));
  }

  #[test]
  fn test_a_subscription_that_never_assigned_the_track_releases_nothing() {
    let mut manager = SwitchingSetManager::new();
    let track = make_track("ns", "t");
    manager
      .assign(track.clone(), 42, 2, 7, 0, 100, 5, 2, 0)
      .unwrap();

    // A second subscription to the track, without an assignment, ends.
    manager.release(&track, 4);
    assert!(manager.get_set_for_track(&track).is_some());
  }

  #[test]
  fn test_is_active() {
    let mut manager = SwitchingSetManager::new();
    let track1 = make_track("ns", "a");
    let track2 = make_track("ns", "b");

    // activate = 2, only one track assigned: not active yet.
    manager
      .assign(track1.clone(), 1, 1, 1, 0, 100, 5, 2, 0, 0, vec![])
      .unwrap();
    assert!(!manager.get_set_for_track(&track1).unwrap().is_active());

    manager
      .assign(track2.clone(), 2, 2, 1, 0, 200, 5, 2, 0, 0, vec![])
      .unwrap();
    assert!(manager.get_set_for_track(&track2).unwrap().is_active());
  }

  #[test]
  fn test_update_assignment_last_write_wins() {
    let mut manager = SwitchingSetManager::new();
    let track = make_track("ns", "t");
    assign(&mut manager, &track, 1, 3, 100);

    let warnings = manager
      .update_assignment(&track, Some(0), None, Some(9), Some(0), Some(4), None, None)
      .unwrap();
    assert!(
      warnings.is_empty(),
      "an update of the set's own properties is not a warning: {warnings:?}"
    );
    let set = manager.get_set_for_track(&track).unwrap();
    assert_eq!(set.weight, 9);
    assert_eq!(set.activate, 0);
    assert_eq!(set.rank, 4);
    // Paused sets are never active.
    assert!(!set.is_active());

    // None fields keep the previous values.
    manager
      .update_assignment(&track, None, None, None, None, Some(1), None, None)
      .unwrap();
    let set = manager.get_set_for_track(&track).unwrap();
    assert_eq!(set.weight, 9);
    assert_eq!(set.activate, 0);
    assert_eq!(set.rank, 1);
  }

  /// A REQUEST_UPDATE carries the whole assignment, so the draft's own
  /// set-level fields move with it: the latency budget and the three
  /// algorithm parameters are the most recently received values, like every
  /// other set property.
  #[test]
  fn test_update_assignment_moves_the_latency_budget_and_algorithm_params() {
    let mut manager = SwitchingSetManager::new();
    let track = make_track("ns", "t");
    assign_with(&mut manager, &track, 1, 3, 0xff05, 100, 250, vec![25, 2, 0]);

    manager
      .update_assignment(
        &track,
        Some(0xff05),
        Some(100),
        Some(5),
        Some(2),
        Some(0),
        Some(900),
        Some(vec![15, 3, 6]),
      )
      .unwrap();

    let set = manager.get_set_for_track(&track).unwrap();
    assert_eq!(set.subscriber_buffer_ms, 900);
    assert_eq!(set.algorithm_params, vec![15, 3, 6]);
  }

  #[test]
  fn test_relay_track_to_set_mapping() {
    let mut manager = SwitchingSetManager::new();
    let track = make_track("ns", "t");
    assign(&mut manager, &track, 42, 7, 100);
    assert_eq!(manager.relay_track_to_set.get(&42), Some(&7));

    manager.remove(&track);
    assert_eq!(manager.relay_track_to_set.get(&42), None);
  }

  #[test]
  fn test_update_assignment_unknown_track() {
    let mut manager = SwitchingSetManager::new();
    let track = make_track("ns", "t");
    assert!(matches!(
      manager.update_assignment(&track, None, None, None, None, None, None, None),
      Err(SwitchingSetError::TrackNotInSet)
    ));
  }

  #[test]
  fn test_update_assignment_moves_the_member_in_the_ladder() {
    // The throughput threshold is a member property, so an update to it moves
    // that track and re-sorts the ladder the algorithms read.
    let mut manager = SwitchingSetManager::new();
    let low = make_track("ns", "low");
    let high = make_track("ns", "high");
    assign(&mut manager, &low, 1, 3, 500);
    assign(&mut manager, &high, 2, 3, 2000);
    assert_eq!(
      manager
        .get_set_for_track(&low)
        .unwrap()
        .members
        .iter()
        .map(|m| m.relay_track_id)
        .collect::<Vec<_>>(),
      vec![1, 2]
    );

    manager
      .update_assignment(&low, None, Some(3000), None, None, None, None, None)
      .unwrap();
    assert_eq!(
      manager
        .get_set_for_track(&low)
        .unwrap()
        .members
        .iter()
        .map(|m| m.relay_track_id)
        .collect::<Vec<_>>(),
      vec![2, 1],
      "the track that asked for more bandwidth is now the top of the ladder"
    );
  }

  #[test]
  fn test_update_assignment_warns_about_an_algorithm_change() {
    // Switching a live set to another algorithm would leave the decisions
    // already made for it pointing at the old one, so the request is ignored
    // and reported rather than silently dropped.
    let mut manager = SwitchingSetManager::new();
    let track = make_track("ns", "t");
    assign(&mut manager, &track, 1, 3, 500);

    let warnings = manager
      .update_assignment(&track, Some(9), None, None, None, None, None, None)
      .unwrap();
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("algorithm 0"));
    assert!(warnings[0].contains("asking for 9"));
    // The set keeps the algorithm it was built with.
    assert_eq!(manager.get_set_for_track(&track).unwrap().algorithm_id, 0);

    // Asking for the same algorithm is not a warning.
    assert!(
      manager
        .update_assignment(&track, Some(0), None, None, None, None, None, None)
        .unwrap()
        .is_empty()
    );
  }
  #[test]
  fn test_reassigned_track_does_not_leave_a_stale_mapping() {
    // A track re-subscribed under a new relay track id must stop counting
    // streams opened on the old one, or the set keeps a depth it no longer has.
    let mut manager = SwitchingSetManager::new();
    let track = make_track("ns", "t");
    assign(&mut manager, &track, 42, 7, 100);

    manager
      .assign(track.clone(), 43, 2, 7, 0, 100, 5, 2, 0, 0, vec![])
      .unwrap();
    assert_eq!(manager.relay_track_to_set.get(&43), Some(&7));
    assert_eq!(manager.relay_track_to_set.get(&42), None);
  }

  #[test]
  fn a_stream_is_tracked_from_open_to_close() {
    let mut manager = SwitchingSetManager::new();
    let track = make_track("ns", "t");
    assign(&mut manager, &track, 42, 7, 100);

    let first = StreamId::new_subgroup(42, 1, Some(0));
    let second = StreamId::new_subgroup(42, 2, Some(0));
    manager.note_stream_opened(&first, 1000);
    manager.note_stream_opened(&second, 1100);

    let table = manager.stream_table(1500);
    assert_eq!(table.len(), 2);
    // Oldest first, and the age is how long the stream has been open rather
    // than the moment it opened: the algorithms compare ages, and the host's
    // epoch must not leak into their arithmetic.
    assert_eq!(
      table
        .iter()
        .map(|s| (s.group_id, s.age_ms))
        .collect::<Vec<_>>(),
      vec![(1, 500), (2, 400)]
    );
    assert_eq!(table[0].set_id, 7);
    assert_eq!(table[0].relay_track_id, 42);

    manager.note_stream_closed(&first);
    assert_eq!(manager.stream_table(1600).len(), 1);
    // Closing twice is what a stream that finished and was then reset looks
    // like from two paths; it cannot invent a negative stream.
    manager.note_stream_closed(&first);
    assert_eq!(manager.stream_table(1600).len(), 1);

    manager.note_stream_closed(&second);
    assert!(manager.stream_table(1600).is_empty());
  }

  #[test]
  fn a_stream_of_a_track_in_no_set_is_not_tracked() {
    let mut manager = SwitchingSetManager::new();
    let stream = StreamId::new_subgroup(99, 1, Some(0));
    manager.note_stream_opened(&stream, 0);
    assert!(manager.stream_table(1000).is_empty());
  }

  #[test]
  fn removing_a_set_takes_its_streams_with_it() {
    // The streams of a set that no longer exists have no meaning, and a
    // publisher that reuses the id must not inherit them.
    let mut manager = SwitchingSetManager::new();
    let track = make_track("ns", "t");
    assign(&mut manager, &track, 42, 7, 100);
    let stream = StreamId::new_subgroup(42, 1, Some(0));
    manager.note_stream_opened(&stream, 0);
    assert_eq!(manager.stream_table(0).len(), 1);

    manager.remove(&track);
    assert!(manager.stream_table(0).is_empty());

    assign(&mut manager, &track, 42, 7, 100);
    assert!(
      manager.stream_table(0).is_empty(),
      "a reused set id starts with the depth it actually has"
    );
  }

  /// A group's forwarding stream lives about one group period under normal
  /// pacing, so an algorithm that reads stream ages needs the period to tell a
  /// backlog from a healthy pipeline.
  #[test]
  fn the_group_cadence_needs_two_consecutive_groups() {
    let mut manager = SwitchingSetManager::new();
    let track = make_track("ns", "t");
    assign(&mut manager, &track, 42, 7, 100);

    // One group: nothing to compare it with, so no cadence.
    manager.note_stream_opened(&StreamId::new_subgroup(42, 4, Some(0)), 0);
    assert_eq!(manager.group_span_ms(7), 0);

    // Consecutive groups 100 ms apart: the cadence is that gap.
    manager.note_stream_opened(&StreamId::new_subgroup(42, 5, Some(0)), 100);
    assert_eq!(manager.group_span_ms(7), 100);

    // A gap in the group numbers is not a group period: the publisher was
    // away, not pacing.
    manager.note_stream_opened(&StreamId::new_subgroup(42, 9, Some(0)), 5000);
    assert_eq!(
      manager.group_span_ms(7),
      100,
      "a skipped run of groups is ignored"
    );

    // The next consecutive group moves it, and only partway: the point of the
    // average is that one odd group does not redefine the cadence.
    manager.note_stream_opened(&StreamId::new_subgroup(42, 10, Some(0)), 5200);
    let span = manager.group_span_ms(7);
    assert!(
      span > 100 && span < 200,
      "expected a damped move, got {span}"
    );

    // A set nobody measured reads as zero rather than as a guess.
    assert_eq!(manager.group_span_ms(1234), 0);
  }

  #[test]
  fn test_every_membership_change_moves_the_epoch() {
    // The forward gate holds its verdict for a group until this counter moves,
    // so a mutation that does not move it would leave the gate forwarding on a
    // track that is no longer selected.
    let mut manager = SwitchingSetManager::new();
    let epoch = manager.epoch_handle();
    let read = || epoch.load(Ordering::Acquire);
    let track = make_track("ns", "t");

    let before = read();
    assign(&mut manager, &track, 42, 7, 100);
    assert!(read() > before, "assign must move the epoch");

    let before = read();
    manager
      .update_assignment(&track, None, None, None, Some(0), None, None, None)
      .expect("track is in a set");
    assert!(
      read() > before,
      "an update that pauses the set must move the epoch"
    );

    let before = read();
    manager.remove(&track);
    assert!(read() > before, "remove must move the epoch");

    // A removal that matches no track changes nothing, so it need not move it.
    let before = read();
    manager.remove(&make_track("ns", "absent"));
    assert_eq!(read(), before);
  }

  #[test]
  fn test_stream_accounting_does_not_move_the_epoch() {
    // Opening and closing forwarding streams happens per group per track and
    // changes neither membership nor activation, so it must not invalidate the
    // gate's cache.
    let mut manager = SwitchingSetManager::new();
    let epoch = manager.epoch_handle();
    let track = make_track("ns", "t");
    assign(&mut manager, &track, 42, 7, 100);

    let before = epoch.load(Ordering::Acquire);
    let stream = StreamId::new_subgroup(42, 1, Some(0));
    manager.note_stream_opened(&stream, 1000);
    manager.note_stream_closed(&stream);
    assert_eq!(epoch.load(Ordering::Acquire), before);
  }

  /// An overtaken delivery is only what the ladder says it is: the streams on a
  /// member *above* the one the decision landed on. Streams at or below the new
  /// rung carry media the decision did not reject, and resetting them would
  /// destroy a delivery the algorithm still stands behind.
  #[test]
  fn overtaken_streams_follow_the_ladder_not_the_track_id() {
    let mut manager = SwitchingSetManager::new();
    // Deliberately assigned out of ladder order, with track ids that run the
    // other way, so a comparison of ids could not pass this test.
    let high = make_track("ns", "high");
    let mid = make_track("ns", "mid");
    let low = make_track("ns", "low");
    assign(&mut manager, &high, 10, 7, 3000);
    assign(&mut manager, &mid, 20, 7, 1000);
    assign(&mut manager, &low, 30, 7, 300);

    let high_stream = StreamId::new_subgroup(10, 5, Some(0));
    let mid_stream = StreamId::new_subgroup(20, 6, Some(0));
    let low_stream = StreamId::new_subgroup(30, 7, Some(0));
    for stream in [&high_stream, &mid_stream, &low_stream] {
      manager.note_stream_opened(stream, 1000);
    }

    // Land on the middle rung: only the high-quality delivery was overtaken.
    let overtaken = manager.streams_above(7, 20);
    assert_eq!(
      overtaken
        .iter()
        .map(|(group, _)| *group)
        .collect::<Vec<_>>(),
      vec![5],
      "the high rung is above the new choice, the low rung is not"
    );

    // Land on the bottom rung: both deliveries above it were overtaken, oldest
    // group first, so the mechanism deals with them in media order.
    let overtaken = manager.streams_above(7, 30);
    assert_eq!(
      overtaken
        .iter()
        .map(|(group, _)| *group)
        .collect::<Vec<_>>(),
      vec![5, 6]
    );

    // A track the set does not contain is not a rung, and nothing is overtaken
    // on the strength of a ladder the set does not have.
    assert!(manager.streams_above(7, 999).is_empty());
    assert!(manager.streams_above(1234, 20).is_empty());
  }

  /// A second step down reaches the first step's repair: a group already
  /// marked as re-sent is still a delivery in flight, and a stream above the
  /// new choice is overtaken whoever opened it. The re-mark updates the
  /// deadline the second repair inherits.
  #[test]
  fn a_re_sent_group_can_be_overtaken_again() {
    let mut manager = SwitchingSetManager::new();
    let high = make_track("ns", "high");
    let mid = make_track("ns", "mid");
    let low = make_track("ns", "low");
    assign(&mut manager, &high, 10, 7, 3000);
    assign(&mut manager, &mid, 20, 7, 1000);
    assign(&mut manager, &low, 30, 7, 300);

    // Group 6 was overtaken once and is being re-sent at the middle rung.
    manager.mark_group_resent(7, 6, Some(400));
    let repair = StreamId::new_subgroup(20, 6, Some(0));
    manager.note_stream_opened(&repair, 2000);

    // Step down again, to the bottom rung: the repair is above it, so it is
    // overtaken a second time - the re-send mark does not shield the group.
    let overtaken = manager.streams_above(7, 30);
    assert_eq!(
      overtaken
        .iter()
        .map(|(group, stream)| (*group, stream.relay_track_id))
        .collect::<Vec<_>>(),
      vec![(6, 20)],
      "the re-sent delivery is overtaken like any other above the new rung"
    );

    // And the second supersede updates the inherited deadline.
    manager.mark_group_resent(7, 6, Some(200));
    assert_eq!(manager.resent_deadline_ms(7, 6), Some(200));
  }

  /// A re-send mark lives for as long as the group it describes is being
  /// re-sent, and no longer: it is a note about live media, not history.
  #[test]
  fn a_resent_group_is_marked_until_its_last_stream_closes() {
    let mut manager = SwitchingSetManager::new();
    let track = make_track("ns", "t");
    assign(&mut manager, &track, 42, 7, 100);

    let first = StreamId::new_subgroup(42, 3, Some(0));
    manager.note_stream_opened(&first, 1000);
    assert!(
      !manager.stream_table(1100)[0].resent,
      "an ordinary delivery is not a re-send"
    );
    manager.note_stream_closed(&first);

    manager.mark_group_resent(7, 3, None);
    let again = StreamId::new_subgroup(42, 3, Some(1));
    manager.note_stream_opened(&again, 2000);
    assert!(
      manager.stream_table(2100)[0].resent,
      "a stream opened for an overtaken group re-sends media"
    );

    manager.note_stream_closed(&again);
    let third = StreamId::new_subgroup(42, 4, Some(0));
    manager.note_stream_opened(&third, 3000);
    assert!(
      !manager.stream_table(3100)[0].resent,
      "the next group is new media, not a re-send"
    );
  }

  /// The close of an overtaken delivery must not clear a re-send mark whose
  /// re-send has not started yet: the decision resets the old delivery and the
  /// re-send's stream opens a tick later, and a group with no live stream at
  /// that instant is "re-send not started", not "re-send over". Clearing the
  /// mark there would leave the re-send opening unmarked: no head replay for
  /// the subscriber, and the algorithm reading a fresh delivery where one was
  /// ordered.
  #[test]
  fn closing_an_overtaken_delivery_does_not_clear_the_unstarted_re_send() {
    let mut manager = SwitchingSetManager::new();
    let low = make_track("ns", "low");
    let high = make_track("ns", "high");
    assign(&mut manager, &low, 1, 3, 500);
    assign(&mut manager, &high, 2, 3, 2000);

    // Group 5 is being delivered at the top rung when a decision overtakes it
    // for the lower one: the mark goes down, the old delivery is reset.
    let overtaken = StreamId::new_subgroup(2, 5, Some(0));
    manager.note_stream_opened(&overtaken, 1000);
    manager.mark_group_resent(3, 5, None);
    manager.note_stream_closed(&overtaken);
    assert!(
      manager.is_resent_group(3, 5),
      "the re-send has not started, so closing the overtaken delivery keeps the mark"
    );

    // The re-send opens on the lower rung and snapshots the mark...
    let resent = StreamId::new_subgroup(1, 5, Some(0));
    manager.note_stream_opened(&resent, 2000);
    assert!(
      manager.stream_table(2100)[0].resent,
      "the re-send's stream is reported as a re-send"
    );

    // ...and its close, as the last re-send stream, is what ends the mark.
    manager.note_stream_closed(&resent);
    assert!(
      !manager.is_resent_group(3, 5),
      "the finished re-send leaves no mark"
    );
  }

  /// The deadline travels with the mark, because it belongs to the group the
  /// decision overtook and not to the set: the same ladder's other groups keep
  /// the buffer their subscriber negotiated.
  #[test]
  fn a_resent_group_remembers_the_deadline_it_was_given() {
    let mut manager = SwitchingSetManager::new();
    let track = make_track("ns", "t");
    assign_with(&mut manager, &track, 42, 7, 0, 100, 800, vec![]);

    // Nothing was overtaken, so nothing was asked.
    assert_eq!(manager.resent_deadline_ms(7, 3), None);

    manager.mark_group_resent(7, 3, Some(400));
    assert_eq!(manager.resent_deadline_ms(7, 3), Some(400));
    assert_eq!(
      manager.resent_deadline_ms(7, 4),
      None,
      "the neighbouring group is not part of the re-send"
    );

    // A request that said nothing about deadlines leaves the negotiated buffer
    // in charge, which is a different thing from a deadline of zero.
    manager.mark_group_resent(7, 5, None);
    assert_eq!(manager.resent_deadline_ms(7, 5), None);

    // The mark is about live media, so it goes when the group's last stream
    // does: a publisher that later reuses the group id starts clean.
    let stream = StreamId::new_subgroup(42, 3, Some(0));
    manager.note_stream_opened(&stream, 1000);
    manager.note_stream_closed(&stream);
    assert_eq!(manager.resent_deadline_ms(7, 3), None);
  }

  /// The rung an age-staged re-send gives to the older groups: one below the
  /// member the decision selected, the ladder's bottom being the floor. A
  /// track that is not in the set has no rung at all.
  #[test]
  fn one_rung_below_walks_down_the_ladder_and_floors_at_the_bottom() {
    let mut manager = SwitchingSetManager::new();
    let low = make_track("ns", "low");
    let mid = make_track("ns", "mid");
    let high = make_track("ns", "high");
    assign(&mut manager, &low, 40, 7, 100);
    assign(&mut manager, &mid, 41, 7, 200);
    assign(&mut manager, &high, 42, 7, 300);

    assert_eq!(manager.member_one_rung_below(7, 42), Some(41));
    assert_eq!(manager.member_one_rung_below(7, 41), Some(40));
    assert_eq!(
      manager.member_one_rung_below(7, 40),
      Some(40),
      "one rung below the bottom is the bottom: the floor, not a hole"
    );
    assert_eq!(manager.member_one_rung_below(7, 99), None);
    assert_eq!(manager.member_one_rung_below(8, 42), None);
  }
}
