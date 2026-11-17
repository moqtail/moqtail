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

//! QMux wire format: QUIC v1 frames, QUIC variable-length integers and transport
//! parameters, carried in records. Over WebSocket one binary message is one record.

use bytes::{Buf, Bytes};

/// QUIC transport error codes QMux reuses.
pub(crate) mod code {
  pub const FLOW_CONTROL_ERROR: u64 = 0x3;
  pub const STREAM_LIMIT_ERROR: u64 = 0x4;
  pub const STREAM_STATE_ERROR: u64 = 0x5;
  pub const FINAL_SIZE_ERROR: u64 = 0x6;
  pub const FRAME_ENCODING_ERROR: u64 = 0x7;
  pub const TRANSPORT_PARAMETER_ERROR: u64 = 0x8;
  pub const PROTOCOL_VIOLATION: u64 = 0xa;
}

// These three codepoints are provisional and expected to change before the format is
// final; they live only here so a change is one edit.
const QX_TRANSPORT_PARAMETERS: u64 = 0x3f51_5330_0d0a_0d0a;
const QX_PING_REQUEST: u64 = 0x348c_6752_9ef8_c7bd;
const QX_PING_RESPONSE: u64 = 0x348c_6752_9ef8_c7be;
const TP_MAX_RECORD_SIZE: u64 = 0x0571_c594_29cd_0845;

/// Record size every peer must accept.
pub(crate) const DEFAULT_MAX_RECORD_SIZE: u64 = 16382;

const VARINT_MAX: u64 = (1 << 62) - 1;

/// A connection error: the QUIC error code to close with, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProtocolError {
  pub code: u64,
  pub reason: String,
}

impl ProtocolError {
  pub fn new(code: u64, reason: impl Into<String>) -> Self {
    Self {
      code,
      reason: reason.into(),
    }
  }
}

fn encoding(reason: &str) -> ProtocolError {
  ProtocolError::new(code::FRAME_ENCODING_ERROR, reason)
}

pub(crate) fn varint_len(v: u64) -> usize {
  match v {
    0..=63 => 1,
    64..=16383 => 2,
    16384..=1_073_741_823 => 4,
    _ => 8,
  }
}

pub(crate) fn put_varint(out: &mut Vec<u8>, v: u64) {
  debug_assert!(v <= VARINT_MAX, "varint out of range: {v}");
  match varint_len(v) {
    1 => out.push(v as u8),
    2 => out.extend_from_slice(&(0x4000 | v as u16).to_be_bytes()),
    4 => out.extend_from_slice(&(0x8000_0000 | v as u32).to_be_bytes()),
    _ => out.extend_from_slice(&(0xc000_0000_0000_0000 | v).to_be_bytes()),
  }
}

/// Reads from a record without copying payloads: `take` hands out slices of it.
struct Reader(Bytes);

impl Reader {
  fn is_empty(&self) -> bool {
    self.0.is_empty()
  }

  fn varint(&mut self) -> Result<u64, ProtocolError> {
    let first = *self.0.first().ok_or_else(|| encoding("truncated varint"))?;
    let len = 1usize << (first >> 6);
    if self.0.len() < len {
      return Err(encoding("truncated varint"));
    }
    let mut v = u64::from(first & 0x3f);
    for b in &self.0[1..len] {
      v = (v << 8) | u64::from(*b);
    }
    self.0.advance(len);
    Ok(v)
  }

  fn take(&mut self, len: u64) -> Result<Bytes, ProtocolError> {
    let len = usize::try_from(len).map_err(|_| encoding("length too large"))?;
    if self.0.len() < len {
      return Err(encoding("frame runs past the end of the record"));
    }
    Ok(self.0.split_to(len))
  }

