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

//! Metrics carried as MoQ tracks, in the layout of "Metrics over MOQT".
//!
//! A source publishes under `("metrics.moq.arpa", "v1", <resource id>)`: a
//! `catalog` track describing each metric, and data tracks named by level `"0"` to
//! `"7"`. In a data track each group is one sample, its group id the capture time
//! in milliseconds: object 0 is the capture info, object `n` the value of metric
//! `n`, and an end-of-group object follows the last. This module encodes and
//! decodes the object payloads.

use crate::model::common::varint::{BufMutVarIntExt, BufVarIntExt};
use crate::model::error::ParseError;
use bytes::{Buf, BufMut, Bytes, BytesMut};

/// The first two fields of every metrics namespace.
pub const NAMESPACE_PREFIX: [&str; 2] = ["metrics.moq.arpa", "v1"];
pub const CATALOG_TRACK: &str = "catalog";
/// The catalog format this module writes.
pub const CATALOG_VERSION: u64 = 1;

/// How a metric's values behave over time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetricKind {
  /// A value at the moment of capture.
  Gauge = 0,
  /// A count that only grows, back to zero when the source restarts.
  Counter = 1,
}

/// How a metric's value is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueKind {
  /// A variable-length unsigned integer.
  Unsigned = 0,
  /// A 64-bit two's-complement integer, most significant byte first.
  Signed = 1,
  /// A 64-bit IEEE 754 float, most significant byte first.
  Float = 2,
}

/// One entry of the catalog; its position (from 1) is the metric's object id.
#[derive(Debug, Clone, PartialEq)]
pub struct MetricDescription {
  pub kind: MetricKind,
  pub value: ValueKind,
  /// ASCII letters, digits, `_` and `:`.
  pub name: String,
  /// An OpenTelemetry unit such as `s`, `By` or `{object}`; may be empty.
  pub unit: String,
}

/// A value to write for one metric.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MetricValue {
  Unsigned(u64),
  Signed(i64),
  Float(f64),
}

fn put_text(buf: &mut BytesMut, text: &str) -> Result<(), ParseError> {
  buf.put_vi(text.len() as u64)?;
  buf.put_slice(text.as_bytes());
  Ok(())
}

fn get_text(buf: &mut Bytes) -> Result<String, ParseError> {
  let len = buf.get_vi()? as usize;
  if buf.remaining() < len {
    return Err(ParseError::NotEnoughBytes {
      context: "metrics::get_text",
      needed: len,
      available: buf.remaining(),
    });
  }
  String::from_utf8(buf.split_to(len).to_vec()).map_err(|e| ParseError::Other {
    context: "metrics::get_text",
    msg: e.to_string(),
  })
}

/// The catalog's object 0: its version and the source's attributes, such as
/// `service.name`.
pub fn encode_catalog_header(attributes: &[(&str, &str)]) -> Result<Bytes, ParseError> {
  let mut buf = BytesMut::new();
  buf.put_vi(CATALOG_VERSION)?;
  buf.put_vi(attributes.len() as u64)?;
  for (key, value) in attributes {
    put_text(&mut buf, key)?;
    put_text(&mut buf, value)?;
  }
  Ok(buf.freeze())
}

pub fn decode_catalog_header(mut buf: Bytes) -> Result<(u64, Vec<(String, String)>), ParseError> {
  let version = buf.get_vi()?;
  let count = buf.get_vi()?;
  let mut attributes = Vec::new();
  for _ in 0..count {
    attributes.push((get_text(&mut buf)?, get_text(&mut buf)?));
  }
  Ok((version, attributes))
}

/// A catalog object describing one metric.
pub fn encode_description(metric: &MetricDescription) -> Result<Bytes, ParseError> {
  let mut buf = BytesMut::new();
  buf.put_vi(metric.kind as u64)?;
  buf.put_vi(metric.value as u64)?;
  put_text(&mut buf, &metric.name)?;
  put_text(&mut buf, &metric.unit)?;
  Ok(buf.freeze())
}

pub fn decode_description(mut buf: Bytes) -> Result<MetricDescription, ParseError> {
  let kind = match buf.get_vi()? {
    0 => MetricKind::Gauge,
    1 => MetricKind::Counter,
    other => return Err(invalid("metric type", other)),
  };
  let value = match buf.get_vi()? {
    0 => ValueKind::Unsigned,
    1 => ValueKind::Signed,
    2 => ValueKind::Float,
    other => return Err(invalid("value representation", other)),
  };
  // Fields added by later versions follow; they are ignored.
  Ok(MetricDescription {
    kind,
    value,
    name: get_text(&mut buf)?,
    unit: get_text(&mut buf)?,
  })
}

fn invalid(what: &'static str, value: u64) -> ParseError {
  ParseError::Other {
    context: "metrics",
    msg: format!("unknown {what} {value}"),
  }
}

/// A data group's object 0: nanoseconds to add to the group id (milliseconds) for
/// the exact capture time, and the catalog version the values follow.
pub fn encode_capture_info(offset_nanos: u64, catalog_version: u64) -> Result<Bytes, ParseError> {
  let mut buf = BytesMut::new();
  buf.put_vi(offset_nanos)?;
  buf.put_vi(catalog_version)?;
  Ok(buf.freeze())
}

pub fn decode_capture_info(mut buf: Bytes) -> Result<(u64, u64), ParseError> {
  Ok((buf.get_vi()?, buf.get_vi()?))
}

pub fn encode_value(value: MetricValue) -> Result<Bytes, ParseError> {
  let mut buf = BytesMut::new();
  match value {
    MetricValue::Unsigned(v) => buf.put_vi(v)?,
    MetricValue::Signed(v) => buf.put_i64(v),
    MetricValue::Float(v) => buf.put_f64(v),
  }
  Ok(buf.freeze())
}

pub fn decode_value(mut buf: Bytes, kind: ValueKind) -> Result<MetricValue, ParseError> {
  let short = || ParseError::NotEnoughBytes {
    context: "metrics::decode_value",
    needed: 8,
    available: 0,
  };
  Ok(match kind {
    ValueKind::Unsigned => MetricValue::Unsigned(buf.get_vi()?),
    ValueKind::Signed if buf.remaining() >= 8 => MetricValue::Signed(buf.get_i64()),
    ValueKind::Float if buf.remaining() >= 8 => MetricValue::Float(buf.get_f64()),
    _ => return Err(short()),
  })
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn catalog_round_trips() {
    let header = encode_catalog_header(&[("service.name", "moqtail-relay")]).unwrap();
    let (version, attributes) = decode_catalog_header(header).unwrap();
    assert_eq!(version, CATALOG_VERSION);
    assert_eq!(attributes[0].1, "moqtail-relay");

    let metric = MetricDescription {
      kind: MetricKind::Counter,
      value: ValueKind::Unsigned,
      name: "objects_received".into(),
      unit: "{object}".into(),
    };
    assert_eq!(
      decode_description(encode_description(&metric).unwrap()).unwrap(),
      metric
    );
  }

  #[test]
  fn values_round_trip() {
    for (value, kind) in [
      (MetricValue::Unsigned(300), ValueKind::Unsigned),
      (MetricValue::Signed(-5), ValueKind::Signed),
      (MetricValue::Float(0.25), ValueKind::Float),
    ] {
      assert_eq!(
        decode_value(encode_value(value).unwrap(), kind).unwrap(),
        value
      );
    }
    assert_eq!(
      decode_capture_info(encode_capture_info(500, 1).unwrap()).unwrap(),
      (500, 1)
    );
  }
}
