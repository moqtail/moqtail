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

//! The command-line forms of Range Filters and of the Properties a test publisher
//! puts on its tracks and Objects.
//!
//! A filter is only checked for what it takes to put it on the wire. Anything a
//! receiver has to refuse -- an odd Property Type, a priority above 255, the same
//! SetID twice -- goes out as given, so a relay's refusal can be tested.

use moqtail::model::common::pair::KeyValuePair;
use moqtail::model::parameter::combined_filter::{CombinedFilter, check_filter_range_limit};
use moqtail::model::parameter::message_parameter::MessageParameter;
use moqtail::model::parameter::range_filter::{Range, RangeFilter, RangeFilterKind};
use moqtail::model::property::track_property::TrackProperty;
use tracing::warn;

fn number(text: &str) -> Result<u64, String> {
  let text = text.trim();
  let parsed = match text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
    Some(hex) => u64::from_str_radix(hex, 16),
    None => text.parse(),
  };
  parsed.map_err(|_| format!("'{text}' is not a number"))
}

fn kind(text: &str) -> Result<RangeFilterKind, String> {
  match text {
    "subgroup" => Ok(RangeFilterKind::Subgroup),
    "object" | "object-id" | "objectid" => Ok(RangeFilterKind::ObjectId),
    "priority" => Ok(RangeFilterKind::Priority),
    "object-property" | "objprop" => Ok(RangeFilterKind::ObjectProperty),
    "track-property" | "trackprop" => Ok(RangeFilterKind::TrackProperty),
    other => Err(format!(
      "unknown filter kind '{other}' (subgroup, object, priority, object-property, track-property)"
    )),
  }
}

fn ranges(text: &str) -> Result<Vec<Range>, String> {
  text
    .split(',')
    .filter(|item| !item.trim().is_empty())
    .map(|item| match item.split_once('-') {
      Some((start, "")) => Ok(Range::open(number(start)?)),
      Some((start, end)) => Ok(Range::new(number(start)?, number(end)?)),
      None => {
        let value = number(item)?;
        Ok(Range::new(value, value))
      }
    })
    .collect()
}

/// Parses one `--range-filter`.
pub fn parse_range_filter(spec: &str) -> Result<MessageParameter, String> {
  let mut parts = spec.split(':');
  let kind = kind(parts.next().unwrap_or_default())?;
  let rest: Vec<&str> = parts.collect();
  if rest == ["clear"] {
    return Ok(MessageParameter::new_range_filter_removal(kind));
  }

  let (set, property_type, list) = match (kind.has_property_type(), rest.as_slice()) {
    (false, [set, list]) => (set, None, list),
    (true, [set, property, list]) => (set, Some(number(property)?), list),
    (false, _) => return Err(format!("expected {spec:?} to be KIND:SET:RANGES")),
    (true, _) => return Err(format!("expected {spec:?} to be KIND:SET:PROPERTY:RANGES")),
  };
  let set_id: u8 = number(set)?
    .try_into()
    .map_err(|_| format!("SetID {set} is not 0-255"))?;
  let filter = RangeFilter::new(set_id, property_type, &ranges(list)?)
    .map_err(|e| format!("{spec:?}: {e}"))?;
  Ok(MessageParameter::new_range_filter(kind, filter))
}

/// Refuses to send `params` past the server's MAX_FILTER_RANGES of `limit`, unless
/// `ignore_limit` asks for them to go anyway, to see the server refuse them.
pub fn check_limit(
  params: &[MessageParameter],
  limit: u64,
  ignore_limit: bool,
) -> anyhow::Result<()> {
  match check_filter_range_limit(params, limit) {
    Ok(()) => Ok(()),
    Err(e) if ignore_limit => {
      warn!("Sending range filters the server does not allow ({e}); expect it to refuse them");
      Ok(())
    }
    Err(e) => anyhow::bail!("{e}; pass --ignore-filter-limit to send them anyway"),
  }
}

/// The filter the objects of a request are checked against. One the server has to
/// refuse checks nothing: the request should not get as far as delivering.
pub fn object_filter(params: &[MessageParameter]) -> CombinedFilter {
  CombinedFilter::for_objects(params).unwrap_or_else(|e| {
    warn!("Requesting range filters the server has to refuse ({e})");
    CombinedFilter::default()
  })
}

/// Where a test publisher takes an Object Property's value from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PropertySource {
  Group,
  Object,
  Subgroup,
  /// The Object ID modulo this.
  ObjectModulo(u64),
  Constant(u64),
}

impl PropertySource {
  pub fn value(self, group: u64, object: u64, subgroup: u64) -> u64 {
    match self {
      Self::Group => group,
      Self::Object => object,
      Self::Subgroup => subgroup,
      Self::ObjectModulo(m) => object % m,
      Self::Constant(v) => v,
    }
  }
}