  fn rest(&mut self) -> Bytes {
    std::mem::take(&mut self.0)
  }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Frame {
  Padding,
  ResetStream {
    id: u64,
    code: u64,
    final_size: u64,
  },
  StopSending {
    id: u64,
    code: u64,
  },
  Stream {
    id: u64,
    offset: u64,
    data: Bytes,
    fin: bool,
  },
  MaxData(u64),
  MaxStreamData {
    id: u64,
    max: u64,
  },
  MaxStreams {
    bidi: bool,
    max: u64,
  },
  DataBlocked(u64),
  StreamDataBlocked {
    id: u64,
    limit: u64,
  },
  StreamsBlocked {
    bidi: bool,
    limit: u64,
  },
  /// `app` false is the transport variant, which also names the offending frame type.
  ConnectionClose {
    app: bool,
    code: u64,
    frame_type: u64,
    reason: Bytes,
  },
  Datagram(Bytes),
  TransportParameters(TransportParams),
  PingRequest(u64),
  PingResponse(u64),
}

/// Bytes a STREAM frame header takes at most: type, id, offset, length.
pub(crate) const STREAM_HEADER_MAX: usize = 1 + 8 + 8 + 8;

impl Frame {
  pub fn encode(&self, out: &mut Vec<u8>) {
    match self {
      Frame::Padding => out.push(0x00),
      Frame::ResetStream {
        id,
        code,
        final_size,
      } => {
        out.push(0x04);
        put_varint(out, *id);
        put_varint(out, *code);
        put_varint(out, *final_size);
      }
      Frame::StopSending { id, code } => {
        out.push(0x05);
        put_varint(out, *id);
        put_varint(out, *code);
      }
      Frame::Stream {
        id,
        offset,
        data,
        fin,
      } => {
        // Always with OFF and LEN, so a record can hold several frames.
        out.push(0x08 | 0x04 | 0x02 | u8::from(*fin));
        put_varint(out, *id);
        put_varint(out, *offset);
        put_varint(out, data.len() as u64);
        out.extend_from_slice(data);
      }
      Frame::MaxData(max) => {
        out.push(0x10);
        put_varint(out, *max);
      }
      Frame::MaxStreamData { id, max } => {
        out.push(0x11);
        put_varint(out, *id);
        put_varint(out, *max);
      }
      Frame::MaxStreams { bidi, max } => {
        out.push(if *bidi { 0x12 } else { 0x13 });
        put_varint(out, *max);
      }
      Frame::DataBlocked(limit) => {
        out.push(0x14);
        put_varint(out, *limit);
      }
      Frame::StreamDataBlocked { id, limit } => {
        out.push(0x15);
        put_varint(out, *id);
        put_varint(out, *limit);
      }
      Frame::StreamsBlocked { bidi, limit } => {
        out.push(if *bidi { 0x16 } else { 0x17 });
        put_varint(out, *limit);
      }
      Frame::ConnectionClose {
        app,
        code,
        frame_type,
        reason,
      } => {
        out.push(if *app { 0x1d } else { 0x1c });
        put_varint(out, *code);
        if !app {
          put_varint(out, *frame_type);
        }
        put_varint(out, reason.len() as u64);
        out.extend_from_slice(reason);
      }
      Frame::Datagram(data) => {
        out.push(0x31);
        put_varint(out, data.len() as u64);
        out.extend_from_slice(data);
      }
      Frame::TransportParameters(params) => {
        let mut body = Vec::new();
        params.encode(&mut body);
        put_varint(out, QX_TRANSPORT_PARAMETERS);
        put_varint(out, body.len() as u64);
        out.extend_from_slice(&body);
      }
      Frame::PingRequest(seq) => {
        put_varint(out, QX_PING_REQUEST);
        put_varint(out, *seq);
      }
      Frame::PingResponse(seq) => {
        put_varint(out, QX_PING_RESPONSE);
        put_varint(out, *seq);
      }
    }
  }

