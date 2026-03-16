//! Builds CMAF (fMP4) media segments: one moof+mdat per encoded access unit.
//!
//! Each MoQ object delivered to the browser's MSE SourceBuffer must be a valid
//! ISO BMFF media segment (moof + mdat). This module wraps raw HEVC packets
//! (HVCC length-prefixed NAL units, as produced by FFmpeg with GLOBAL_HEADER)
//! into that format.

use bytes::Bytes;

/// Wraps a single encoded HEVC access unit in a CMAF chunk (moof + mdat).
///
/// * `sequence_number` — increments per fragment (usually per frame).
/// * `decode_time` — in timescale ticks (e.g. 90 000 Hz).
/// * `duration` — sample duration in timescale ticks.
/// * `is_keyframe` — true for IDR/keyframe; sets sync-sample flag.
/// * `data` — raw HEVC payload (HVCC length-prefixed NALs from FFmpeg).
pub fn wrap_cmaf_chunk(
  sequence_number: u32,
  decode_time: u64,
  duration: u32,
  is_keyframe: bool,
  data: &[u8],
) -> Bytes {
  // Pre-calculate sizes bottom-up so we can write everything in one pass.
  let mdat_payload_len = data.len();
  let mdat_size = 8 + mdat_payload_len; // box header (8) + payload

  // trun: fullbox(12) + sample_count(4) + data_offset(4) + per-sample(duration 4 + size 4 + flags 4) = 32
  let trun_size: u32 = 32;
  // tfdt: fullbox(12) + baseMediaDecodeTime(8) = 20  (version 1 for u64 time)
  let tfdt_size: u32 = 20;
  // tfhd: fullbox(12) + track_id(4) = 16
  let tfhd_size: u32 = 16;
  // traf: box(8) + children
  let traf_size: u32 = 8 + tfhd_size + tfdt_size + trun_size;
  // mfhd: fullbox(12) + sequence_number(4) = 16
  let mfhd_size: u32 = 16;
  // moof: box(8) + children
  let moof_size: u32 = 8 + mfhd_size + traf_size;

  // trun data_offset: bytes from the start of moof to the start of mdat payload
  let data_offset: i32 = moof_size as i32 + 8; // +8 for mdat box header

  let total = moof_size as usize + mdat_size;
  let mut buf = Vec::with_capacity(total);

  // ---- moof ----
  write_u32(&mut buf, moof_size);
  buf.extend_from_slice(b"moof");

  // mfhd
  write_u32(&mut buf, mfhd_size);
  buf.extend_from_slice(b"mfhd");
  write_u32(&mut buf, 0); // version + flags
  write_u32(&mut buf, sequence_number);

  // traf
  write_u32(&mut buf, traf_size);
  buf.extend_from_slice(b"traf");

  // tfhd — flags: 0x020000 = default-base-is-moof
  write_u32(&mut buf, tfhd_size);
  buf.extend_from_slice(b"tfhd");
  write_u32(&mut buf, 0x00_02_00_00); // version 0, flags=default-base-is-moof
  write_u32(&mut buf, 1); // track_ID

  // tfdt — version 1 (64-bit baseMediaDecodeTime)
  write_u32(&mut buf, tfdt_size);
  buf.extend_from_slice(b"tfdt");
  write_u32(&mut buf, 0x01_00_00_00); // version 1, flags 0
  write_u64(&mut buf, decode_time);

  // trun — flags: 0x000301 = data-offset-present | first-sample-flags-present | sample-duration-present | sample-size-present
  // Actually the exact flag bits:
  //   0x000001 = data-offset-present
  //   0x000004 = first-sample-flags-present  (not needed if we use per-sample flags)
  //   0x000100 = sample-duration-present
  //   0x000200 = sample-size-present
  //   0x000400 = sample-flags-present (per sample)
  // We use per-sample: duration + size + flags = 0x000701
  let trun_flags: u32 = 0x00_00_07_01; // data-offset + duration + size + flags per sample
  write_u32(&mut buf, trun_size);
  buf.extend_from_slice(b"trun");
  write_u32(&mut buf, trun_flags); // version 0 + flags
  write_u32(&mut buf, 1); // sample_count
  write_i32(&mut buf, data_offset);
  // Per-sample fields:
  write_u32(&mut buf, duration);
  write_u32(&mut buf, mdat_payload_len as u32);
  // Sample flags: for keyframe = 0x02000000 (depends_on_nothing),
  // for non-key = 0x01010000 (is_non_sync | depends_on_other)
  let sample_flags: u32 = if is_keyframe { 0x02000000 } else { 0x01010000 };
  write_u32(&mut buf, sample_flags);

  // ---- mdat ----
  write_u32(&mut buf, mdat_size as u32);
  buf.extend_from_slice(b"mdat");
  buf.extend_from_slice(data);

  debug_assert_eq!(buf.len(), total);
  Bytes::from(buf)
}

