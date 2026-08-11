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

//! The parameters the relay puts in the messages it originates.
//!
//! A Message Parameter addresses the peer on one hop. A relay is an endpoint on two,
//! so it re-originates rather than forwards: every message it sends carries a list it
//! built, which may take a received value into account but never carries one across
//! unread. The functions here are that decision, in one place, so a new message type
//! has to state what it means rather than inherit whatever arrived.
//!
//! Passing a received list along is not merely untidy. An AUTHORIZATION_TOKEN alias is
//! scoped to the session it was registered on, and the two endpoints of a session have
//! separate alias spaces, so an alias copied to another hop names nothing there — and
//! two subscribers who each register alias 1, both legally, would collide on the
//! relay's upstream session.

use moqtail::model::common::location::Location;
use moqtail::model::control::constant::{FilterType, GroupOrder};
use moqtail::model::parameter::constant::MessageParameterType;
use moqtail::model::parameter::message_parameter::MessageParameter;

/// The ergonomic lookup lives on `Vec`, but everything here reads a borrowed list.
fn find(
  params: &[MessageParameter],
  param_type: MessageParameterType,
) -> Option<&MessageParameter> {
  params.iter().find(|p| p.type_value() == param_type as u64)
}

/// The relay's SUBSCRIBE to an upstream publisher.
///
/// Deliberately not derived from the subscriber's SUBSCRIBE. One upstream subscription
/// serves every current and future subscriber, so adopting the filter, priority or
/// order that one of them happened to ask for would let the first subscriber decide
/// what the rest can be served. Each of those is applied per subscriber on the way out
/// instead, where it belongs.
///
/// The filter is stated rather than omitted. An omitted filter means unfiltered, which
/// starts at `{0,0}` — a demand for the track's entire history, which is not what a
/// relay joining a live track wants and not what it would do with the result. Largest
/// Object starts it at the present; earlier Objects reach a subscriber through FETCH,
/// which has its own upstream path.
pub fn upstream_subscribe() -> Vec<MessageParameter> {
  vec![
    MessageParameter::new_forward(true),
    MessageParameter::new_group_order(GroupOrder::Ascending),
    MessageParameter::new_subscription_filter(FilterType::LatestObject, None, None),
  ]
}

/// The relay's SUBSCRIBE_OK to a subscriber, given what the upstream publisher said.
///
/// LARGEST_OBJECT is the only parameter that survives the hop, and only as an input:
/// the relay reports the larger of what upstream told it and what it has since seen,
/// and omits it when there is neither, because an absent parameter means no Objects
/// where `{0,0}` would claim one. EXPIRES is dropped rather than relayed — it states
/// when *the sender* will end the subscription, and the relay makes no such promise on
/// its own behalf.
pub fn downstream_subscribe_ok(
  upstream: &[MessageParameter],
  observed: Option<Location>,
) -> Vec<MessageParameter> {
  match largest_object(upstream, observed) {
    Some(largest) => vec![MessageParameter::new_largest_object(largest)],
    None => Vec::new(),
  }
}

/// The relay's PUBLISH to a subscriber, given the upstream PUBLISH being passed on and
/// the SUBSCRIBE_TRACKS that asked for it.
///
/// FORWARD comes from that SUBSCRIBE_TRACKS, not from the publisher: a subscriber that
/// asked for tracks without forwarding must be offered them the same way. Absent or 1
/// there means forwarding, so only an explicit 0 turns it off.
pub fn downstream_publish(
  upstream: &[MessageParameter],
  subscribe_tracks: &[MessageParameter],
  observed: Option<Location>,
) -> Vec<MessageParameter> {
  let forward = matches!(
    find(subscribe_tracks, MessageParameterType::Forward),
    None | Some(MessageParameter::Forward { forward: true })
  );

  let mut params = vec![MessageParameter::new_forward(forward)];
  if let Some(largest) = largest_object(upstream, observed) {
    params.push(MessageParameter::new_largest_object(largest));
  }
  params
}

/// The relay's FETCH to an upstream publisher, filling a gap it cannot serve itself.
///
/// The range is what the relay is missing and travels in the message body; nothing the
/// downstream FETCH carried as a parameter describes it.
pub fn upstream_fetch() -> Vec<MessageParameter> {
  vec![MessageParameter::new_group_order(GroupOrder::Ascending)]
}