  /// Encoded size, for packing frames into a record.
  pub fn encoded_len(&self) -> usize {
    let mut out = Vec::new();
    self.encode(&mut out);
    out.len()
  }
}

/// Decodes every frame in one record.
pub(crate) fn decode_record(record: Bytes) -> Result<Vec<Frame>, ProtocolError> {
  let mut r = Reader(record);
  let mut frames = Vec::new();
  while !r.is_empty() {
    frames.push(decode_frame(&mut r)?);
  }
  Ok(frames)
}

fn decode_frame(r: &mut Reader) -> Result<Frame, ProtocolError> {
  let ty = r.varint()?;
  Ok(match ty {
    0x00 => Frame::Padding,
    0x04 => Frame::ResetStream {
      id: r.varint()?,
      code: r.varint()?,
      final_size: r.varint()?,
    },
    0x05 => Frame::StopSending {
      id: r.varint()?,
      code: r.varint()?,
    },
    0x08..=0x0f => {
      let id = r.varint()?;
      let offset = if ty & 0x04 != 0 { r.varint()? } else { 0 };
      let data = if ty & 0x02 != 0 {
        let len = r.varint()?;
        r.take(len)?
      } else {
        r.rest()
      };
      if offset + data.len() as u64 > VARINT_MAX {
        return Err(ProtocolError::new(
          code::FRAME_ENCODING_ERROR,
          "stream data past the maximum offset",
        ));
      }
      Frame::Stream {
        id,
        offset,
        data,
        fin: ty & 0x01 != 0,
      }
    }
    0x10 => Frame::MaxData(r.varint()?),
    0x11 => Frame::MaxStreamData {
      id: r.varint()?,
      max: r.varint()?,
    },
    0x12 | 0x13 => Frame::MaxStreams {
      bidi: ty == 0x12,
      max: r.varint()?,
    },
    0x14 => Frame::DataBlocked(r.varint()?),
    0x15 => Frame::StreamDataBlocked {
      id: r.varint()?,
      limit: r.varint()?,
    },
    0x16 | 0x17 => Frame::StreamsBlocked {
      bidi: ty == 0x16,
      limit: r.varint()?,
    },
    0x1c | 0x1d => {
      let app = ty == 0x1d;
      let code = r.varint()?;
      let frame_type = if app { 0 } else { r.varint()? };
      let len = r.varint()?;
      Frame::ConnectionClose {
        app,
        code,
        frame_type,
        reason: r.take(len)?,
      }
    }
    0x30 => Frame::Datagram(r.rest()),
    0x31 => {
      let len = r.varint()?;
      Frame::Datagram(r.take(len)?)
    }
    QX_TRANSPORT_PARAMETERS => {
      let len = r.varint()?;
      Frame::TransportParameters(TransportParams::decode(r.take(len)?)?)
    }
    QX_PING_REQUEST => Frame::PingRequest(r.varint()?),
    QX_PING_RESPONSE => Frame::PingResponse(r.varint()?),
    other => {
      return Err(encoding(&format!("frame type {other:#x} is not allowed")));
    }
  })
}

/// The transport parameters QMux carries. Absent parameters take the QUIC default,
/// which is 0 for every limit: a peer that omits one grants nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TransportParams {
  pub max_idle_timeout_ms: u64,
  pub initial_max_data: u64,
  pub initial_max_stream_data_bidi_local: u64,
  pub initial_max_stream_data_bidi_remote: u64,
  pub initial_max_stream_data_uni: u64,
  pub initial_max_streams_bidi: u64,
  pub initial_max_streams_uni: u64,
  /// 0: the peer does not accept datagrams.
  pub max_datagram_frame_size: u64,
  pub max_record_size: u64,
}

impl Default for TransportParams {
  fn default() -> Self {
    Self {
      max_idle_timeout_ms: 0,
      initial_max_data: 0,
      initial_max_stream_data_bidi_local: 0,
      initial_max_stream_data_bidi_remote: 0,
      initial_max_stream_data_uni: 0,
      initial_max_streams_bidi: 0,
      initial_max_streams_uni: 0,
      max_datagram_frame_size: 0,
      max_record_size: DEFAULT_MAX_RECORD_SIZE,
    }
  }
}

/// QUIC transport parameters that make no sense without QUIC packets; receiving one
/// is an error.
const PROHIBITED_PARAMS: [u64; 10] = [0x00, 0x02, 0x03, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f, 0x10];

impl TransportParams {
  fn fields(&self) -> [(u64, u64); 9] {
    [
      (0x01, self.max_idle_timeout_ms),
      (0x04, self.initial_max_data),
      (0x05, self.initial_max_stream_data_bidi_local),
      (0x06, self.initial_max_stream_data_bidi_remote),
      (0x07, self.initial_max_stream_data_uni),
      (0x08, self.initial_max_streams_bidi),
      (0x09, self.initial_max_streams_uni),
      (0x20, self.max_datagram_frame_size),
      (TP_MAX_RECORD_SIZE, self.max_record_size),
    ]
  }

