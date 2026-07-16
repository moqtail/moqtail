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

use crate::model::error::ParseError;
use bytes::{Buf, BufMut, Bytes, BytesMut};

pub trait BufVarIntExt {
  fn get_vi(&mut self) -> Result<u64, ParseError>;
}

pub trait BufMutVarIntExt<T> {
  fn put_vi(&mut self, value: T) -> Result<(), ParseError>;
}

impl BufVarIntExt for Bytes {
  fn get_vi(&mut self) -> Result<u64, ParseError> {
    if self.remaining() == 0 {
      return Err(ParseError::NotEnoughBytes {
        context: "varint first byte",
        needed: 1,
        available: 0,
      });
    }

    let first = self.get_u8();
    // n leading ones -> length n+1 bytes.
    let length = first.leading_ones() as usize + 1;
    let extra_bytes = length - 1;

    if self.remaining() < extra_bytes {
      return Err(ParseError::NotEnoughBytes {
        context: "varint continuation",
        needed: length,
        available: self.remaining() + 1,
      });
    }

    let mut val = if length <= 8 {
      let data_bits = 8 - length;
      u64::from(first) & ((1u64 << data_bits) - 1)
    } else {
      0
    };

    for _ in 0..extra_bytes {
      val = (val << 8) | u64::from(self.get_u8());
    }
    Ok(val)
  }
}
impl<T> BufMutVarIntExt<T> for BytesMut
where
  T: TryInto<u64>,
{
  fn put_vi(&mut self, value: T) -> Result<(), ParseError> {
    // first convert into u64 or return an error
    let v: u64 = value.try_into().map_err(|_| ParseError::CastingError {
      context: "varint put_vi",
      from_type: std::any::type_name::<T>(),
      to_type: "u64",
      details: String::new(),
    })?;

    let length = minimal_vi_length(v);

    if length == 9 {
      let mut out = [0u8; 9];
      out[0] = 0xFF;
      out[1..9].copy_from_slice(&v.to_be_bytes());
      self.put_slice(&out);
      return Ok(());
    }

    // Big-endian value in the low `length` bytes, then OR the leading-ones
    // prefix (length - 1 ones) into the top bits of the first byte.
    let mut out = [0u8; 8];
    for i in 0..length {
      out[length - 1 - i] = (v >> (8 * i)) as u8;
    }
    let ones = length - 1;
    if ones > 0 {
      out[0] |= (((1u16 << ones) - 1) << (8 - ones)) as u8;
    }
    self.put_slice(&out[0..length]);

    Ok(())
  }
}

fn minimal_vi_length(v: u64) -> usize {
  if v < (1u64 << 7) {
    1
  } else if v < (1u64 << 14) {
    2
  } else if v < (1u64 << 21) {
    3
  } else if v < (1u64 << 28) {
    4
  } else if v < (1u64 << 35) {
    5
  } else if v < (1u64 << 42) {
    6
  } else if v < (1u64 << 49) {
    7
  } else if v < (1u64 << 56) {
    8
  } else {
    9
  }
}

#[cfg(test)]
mod tests {
  //! Every vector here comes from `dev/conformance/draft18/varint.json`, which is
  //! shared with moqtail-ts. Table 1 (the length/range summary) and Table 2 (the
  //! example encodings) live there, not in this file.

  use super::*;
  use crate::conformance::{self, parse_bytes, parse_u64};
  use bytes::Bytes;

  #[test]
  fn table2_vectors_encode() {
    for v in conformance::varint().vectors.entries {
      if !v.minimal {
        continue; // encoding always produces the minimal form
      }
      let value = parse_u64(&v.value);
      let mut buf = BytesMut::new();
      buf.put_vi(value).unwrap();
      assert_eq!(
        buf.freeze(),
        Bytes::from(parse_bytes(&v.encoding)),
        "encode {value}"
      );
    }
  }

  #[test]
  fn table2_vectors_decode() {
    for v in conformance::varint().vectors.entries {
      let value = parse_u64(&v.value);
      let mut buf = Bytes::from(parse_bytes(&v.encoding));
      assert_eq!(buf.get_vi().unwrap(), value, "decode {}", v.encoding);
      assert_eq!(buf.remaining(), 0, "consumed all bytes for {value}");
    }
  }

  #[test]
  fn encodes_boundaries_at_minimal_length_and_exact_bytes() {
    for b in conformance::varint().boundaries.entries {
      let value = parse_u64(&b.value);
      let mut buf = BytesMut::new();
      buf.put_vi(value).unwrap();
      let bytes = buf.freeze();
      assert_eq!(bytes.len(), b.length, "length for {value}");
      assert_eq!(
        &bytes[..],
        &parse_bytes(&b.encoding)[..],
        "encoding for {value}"
      );
    }
  }

  #[test]
  fn roundtrips_all_lengths() {
    for raw in conformance::varint().roundtrip_values.entries {
      let value = parse_u64(&raw);
      let mut buf = BytesMut::new();
      buf.put_vi(value).unwrap();
      let mut frozen = buf.freeze();
      assert_eq!(frozen.get_vi().unwrap(), value);
    }
  }

  #[test]
  fn decodes_non_minimal_encodings_across_lengths() {
    for n in conformance::varint().non_minimal.entries {
      let mut buf = Bytes::from(parse_bytes(&n.encoding));
      assert_eq!(
        buf.get_vi().unwrap(),
        parse_u64(&n.value),
        "decode {}",
        n.encoding
      );
      assert_eq!(buf.remaining(), 0);
    }
  }

  #[test]
  fn minimal_encodings_have_expected_prefix_shape() {
    for p in conformance::varint().prefix_shapes.entries {
      let value = parse_u64(&p.value);
      let mask = conformance::parse_hex(&p.mask) as u8;
      let expected = conformance::parse_hex(&p.prefix) as u8;
      let mut buf = BytesMut::new();
      buf.put_vi(value).unwrap();
      let bytes = buf.freeze();
      assert_eq!(bytes[0] & mask, expected, "prefix for {value}");
    }
  }

  #[test]
  fn truncated_encodings_are_errors() {
    for t in conformance::varint().truncated.entries {
      let mut buf = Bytes::from(parse_bytes(&t.encoding));
      assert!(buf.get_vi().is_err(), "expected error: {}", t.reason);
    }
  }

  #[test]
  fn decode_leaves_trailing_bytes_untouched() {
    let mut buf = Bytes::from(vec![0x25, 0xaa, 0xbb]);
    assert_eq!(buf.get_vi().unwrap(), 37);
    assert_eq!(&buf[..], &[0xaa, 0xbb]);
  }
}
