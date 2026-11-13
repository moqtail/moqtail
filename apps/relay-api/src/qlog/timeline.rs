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

//! A connection's message timeline, built from its qlog events: each request with
//! the responses that came back on its stream, parse errors, and data streams
//! counted per track. It follows the dashboard's `src/lib/timeline.ts`; both are
//! tested against `testdata/timeline.sqlog`.

use serde_json::{Map, Value, json};
use std::collections::{BTreeSet, HashMap};

const REQUESTS: [&str; 7] = [
  "subscribe",
  "publish",
  "fetch",
  "track_status",
  "publish_namespace",
  "subscribe_namespace",
  "subscribe_tracks",
];

fn field_text(field: &Value) -> String {
  match field.get("value").and_then(Value::as_str) {
    Some(text) => text.to_string(),
    None => format!(
      "0x{}",
      field
        .get("value_bytes")
        .and_then(Value::as_str)
        .unwrap_or("")
    ),
  }
}

fn tuple_text(fields: &Value) -> String {
  fields
    .as_array()
    .map(|f| f.iter().map(field_text).collect::<Vec<_>>().join("/"))
    .unwrap_or_default()
}

fn plain(value: &Value) -> String {
  match value {
    Value::String(s) => s.clone(),
    Value::Null => "null".into(),
    other => other.to_string(),
  }
}

fn value_text(value: &Value) -> String {
  match value {
    Value::Object(v) => {
      if v.len() == 2 && v.contains_key("group") && v.contains_key("object") {
        return format!("{}:{}", plain(&v["group"]), plain(&v["object"]));
      }
      v.iter()
        .map(|(k, x)| format!("{k}={}", value_text(x)))
        .collect::<Vec<_>>()
        .join(",")
    }
    Value::Array(_) => tuple_text(value),
    other => plain(other),
  }
}

fn option_name(option: &Value) -> String {
  match option.get("name").and_then(Value::as_str) {
    Some("unknown") | None => format!(
      "0x{:x}",
      option.get("type").and_then(Value::as_u64).unwrap_or(0)
    ),
    Some(name) => name.to_string(),
  }
}

/// A one-line description of a control message: type, track, ids, codes.
pub fn describe(m: &Map<String, Value>) -> String {
  let kind = m.get("type").and_then(Value::as_str).unwrap_or("?");
  let mut parts = vec![kind.to_uppercase()];
  for key in [
    "track_namespace",
    "track_namespace_prefix",
    "track_namespace_suffix",
  ] {
    if let Some(ns) = m.get(key) {
      let mut text = tuple_text(ns);
      if let Some(name) = m.get("track_name") {
        text += &format!(" : {}", field_text(name));
      }
      parts.push(text);
    }
  }
  if let Some(standalone) = m.get("standalone_fetch") {
    parts.push(format!(
      "{} : {}",
      tuple_text(&standalone["track_namespace"]),
      field_text(&standalone["track_name"])
    ));
  }
  for key in [
    "request_id",
    "track_alias",
    "error_code",
    "status_code",
    "stream_count",
  ] {
    if let Some(v) = m.get(key) {
      parts.push(format!("{key}={}", plain(v)));
    }
  }
  if let Some(reason) = m.get("reason_phrase").and_then(Value::as_str)
    && !reason.is_empty()
  {
    parts.push(format!("'{reason}'"));
  }
  if let Some(params) = m.get("parameters").and_then(Value::as_array)
    && !params.is_empty()
  {
    let text: Vec<_> = params
      .iter()
      .map(|p| format!("{}={}", option_name(p), value_text(&p["value"])))
      .collect();
    parts.push(format!("params[{}]", text.join(", ")));
  }
  if kind == "setup"
    && let Some(options) = m.get("setup_options").and_then(Value::as_array)
  {
    let text: Vec<_> = options
      .iter()
      .map(|o| {
        let value = match o.get("value") {
          Some(v) if !v.is_null() => v.clone(),
          _ => o["value_bytes"]["data"].clone(),
        };
        format!("{}={}", option_name(o), value_text(&value))
      })
      .collect();
    parts.push(if text.is_empty() {
      "(no options)".into()
    } else {
      text.join(", ")
    });
  }
  parts.join(" ")
}