  fn encode(&self, out: &mut Vec<u8>) {
    for (id, value) in self.fields() {
      let default = if id == TP_MAX_RECORD_SIZE {
        DEFAULT_MAX_RECORD_SIZE
      } else {
        0
      };
      if value != default {
        put_varint(out, id);
        put_varint(out, varint_len(value) as u64);
        put_varint(out, value);
      }
    }
  }

  fn decode(body: Bytes) -> Result<Self, ProtocolError> {
    let bad = |reason: &str| ProtocolError::new(code::TRANSPORT_PARAMETER_ERROR, reason);
    let mut r = Reader(body);
    let mut params = Self::default();
    let mut seen = Vec::new();
    while !r.is_empty() {
      let id = r.varint().map_err(|_| bad("truncated parameter id"))?;
      let len = r.varint().map_err(|_| bad("truncated parameter length"))?;
      let value_bytes = r.take(len).map_err(|_| bad("truncated parameter value"))?;
      if PROHIBITED_PARAMS.contains(&id) {
        return Err(bad(&format!("parameter {id:#x} is not allowed")));
      }
      let known = params.fields().iter().any(|(known, _)| *known == id);
      if !known {
        continue;
      }
      if seen.contains(&id) {
        return Err(bad(&format!("parameter {id:#x} repeated")));
      }
      seen.push(id);
      let mut value_reader = Reader(value_bytes);
      let value = value_reader
        .varint()
        .map_err(|_| bad("bad parameter value"))?;
      if !value_reader.is_empty() {
        return Err(bad("parameter value longer than its varint"));
      }
      match id {
        0x01 => params.max_idle_timeout_ms = value,
        0x04 => params.initial_max_data = value,
        0x05 => params.initial_max_stream_data_bidi_local = value,
        0x06 => params.initial_max_stream_data_bidi_remote = value,
        0x07 => params.initial_max_stream_data_uni = value,
        0x08 => params.initial_max_streams_bidi = value,
        0x09 => params.initial_max_streams_uni = value,
        0x20 => params.max_datagram_frame_size = value,
        _ => params.max_record_size = value,
      }
    }
    if params.initial_max_streams_bidi > 1 << 60 || params.initial_max_streams_uni > 1 << 60 {
      return Err(bad("stream limit above 2^60"));
    }
    if params.max_record_size < DEFAULT_MAX_RECORD_SIZE {
      return Err(bad("max_record_size below the minimum"));
    }
    Ok(params)
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn roundtrip(frame: Frame) {
    let mut out = Vec::new();
    frame.encode(&mut out);
    assert_eq!(out.len(), frame.encoded_len());
    let decoded = decode_record(Bytes::from(out)).expect("decode");
    assert_eq!(decoded, vec![frame]);
  }

  #[test]
  fn varint_matches_quic_examples() {
    // The four encodings QUIC's own specification uses as examples.
    for (value, bytes) in [
      (
        151_288_809_941_952_652u64,
        &[0xc2, 0x19, 0x7c, 0x5e, 0xff, 0x14, 0xe8, 0x8c][..],
      ),
      (494_878_333, &[0x9d, 0x7f, 0x3e, 0x7d][..]),
      (15_293, &[0x7b, 0xbd][..]),
      (37, &[0x25][..]),
    ] {
      let mut out = Vec::new();
      put_varint(&mut out, value);
      assert_eq!(out, bytes);
      assert_eq!(
        Reader(Bytes::copy_from_slice(bytes)).varint().unwrap(),
        value
      );
    }
  }

  #[test]
  fn every_frame_round_trips() {
    for frame in [
      Frame::Padding,
      Frame::ResetStream {
        id: 6,
        code: 0x2,
        final_size: 1000,
      },
      Frame::StopSending { id: 4, code: 0x10 },
      Frame::Stream {
        id: 2,
        offset: 70_000,
        data: Bytes::from_static(b"hello"),
        fin: true,
      },
      Frame::MaxData(1 << 24),
      Frame::MaxStreamData {
        id: 3,
        max: 1 << 20,
      },
      Frame::MaxStreams {
        bidi: true,
        max: 100,
      },
      Frame::MaxStreams {
        bidi: false,
        max: 1000,
      },
      Frame::DataBlocked(5),
      Frame::StreamDataBlocked { id: 1, limit: 9 },
      Frame::StreamsBlocked {
        bidi: false,
        limit: 7,
      },
      Frame::ConnectionClose {
        app: true,
        code: 0x10,
        frame_type: 0,
        reason: Bytes::from_static(b"bye"),
      },
      Frame::ConnectionClose {
        app: false,
        code: code::PROTOCOL_VIOLATION,
        frame_type: 0x08,
        reason: Bytes::new(),
      },
      Frame::Datagram(Bytes::from_static(b"dgram")),
      Frame::TransportParameters(TransportParams {
        max_idle_timeout_ms: 30_000,
        initial_max_data: 1 << 24,
        initial_max_streams_uni: 1000,
        max_datagram_frame_size: DEFAULT_MAX_RECORD_SIZE,
        ..Default::default()
      }),
      Frame::PingRequest(3),
      Frame::PingResponse(3),
    ] {
      roundtrip(frame);
    }
  }

  #[test]
  fn transport_parameters_frame_starts_with_its_magic() {
    let mut out = Vec::new();
    Frame::TransportParameters(TransportParams::default()).encode(&mut out);
    assert_eq!(&out[..8], b"\xffQS0\r\n\r\n");
    assert_eq!(out[8], 0, "default parameters encode to an empty body");
  }

  #[test]
  fn stream_frames_without_length_take_the_rest_of_the_record() {
    // STREAM with OFF, no LEN, FIN: id 0, offset 0, data "abc".
    let frames = decode_record(Bytes::from_static(&[0x0d, 0x00, 0x00, b'a', b'b', b'c'])).unwrap();
    assert_eq!(
      frames,
      vec![Frame::Stream {
        id: 0,
        offset: 0,
        data: Bytes::from_static(b"abc"),
        fin: true
      }]
    );
  }

  #[test]
  fn several_frames_share_a_record() {
    let mut out = Vec::new();
    Frame::MaxData(10).encode(&mut out);
    Frame::Stream {
      id: 0,
      offset: 0,
      data: Bytes::from_static(b"x"),
      fin: false,
    }
    .encode(&mut out);
    assert_eq!(decode_record(Bytes::from(out)).unwrap().len(), 2);
  }

  #[test]
  fn forbidden_and_truncated_frames_are_encoding_errors() {
    // ACK (0x02) has no place in QMux.
    let err = decode_record(Bytes::from_static(&[0x02, 0, 0, 0, 0])).unwrap_err();
    assert_eq!(err.code, code::FRAME_ENCODING_ERROR);
    // STREAM claiming 5 bytes with only 2 present.
    let err = decode_record(Bytes::from_static(&[0x0a, 0x00, 0x05, b'a', b'b'])).unwrap_err();
    assert_eq!(err.code, code::FRAME_ENCODING_ERROR);
  }

  #[test]
  fn prohibited_or_small_parameters_are_rejected() {
    // stateless_reset_token (0x02) with a 1-byte value.
    let err = TransportParams::decode(Bytes::from_static(&[0x02, 0x01, 0x00])).unwrap_err();
    assert_eq!(err.code, code::TRANSPORT_PARAMETER_ERROR);

    let mut body = Vec::new();
    put_varint(&mut body, TP_MAX_RECORD_SIZE);
    put_varint(&mut body, 2);
    put_varint(&mut body, 1000);
    let err = TransportParams::decode(Bytes::from(body)).unwrap_err();
    assert_eq!(err.code, code::TRANSPORT_PARAMETER_ERROR);

    // Unknown parameters are skipped.
    let params = TransportParams::decode(Bytes::from_static(&[0x3f, 0x01, 0x07])).unwrap();
    assert_eq!(params, TransportParams::default());
  }
}