/// An Object Property a test publisher puts on every Object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObjectPropertySpec {
  pub property_type: u64,
  pub source: PropertySource,
}

/// Parses one `--object-property`.
pub fn parse_object_property(spec: &str) -> Result<ObjectPropertySpec, String> {
  let (property_type, source) = spec
    .split_once('=')
    .ok_or_else(|| format!("expected {spec:?} to be TYPE=SOURCE"))?;
  let property_type = even_type(property_type)?;
  let source = match source.trim() {
    "group" => PropertySource::Group,
    "object" => PropertySource::Object,
    "subgroup" => PropertySource::Subgroup,
    other => match other.strip_prefix("mod:") {
      Some(m) => match number(m)? {
        0 => return Err("mod:0 has no values".to_string()),
        m => PropertySource::ObjectModulo(m),
      },
      None => PropertySource::Constant(number(other)?),
    },
  };
  Ok(ObjectPropertySpec {
    property_type,
    source,
  })
}

/// Parses one `--track-property`.
pub fn parse_track_property(spec: &str) -> Result<TrackProperty, String> {
  let (property_type, value) = spec
    .split_once('=')
    .ok_or_else(|| format!("expected {spec:?} to be TYPE=VALUE"))?;
  let kvp = KeyValuePair::try_new_varint(even_type(property_type)?, number(value)?)
    .map_err(|e| format!("{spec:?}: {e:?}"))?;
  TrackProperty::deserialize(kvp).map_err(|e| format!("{spec:?}: {e:?}"))
}

/// Only an even type holds a single integer.
fn even_type(text: &str) -> Result<u64, String> {
  let property_type = number(text)?;
  if property_type % 2 != 0 {
    return Err(format!(
      "Property Type {property_type:#x} is odd; only even types hold an integer"
    ));
  }
  Ok(property_type)
}

#[cfg(test)]
mod tests {
  use super::*;

  fn ranges_of(param: &MessageParameter) -> Vec<Range> {
    match param {
      MessageParameter::RangeFilter {
        filter: Some(filter),
        ..
      } => filter.ranges().unwrap(),
      other => panic!("not a range filter: {other:?}"),
    }
  }

  #[test]
  fn parses_ranges_single_values_and_open_ends() {
    let param = parse_range_filter("object:0:0-2,4,7-").unwrap();
    assert_eq!(
      ranges_of(&param),
      vec![Range::new(0, 2), Range::new(4, 4), Range::open(7)]
    );
    assert_eq!(param.type_value(), 0x26);
  }

  #[test]
  fn a_property_kind_names_its_property() {
    let param = parse_range_filter("object-property:1:0x3c:10-20").unwrap();
    let MessageParameter::RangeFilter {
      kind,
      filter: Some(filter),
    } = param
    else {
      panic!()
    };
    assert_eq!(kind, RangeFilterKind::ObjectProperty);
    assert_eq!((filter.set_id, filter.property_type), (1, Some(0x3C)));
  }

  #[test]
  fn clear_is_the_zero_length_removal() {
    assert_eq!(
      parse_range_filter("priority:clear").unwrap(),
      MessageParameter::new_range_filter_removal(RangeFilterKind::Priority)
    );
  }

  #[test]
  fn what_a_receiver_must_refuse_still_parses() {
    // An odd Property Type and a priority above 255 are the receiver's to refuse.
    assert!(parse_range_filter("object-property:0:0x3d:1").is_ok());
    assert!(parse_range_filter("priority:0:0-300").is_ok());
  }

  #[test]
  fn what_cannot_be_encoded_is_refused() {
    for spec in [
      "object:0:5-3",
      "object:0:10-15,3-5",
      "object:0:3-,10-15",
      "object:256:0",
      "object-property:0:1-2",
      "subgroup:0:0x3c:1",
      "colour:0:1",
    ] {
      assert!(parse_range_filter(spec).is_err(), "{spec}");
    }
  }

  #[test]
  fn parses_object_property_sources() {
    let spec = parse_object_property("0x3c=mod:3").unwrap();
    assert_eq!(spec.source.value(4, 7, 1), 1);
    assert_eq!(
      parse_object_property("60=group").unwrap().source,
      PropertySource::Group
    );
    assert!(parse_object_property("0x3d=1").is_err());
    assert!(parse_object_property("0x3c=mod:0").is_err());
  }

  #[test]
  fn parses_track_properties() {
    assert!(parse_track_property("0x3c=1").is_ok());
    assert!(parse_track_property("0x3d=1").is_err());
  }
}
