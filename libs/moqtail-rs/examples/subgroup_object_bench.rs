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

//! Measures what a relay pays per subscriber to turn one Object into bytes for its
//! subgroup stream. Run with `cargo run --release --example subgroup_object_bench`.

use bytes::Bytes;
use moqtail::model::common::pair::KeyValuePair;
use moqtail::model::data::subgroup_object::SubgroupObject;
use moqtail::model::property::object_property::ObjectProperty;
use std::hint::black_box;
use std::time::Instant;

const OBJECTS: usize = 2_000;

fn object(payload_len: usize) -> SubgroupObject {
  SubgroupObject {
    object_id: 7,
    properties: Some(vec![ObjectProperty::Unknown {
      kvp: KeyValuePair::try_new_varint(2, 1_700_000_000_000).unwrap(),
    }]),
    object_status: None,
    payload: Some(Bytes::from(vec![0xab; payload_len])),
  }
}

fn run(label: &str, subscribers: usize, payload_len: usize, f: impl Fn(&SubgroupObject)) {
  let obj = object(payload_len);
  let start = Instant::now();
  for _ in 0..OBJECTS {
    for _ in 0..subscribers {
      f(&obj);
    }
  }
  let per = start.elapsed().as_nanos() as f64 / (OBJECTS * subscribers) as f64;
  println!(
    "{label:<10} subs={subscribers:<4} payload={payload_len:<7} {per:>9.0} ns/object/subscriber"
  );
}

fn main() {
  for &payload_len in &[1_200, 50_000, 200_000] {
    for &subs in &[1, 100] {
      run("serialize", subs, payload_len, |o| {
        black_box(o.serialize(Some(6), true).unwrap());
      });
      run("parts", subs, payload_len, |o| {
        black_box(o.serialize_parts(Some(6), true).unwrap());
      });
    }
  }
}