/// Converts Annex B HEVC bitstream (start-code delimited) to HVCC/AVCC format
/// (4-byte big-endian length prefix per NAL unit).
///
/// FFmpeg with `GLOBAL_HEADER` flag + `hevc_videotoolbox` already produces
/// HVCC-format output (length-prefixed). `libx265` may produce Annex B
/// (00 00 00 01 or 00 00 01 delimiters). This function handles both cases:
/// if the data already looks like length-prefixed NALs, it returns it as-is.
pub fn annex_b_to_hvcc(data: &[u8]) -> Vec<u8> {
  // Quick heuristic: if first 4 bytes don't look like a start code,
  // assume it's already length-prefixed (HVCC format).
  if data.len() < 4 {
    return data.to_vec();
  }
  if data[0..3] != [0, 0, 1] && data[0..4] != [0, 0, 0, 1] {
    return data.to_vec();
  }

  // Split on Annex B start codes and re-prefix with 4-byte lengths.
  let mut out = Vec::with_capacity(data.len());
  let mut i = 0;
  let len = data.len();

  while i < len {
    // Skip start code
    let start;
    if i + 3 < len && data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 0 && data[i + 3] == 1 {
      start = i + 4;
    } else if i + 2 < len && data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1 {
      start = i + 3;
    } else {
      // No start code at current position — skip byte
      i += 1;
      continue;
    }

    // Find next start code (or end of data)
    let mut end = start;
    while end < len {
      if end + 3 < len
        && data[end] == 0
        && data[end + 1] == 0
        && data[end + 2] == 0
        && data[end + 3] == 1
      {
        break;
      }
      if end + 2 < len && data[end] == 0 && data[end + 1] == 0 && data[end + 2] == 1 {
        break;
      }
      end += 1;
    }

    // Strip trailing zeros that belong to the next start code
    let mut nal_end = end;
    while nal_end > start && data[nal_end - 1] == 0 {
      nal_end -= 1;
    }

    let nal_len = nal_end - start;
    if nal_len > 0 {
      out.extend_from_slice(&(nal_len as u32).to_be_bytes());
      out.extend_from_slice(&data[start..nal_end]);
    }

    i = end;
  }

  out
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn write_u32(buf: &mut Vec<u8>, v: u32) {
  buf.extend_from_slice(&v.to_be_bytes());
}

fn write_u64(buf: &mut Vec<u8>, v: u64) {
  buf.extend_from_slice(&v.to_be_bytes());
}

fn write_i32(buf: &mut Vec<u8>, v: i32) {
  buf.extend_from_slice(&v.to_be_bytes());
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_cmaf_chunk_starts_with_moof() {
    let chunk = wrap_cmaf_chunk(1, 0, 3000, true, &[0xAA; 16]);
    assert!(chunk.len() > 8);
    assert_eq!(&chunk[4..8], b"moof");
  }

  #[test]
  fn test_cmaf_chunk_ends_with_mdat_payload() {
    let payload = [0xBB; 32];
    let chunk = wrap_cmaf_chunk(1, 0, 3000, true, &payload);
    // Last 32 bytes should be our payload
    assert_eq!(&chunk[chunk.len() - 32..], &payload);
  }

  #[test]
  fn test_annex_b_passthrough_for_hvcc() {
    // Already length-prefixed: first bytes are a length, not a start code
    let data = vec![0x00, 0x00, 0x00, 0x05, 0x40, 0x01, 0x0C, 0x01, 0x00];
    let result = annex_b_to_hvcc(&data);
    assert_eq!(result, data);
  }

  #[test]
  fn test_annex_b_conversion() {
    // Annex B with 4-byte start code + 3 bytes NAL + 4-byte start code + 2 bytes NAL
    let mut data = vec![];
    data.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]); // start code
    data.extend_from_slice(&[0x40, 0x01, 0x0C]); // NAL 1 (3 bytes)
    data.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]); // start code
    data.extend_from_slice(&[0x42, 0x01]); // NAL 2 (2 bytes)

    let result = annex_b_to_hvcc(&data);
    // Should be: 4-byte len(3) + NAL1 + 4-byte len(2) + NAL2
    assert_eq!(result.len(), 4 + 3 + 4 + 2);
    assert_eq!(&result[0..4], &[0, 0, 0, 3]); // length 3
    assert_eq!(&result[4..7], &[0x40, 0x01, 0x0C]); // NAL 1
    assert_eq!(&result[7..11], &[0, 0, 0, 2]); // length 2
    assert_eq!(&result[11..13], &[0x42, 0x01]); // NAL 2
  }
}