fn raw_hex(data: &Value) -> String {
  data["raw"]["data"].as_str().unwrap_or("").to_string()
}

struct Streams {
  direction: &'static str,
  label: String,
  streams: u64,
  /// Streams that ended with FIN.
  fin: u64,
  /// Streams cut short with RESET_STREAM or STOP_SENDING.
  cut: u64,
  groups: Option<BTreeSet<u64>>,
}

fn streams_entry<'a>(
  data: &'a mut Vec<Streams>,
  index: &mut HashMap<String, usize>,
  key: String,
  new: impl FnOnce() -> Streams,
) -> &'a mut Streams {
  let i = *index.entry(key).or_insert_with(|| {
    data.push(new());
    data.len() - 1
  });
  &mut data[i]
}

/// The timeline of `events`, with at most `limit` entries; `truncated` says
/// whether entries were left out.
pub fn build(events: &[Value], limit: usize) -> Value {
  let timed: Vec<&Value> = events.iter().filter(|e| e["time"].is_number()).collect();
  let start = timed
    .first()
    .and_then(|e| e["time"].as_f64())
    .unwrap_or(0.0);
  let mut peer_setup = Value::Null;
  let mut entries: Vec<Value> = Vec::new();
  // Stream id to the position of its request in `entries`.
  let mut open_requests: HashMap<u64, usize> = HashMap::new();
  // Data streams in the order their first stream appeared, found by key.
  let mut data: Vec<Streams> = Vec::new();
  let mut data_index: HashMap<String, usize> = HashMap::new();
  // Which data group each stream belongs to, from its header.
  let mut stream_keys: HashMap<String, String> = HashMap::new();

  for event in timed {
    let at = (event["time"].as_f64().unwrap_or(0.0) - start) / 1000.0;
    let name = event["name"].as_str().unwrap_or("");
    let direction = if name.ends_with("_parsed") {
      "<-"
    } else {
      "->"
    };
    let d = &event["data"];
    if name.starts_with("moqt:control_message") {
      let Some(m) = d["message"].as_object() else {
        continue;
      };
      let kind = m.get("type").and_then(Value::as_str).unwrap_or("?");
      let stream_id = d["stream_id"].as_u64().unwrap_or(0);
      let message = json!({
        "at": at,
        "direction": direction,
        "stream_id": stream_id,
        "type": kind,
        "text": describe(m),
        "hex": raw_hex(d),
      });
      if kind == "setup" {
        if direction == "<-" {
          peer_setup = message;
        }
        continue;
      }
      if REQUESTS.contains(&kind) {
        open_requests.insert(stream_id, entries.len());
        entries.push(json!({ "kind": "request", "request": message, "responses": [] }));
      } else if let Some(&i) = open_requests.get(&stream_id) {
        entries[i]["responses"]
          .as_array_mut()
          .unwrap()
          .push(message);
      } else {
        entries.push(json!({ "kind": "message", "message": message }));
      }
    } else if name.starts_with("moqt:subgroup_header") {
      let alias = plain(&d["track_alias"]);
      let key = format!("{direction} alias {alias}");
      stream_keys.insert(
        format!("{direction} {}", plain(&d["stream_id"])),
        key.clone(),
      );
      let entry = streams_entry(&mut data, &mut data_index, key, || Streams {
        direction,
        label: format!("track_alias={alias}"),
        streams: 0,
        fin: 0,
        cut: 0,
        groups: Some(BTreeSet::new()),
      });
      entry.streams += 1;
      if let (Some(groups), Some(group)) = (&mut entry.groups, d["group_id"].as_u64()) {
        groups.insert(group);
      }
    } else if name.starts_with("moqt:fetch_header") {
      let request = plain(&d["request_id"]);
      let key = format!("{direction} fetch {request}");
      stream_keys.insert(
        format!("{direction} {}", plain(&d["stream_id"])),
        key.clone(),
      );
      let entry = streams_entry(&mut data, &mut data_index, key, || Streams {
        direction,
        label: format!("fetch request_id={request}"),
        streams: 0,
        fin: 0,
        cut: 0,
        groups: None,
      });
      entry.streams += 1;
    } else if name.starts_with("moqtail:stream_end") {
      // STOP_SENDING travels against the data: the stream it names is the other
      // side's, so its header was seen in the opposite direction.
      let end = d["end"].as_str().unwrap_or("");
      let data_direction = match (end, direction) {
        ("stop_sending", "->") => "<-",
        ("stop_sending", _) => "->",
        _ => direction,
      };
      let group = stream_keys
        .get(&format!("{data_direction} {}", plain(&d["stream_id"])))
        .and_then(|key| data_index.get(key))
        .map(|&i| &mut data[i]);
      if end == "fin" {
        if let Some(group) = group {
          group.fin += 1;
        }
      } else {
        let label = group.map(|g| {
          g.cut += 1;
          g.label.clone()
        });
        entries.push(json!({
          "kind": "stream_end",
          "at": at,
          "direction": direction,
          "stream_id": d["stream_id"],
          "end": if end == "reset" { "reset" } else { "stop_sending" },
          "error_code": d["error_code"],
          "label": label,
        }));
      }
    } else if name == "moqtail:connection_closed" {
      entries.push(json!({
        "kind": "close",
        "at": at,
        "by": plain(&d["by"]),
        "error_code": if d["error_code"].is_number() { d["error_code"].clone() } else { Value::Null },
        "reason": d["reason"].as_str().unwrap_or(""),
      }));
    } else if name == "loglevel:error" {
      entries.push(json!({
        "kind": "error",
        "at": at,
        "message": d["message"].as_str().unwrap_or("parse error"),
        "hex": raw_hex(d),
      }));
    }
  }

  let data: Vec<Value> = data
    .into_iter()
    .map(|s| {
      let mut entry = json!({
        "direction": s.direction,
        "label": s.label,
        "streams": s.streams,
        "fin": s.fin,
        "cut": s.cut,
      });
      if let Some(groups) = s.groups.filter(|g| !g.is_empty()) {
        entry["groups"] = json!({
          "first": groups.first(),
          "last": groups.last(),
          "distinct": groups.len(),
        });
      }
      entry
    })
    .collect();
  let truncated = entries.len() > limit;
  entries.truncate(limit);
  json!({ "peer_setup": peer_setup, "entries": entries, "data": data, "truncated": truncated })
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn the_shared_trace_gives_the_same_timeline_as_the_dashboard() {
    let events: Vec<Value> = include_str!("../../testdata/timeline.sqlog")
      .lines()
      .filter_map(|l| serde_json::from_str(l.trim_start_matches('\u{1e}')).ok())
      .collect();
    let expected: Value =
      serde_json::from_str(include_str!("../../testdata/timeline.expected.json")).unwrap();
    let mut timeline = build(&events, 100);
    timeline.as_object_mut().unwrap().remove("truncated");
    assert_eq!(as_floats(timeline), as_floats(expected));
  }

  /// JavaScript writes `0` where Rust writes `0.0`, and the two parse the large
  /// qlog timestamps to slightly different floats; compare numbers to the
  /// microsecond.
  fn as_floats(value: Value) -> Value {
    match value {
      Value::Number(n) => json!((n.as_f64().unwrap() * 1e6).round()),
      Value::Array(a) => Value::Array(a.into_iter().map(as_floats).collect()),
      Value::Object(o) => Value::Object(o.into_iter().map(|(k, v)| (k, as_floats(v))).collect()),
      other => other,
    }
  }

  #[test]
  fn entries_past_the_limit_are_cut_and_marked() {
    let event = |t: f64| {
      json!({ "time": t, "name": "moqt:control_message_created",
      "data": { "stream_id": 0, "message": { "type": "goaway" } } })
    };
    let timeline = build(&[event(0.0), event(1.0), event(2.0)], 2);
    assert_eq!(timeline["entries"].as_array().unwrap().len(), 2);
    assert_eq!(timeline["truncated"], true);
  }
}
