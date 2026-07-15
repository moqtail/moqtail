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
  /*
  MOQT variable-length integers.
  The number of leading 1 bits of the first byte gives length - 1.

  Leading bits  Length  Usable bits  Range
  0             1       7            0 - 127
  10            2       14           0 - 16383
  110           3       21           0 - 2097151
  1110          4       28           0 - 268435455
  11110         5       35           0 - 34359738367
  111110        6       42           0 - 4398046511103
  1111110       7       49           0 - 562949953421311
  11111110      8       56           0 - 72057594037927935
  11111111      9       64           0 - 18446744073709551615
  */
  use super::*;
  use bytes::Bytes;

  const TEST_VECTORS: &[(u64, &[u8])] = &[
    (37, &[0x25]),
    (15293, &[0xbb, 0xbd]),
    (226442877, &[0xed, 0x7f, 0x3e, 0x7d]),
    (2893212287960, &[0xfa, 0xa1, 0xa0, 0xe4, 0x03, 0xd8]),
    (151288809941952, &[0xfc, 0x89, 0x98, 0xab, 0xc6, 0x6b, 0xc0]),
    (
      70423237261249041,
      &[0xfe, 0xfa, 0x31, 0x8f, 0xa8, 0xe3, 0xca, 0x11],
    ),
    (
      18446744073709551615,
      &[0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff],
    ),
  ];

  #[test]
  fn test_vectors_encode() {
    for (value, expected) in TEST_VECTORS {
      let mut buf = BytesMut::new();
      buf.put_vi(*value).unwrap();
      assert_eq!(
        buf.freeze(),
        Bytes::from(expected.to_vec()),
        "encode {value}"
      );
    }
  }

  #[test]
  fn test_vectors_decode() {
    for (value, encoding) in TEST_VECTORS {
      let mut buf = Bytes::from(encoding.to_vec());
      assert_eq!(buf.get_vi().unwrap(), *value, "decode {encoding:?}");
      assert_eq!(buf.remaining(), 0, "consumed all bytes for {value}");
    }
  }

  #[test]
  fn encodes_minimal_length_at_boundaries() {
    // (value, expected byte length). Just below/above each length transition.
    let cases: &[(u64, usize)] = &[
      (0, 1),
      (127, 1),
      (128, 2),
      (16383, 2),
      (16384, 3),
      (2097151, 3),
      (2097152, 4),
      (268435455, 4),
      (268435456, 5),
      (34359738367, 5),
      (34359738368, 6),
      (4398046511103, 6),
      (4398046511104, 7),
      (562949953421311, 7),
      (562949953421312, 8),
      (72057594037927935, 8),
      (72057594037927936, 9),
      (u64::MAX, 9),
    ];
    for (value, len) in cases {
      let mut buf = BytesMut::new();
      buf.put_vi(*value).unwrap();
      let bytes = buf.freeze();
      assert_eq!(bytes.len(), *len, "length for {value}");
    }
  }

  #[test]
  fn encodes_boundary_values_exactly() {
    let cases: &[(u64, &[u8])] = &[
      (0, &[0x00]),
      (127, &[0x7f]),
      (128, &[0x80, 0x80]),
      (16_383, &[0xbf, 0xff]),
      (16_384, &[0xc0, 0x40, 0x00]),
      (2_097_151, &[0xdf, 0xff, 0xff]),
      (2_097_152, &[0xe0, 0x20, 0x00, 0x00]),
      (268_435_455, &[0xef, 0xff, 0xff, 0xff]),
      (268_435_456, &[0xf0, 0x10, 0x00, 0x00, 0x00]),
      (34_359_738_367, &[0xf7, 0xff, 0xff, 0xff, 0xff]),
      (34_359_738_368, &[0xf8, 0x08, 0x00, 0x00, 0x00, 0x00]),
      (4_398_046_511_103, &[0xfb, 0xff, 0xff, 0xff, 0xff, 0xff]),
      (
        4_398_046_511_104,
        &[0xfc, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00],
      ),
      (
        562_949_953_421_311,
        &[0xfd, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff],
      ),
      (
        562_949_953_421_312,
        &[0xfe, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
      ),
      (
        72_057_594_037_927_935,
        &[0xfe, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff],
      ),
      (
        72_057_594_037_927_936,
        &[0xff, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
      ),
    ];

    for (value, expected) in cases {
      let mut buf = BytesMut::new();
      buf.put_vi(*value).unwrap();
      assert_eq!(&buf.freeze()[..], *expected, "encode {value}");
    }
  }

  #[test]
  fn roundtrip_all_lengths() {
    let values: &[u64] = &[
      0,
      1,
      63,
      64,
      127,
      128,
      16383,
      16384,
      2097151,
      2097152,
      268435455,
      268435456,
      34359738367,
      34359738368,
      4398046511103,
      4398046511104,
      562949953421311,
      562949953421312,
      72057594037927935,
      72057594037927936,
      u64::MAX,
    ];
    for value in values {
      let mut buf = BytesMut::new();
      buf.put_vi(*value).unwrap();
      let mut frozen = buf.freeze();
      assert_eq!(frozen.get_vi().unwrap(), *value);
    }
  }

  #[test]
  fn decodes_non_minimal_encodings_across_lengths() {
    let cases: &[(&[u8], u64)] = &[
      (&[0x25], 37),
      (&[0x80, 0x25], 37),
      (&[0xc0, 0x00, 0x25], 37),
      (&[0xe0, 0x00, 0x00, 0x25], 37),
      (&[0xf0, 0x00, 0x00, 0x00, 0x25], 37),
      (&[0xf8, 0x00, 0x00, 0x00, 0x00, 0x25], 37),
      (&[0xfc, 0x00, 0x00, 0x00, 0x00, 0x00, 0x25], 37),
      (&[0xfe, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x25], 37),
      (&[0xff, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x25], 37),
    ];

    for (encoding, expected) in cases {
      let mut buf = Bytes::from(encoding.to_vec());
      assert_eq!(buf.get_vi().unwrap(), *expected, "decode {encoding:?}");
      assert_eq!(buf.remaining(), 0);
    }
  }

  #[test]
  fn minimal_encodings_have_expected_prefix_shape() {
    let cases: &[(u64, u8, u8)] = &[
      // value, mask, expected masked prefix
      (0, 0b1000_0000, 0b0000_0000),                 // 0xxxxxxx
      (128, 0b1100_0000, 0b1000_0000),               // 10xxxxxx
      (16_384, 0b1110_0000, 0b1100_0000),            // 110xxxxx
      (2_097_152, 0b1111_0000, 0b1110_0000),         // 1110xxxx
      (268_435_456, 0b1111_1000, 0b1111_0000),       // 11110xxx
      (34_359_738_368, 0b1111_1100, 0b1111_1000),    // 111110xx
      (4_398_046_511_104, 0b1111_1110, 0b1111_1100), // 1111110x
      (562_949_953_421_312, 0xff, 0xfe),             // 11111110
      (72_057_594_037_927_936, 0xff, 0xff),          // 11111111
    ];

    for (value, mask, expected_prefix) in cases {
      let mut buf = BytesMut::new();
      buf.put_vi(*value).unwrap();
      let bytes = buf.freeze();
      assert_eq!(bytes[0] & mask, *expected_prefix, "prefix for {value}");
    }
  }

  #[test]
  fn decode_empty_is_error() {
    assert!(Bytes::from(vec![]).get_vi().is_err());
  }

  #[test]
  fn decode_truncated_continuation_is_error() {
    // First byte announces a longer encoding than the bytes available.
    assert!(Bytes::from(vec![0x80]).get_vi().is_err()); // 2-byte, missing 1
    assert!(Bytes::from(vec![0xc0, 0x00]).get_vi().is_err()); // 3-byte, missing 1
    assert!(Bytes::from(vec![0xfe, 0x00]).get_vi().is_err()); // 8-byte, mostly missing
    assert!(Bytes::from(vec![0xff]).get_vi().is_err()); // 9-byte, only prefix
  }

  #[test]
  fn decode_leaves_trailing_bytes_untouched() {
    let mut buf = Bytes::from(vec![0x25, 0xaa, 0xbb]);
    assert_eq!(buf.get_vi().unwrap(), 37);
    assert_eq!(&buf[..], &[0xaa, 0xbb]);
  }
}