/// The larger of what an upstream told the relay and what the relay has seen itself.
fn largest_object(upstream: &[MessageParameter], observed: Option<Location>) -> Option<Location> {
  let reported = match find(upstream, MessageParameterType::LargestObject) {
    Some(MessageParameter::LargestObject { location }) => Some(location.clone()),
    _ => None,
  };

  match (reported, observed) {
    (Some(u), Some(o)) => Some(if o > u { o } else { u }),
    (u, o) => u.or(o),
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use moqtail::model::parameter::authorization_token::AuthorizationToken;
  use moqtail::model::parameter::message_parameter::MessageParameterVecExt;

  fn loc(group: u64, object: u64) -> Location {
    Location::new(group, object)
  }

  fn token() -> MessageParameter {
    MessageParameter::new_authorization_token(AuthorizationToken::new_use_alias(1))
  }

  #[test]
  fn an_upstream_subscribe_starts_at_the_present_and_forwards() {
    let params = upstream_subscribe();
    assert!(matches!(
      params.get_param(MessageParameterType::Forward),
      Some(MessageParameter::Forward { forward: true })
    ));
    // Stated, not omitted: omitted would ask for the track from {0,0}.
    assert!(matches!(
      params.get_param(MessageParameterType::SubscriptionFilter),
      Some(MessageParameter::SubscriptionFilter {
        filter_type: FilterType::LatestObject,
        ..
      })
    ));
    assert!(
      params
        .get_param(MessageParameterType::AuthorizationToken)
        .is_none()
    );
  }

  #[test]
  fn largest_object_takes_the_larger_of_the_two() {
    let earlier = loc(4, 2);
    let later = loc(4, 9);
    let later_group = loc(5, 0);

    let reported = |l: &Location| vec![MessageParameter::new_largest_object(l.clone())];

    // Whichever side is ahead wins, regardless of which side it is on.
    assert_eq!(
      largest_object(&reported(&earlier), Some(later.clone())),
      Some(later.clone())
    );
    assert_eq!(
      largest_object(&reported(&later), Some(earlier.clone())),
      Some(later.clone())
    );
    // A later group beats a later object within an earlier group.
    assert_eq!(
      largest_object(&reported(&later), Some(later_group.clone())),
      Some(later_group)
    );

    // One side only.
    assert_eq!(largest_object(&reported(&later), None), Some(later.clone()));
    assert_eq!(largest_object(&[], Some(later.clone())), Some(later));

    // Neither: the parameter is omitted, never sent as {0,0}.
    assert_eq!(largest_object(&[], None), None);
  }

  #[test]
  fn a_subscribe_ok_reports_the_larger_of_the_two() {
    let upstream = vec![MessageParameter::new_largest_object(loc(4, 1))];
    let params = downstream_subscribe_ok(&upstream, Some(loc(7, 0)));
    assert_eq!(
      params.get_param(MessageParameterType::LargestObject),
      Some(&MessageParameter::new_largest_object(loc(7, 0)))
    );
  }

  #[test]
  fn a_subscribe_ok_keeps_the_upstream_value_when_it_is_ahead() {
    let upstream = vec![MessageParameter::new_largest_object(loc(9, 2))];
    let params = downstream_subscribe_ok(&upstream, Some(loc(3, 0)));
    assert_eq!(
      params.get_param(MessageParameterType::LargestObject),
      Some(&MessageParameter::new_largest_object(loc(9, 2)))
    );
  }

  #[test]
  fn a_subscribe_ok_with_no_objects_anywhere_carries_nothing() {
    assert!(downstream_subscribe_ok(&[], None).is_empty());
  }

  #[test]
  fn a_subscribe_ok_drops_what_the_upstream_addressed_to_us() {
    let upstream = vec![
      token(),
      MessageParameter::new_expires(30_000),
      MessageParameter::new_largest_object(loc(1, 0)),
    ];
    let params = downstream_subscribe_ok(&upstream, None);
    assert_eq!(params.len(), 1);
    assert!(
      params
        .get_param(MessageParameterType::AuthorizationToken)
        .is_none()
    );
    assert!(params.get_param(MessageParameterType::Expires).is_none());
  }

  #[test]
  fn a_pushed_publish_takes_forwarding_from_the_request_that_asked_for_it() {
    let upstream = vec![token(), MessageParameter::new_forward(true)];

    let off = downstream_publish(&upstream, &[MessageParameter::new_forward(false)], None);
    assert!(matches!(
      off.get_param(MessageParameterType::Forward),
      Some(MessageParameter::Forward { forward: false })
    ));

    // Omitted in SUBSCRIBE_TRACKS means forwarding, the same as an explicit 1.
    let on = downstream_publish(&upstream, &[], None);
    assert!(matches!(
      on.get_param(MessageParameterType::Forward),
      Some(MessageParameter::Forward { forward: true })
    ));

    assert!(
      off
        .get_param(MessageParameterType::AuthorizationToken)
        .is_none()
    );
  }

  #[test]
  fn a_pushed_publish_reports_the_track_position() {
    let upstream = vec![MessageParameter::new_largest_object(loc(2, 5))];
    let params = downstream_publish(&upstream, &[], Some(loc(2, 9)));
    assert_eq!(
      params.get_param(MessageParameterType::LargestObject),
      Some(&MessageParameter::new_largest_object(loc(2, 9)))
    );
  }
}
