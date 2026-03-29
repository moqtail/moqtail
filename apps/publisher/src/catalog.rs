use anyhow::Result;
use base64::Engine;
use bytes::Bytes;
use std::time::{SystemTime, UNIX_EPOCH};

pub struct CatalogTrack {
  pub name: String,
  pub codec: String,
  pub width: u16,
  pub height: u16,
  pub bitrate_bps: u32,
  pub framerate: f64,
  pub role: String,
  pub target_latency_ms: u32,
  pub init_segment: Vec<u8>,
}

/// Builds an MSF draft-00 catalog JSON payload.
pub fn build_catalog_json(tracks: &[CatalogTrack]) -> Result<Bytes> {
  let generated_at = SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .map(|d| d.as_millis() as u64)
    .unwrap_or(0);

  let track_entries: Vec<serde_json::Value> = tracks
    .iter()
    .map(|t| {
      let init_b64 = base64::engine::general_purpose::STANDARD.encode(&t.init_segment);
      serde_json::json!({
        "name": t.name,
        "packaging": "cmaf",
        "role": t.role,
        "isLive": true,
        "targetLatency": t.target_latency_ms,
        "renderGroup": 1,
        "altGroup": 1,
        "codec": t.codec,
        "width": t.width,
        "height": t.height,
        "bitrate": t.bitrate_bps,
        "framerate": t.framerate,
        "timescale": 90000,
        "initData": init_b64
      })
    })
    .collect();

  let catalog = serde_json::json!({
    "version": 1,
    "generatedAt": generated_at,
    "tracks": track_entries
  });

  let json_bytes = serde_json::to_vec(&catalog)?;
  Ok(Bytes::from(json_bytes))
}

/// Ensures the extradata is a valid HEVCDecoderConfigurationRecord (HVCC).
///
/// - If it already starts with `0x01` (configurationVersion), it's HVCC — pass through.
/// - If it starts with `0x00` (Annex B start code), convert VPS/SPS/PPS to HVCC.
pub fn ensure_hvcc(extradata: &[u8]) -> Vec<u8> {
  if extradata.is_empty() {
    return Vec::new();
  }
  if extradata[0] == 0x01 {
    return extradata.to_vec();
  }
  annex_b_to_hvcc_record(extradata)
}

/// Converts Annex B HEVC extradata (VPS/SPS/PPS with start codes) to an
/// HEVCDecoderConfigurationRecord (ISO 14496-15 §8.3.3.1).
fn annex_b_to_hvcc_record(extradata: &[u8]) -> Vec<u8> {
  let nals = split_annex_b_nals(extradata);

  let mut vps_list: Vec<&[u8]> = Vec::new();
  let mut sps_list: Vec<&[u8]> = Vec::new();
  let mut pps_list: Vec<&[u8]> = Vec::new();

  for nal in &nals {
    if nal.len() < 2 {
      continue;
    }
    let nal_type = (nal[0] >> 1) & 0x3F;
    match nal_type {
      32 => vps_list.push(nal),
      33 => sps_list.push(nal),
      34 => pps_list.push(nal),
      _ => {}
    }
  }

  // Extract profile_tier_level from SPS.
  // SPS NAL layout: [0-1] NAL header, [2] vps_id+max_sub_layers+temporal_nesting,
  // [3] profile_space(2)+tier(1)+profile_idc(5), [4-7] compat_flags,
  // [8-13] constraint_flags, [14] level_idc
  let (ptl_byte, compat_flags, constraint_flags, level_idc, max_sub_layers, temporal_nesting) =
    if let Some(sps) = sps_list.first() {
      if sps.len() >= 15 {
        let b2 = sps[2];
        let max_sub = (b2 >> 1) & 0x07;
        let temporal = b2 & 0x01;
        let mut cf = [0u8; 6];
        cf.copy_from_slice(&sps[8..14]);
        (
          sps[3],
          [sps[4], sps[5], sps[6], sps[7]],
          cf,
          sps[14],
          max_sub,
          temporal,
        )
      } else {
        (0x01, [0x60, 0, 0, 0], [0u8; 6], 93, 0, 1)
      }
    } else {
      (0x01, [0x60, 0, 0, 0], [0u8; 6], 93, 0, 1)
    };

  let mut rec = Vec::with_capacity(256);

  // Header
  rec.push(1); // configurationVersion
  rec.push(ptl_byte); // profile_space(2) + tier(1) + profile_idc(5)
  rec.extend_from_slice(&compat_flags); // general_profile_compatibility_flags
  rec.extend_from_slice(&constraint_flags); // general_constraint_indicator_flags
  rec.push(level_idc); // general_level_idc
  rec.extend_from_slice(&[0xF0, 0x00]); // min_spatial_segmentation_idc = 0 + reserved
  rec.push(0xFC); // parallelismType = 0 + reserved
  rec.push(0xFC); // chromaFormatIdc = 0 + reserved (4:2:0 implied by 0)
  rec.push(0xF8); // bitDepthLumaMinus8 = 0 + reserved
  rec.push(0xF8); // bitDepthChromaMinus8 = 0 + reserved
  rec.extend_from_slice(&[0x00, 0x00]); // avgFrameRate = 0
  // constantFrameRate(2)=0 + numTemporalLayers(3) + temporalIdNested(1) + lengthSizeMinusOne(2)=3
  let misc: u8 = ((max_sub_layers + 1) << 3) | (temporal_nesting << 2) | 3;
  rec.push(misc);

  // NAL arrays
  let num_arrays =
    (!vps_list.is_empty() as u8) + (!sps_list.is_empty() as u8) + (!pps_list.is_empty() as u8);
  rec.push(num_arrays);

  // Helper closure to write one array
  let write_array = |rec: &mut Vec<u8>, nal_type: u8, nals: &[&[u8]]| {
    rec.push(0x80 | nal_type); // array_completeness(1)=1 + reserved(1)=0 + NAL_unit_type(6)
    rec.extend_from_slice(&(nals.len() as u16).to_be_bytes());
    for nal in nals {
      rec.extend_from_slice(&(nal.len() as u16).to_be_bytes());
      rec.extend_from_slice(nal);
    }
  };

  if !vps_list.is_empty() {
    write_array(&mut rec, 32, &vps_list);
  }
  if !sps_list.is_empty() {
    write_array(&mut rec, 33, &sps_list);
  }
  if !pps_list.is_empty() {
    write_array(&mut rec, 34, &pps_list);
  }

  rec
}

/// Splits Annex B byte stream into individual NAL units (without start codes).
fn split_annex_b_nals(data: &[u8]) -> Vec<&[u8]> {
  let mut nals = Vec::new();
  let mut i = 0;
  let len = data.len();

  while i < len {
    let nal_start;
    if i + 3 < len && data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 0 && data[i + 3] == 1 {
      nal_start = i + 4;
    } else if i + 2 < len && data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1 {
      nal_start = i + 3;
    } else {
      i += 1;
      continue;
    }

    let mut nal_end = nal_start;
    while nal_end < len {
      if nal_end + 3 < len
        && data[nal_end] == 0
        && data[nal_end + 1] == 0
        && data[nal_end + 2] == 0
        && data[nal_end + 3] == 1
      {
        break;
      }
      if nal_end + 2 < len && data[nal_end] == 0 && data[nal_end + 1] == 0 && data[nal_end + 2] == 1
      {
        break;
      }
      nal_end += 1;
    }

    if nal_end > nal_start {
      nals.push(&data[nal_start..nal_end]);
    }
    i = nal_end;
  }
  nals
}

/// Returns an HEVC codec string from extradata.
/// Handles both HVCC and Annex B formats.
pub fn codec_string_from_extradata(extradata: &[u8]) -> String {
  let hvcc = ensure_hvcc(extradata);
  if hvcc.len() < 13 {
    return "hvc1.1.6.L93.B0".to_owned();
  }
  let b1 = hvcc[1];
  let profile_space = (b1 >> 6) & 0x3;
  let tier_flag = (b1 >> 5) & 0x1;
  let profile_idc = b1 & 0x1F;
  let level_idc = hvcc[12];

  if profile_idc == 0 || level_idc == 0 {
    return "hvc1.1.6.L93.B0".to_owned();
  }

  let compat_flags = u32::from_be_bytes([hvcc[2], hvcc[3], hvcc[4], hvcc[5]]).reverse_bits();
  let space_str = match profile_space {
    0 => String::new(),
    1 => "A".to_owned(),
    2 => "B".to_owned(),
    3 => "C".to_owned(),
    _ => String::new(),
  };
  let tier_char = if tier_flag == 0 { 'L' } else { 'H' };
  format!(
    "hvc1.{}{}.{:X}.{}{}.B0",
    space_str, profile_idc, compat_flags, tier_char, level_idc
  )
}

/// Builds a minimal CMAF init segment (ftyp + moov) for HEVC video.
/// Handles both HVCC and Annex B extradata formats.
// Retained for future `initData` support in the MSF catalog.
#[allow(dead_code)]
pub fn build_init_segment(extradata: &[u8], width: u16, height: u16) -> Vec<u8> {
  let hvcc = ensure_hvcc(extradata);
  let mut buf = Vec::with_capacity(512);
  mp4::write_ftyp(&mut buf);
  mp4::write_moov(&mut buf, &hvcc, width, height);
  buf
}

// ---------------------------------------------------------------------------
// MP4 box helpers (retained for future `initData` support)
// ---------------------------------------------------------------------------
mod mp4 {
  #![allow(dead_code)]

  fn write_u8(buf: &mut Vec<u8>, v: u8) {
    buf.push(v);
  }
  fn write_u16(buf: &mut Vec<u8>, v: u16) {
    buf.extend_from_slice(&v.to_be_bytes());
  }
  fn write_u32(buf: &mut Vec<u8>, v: u32) {
    buf.extend_from_slice(&v.to_be_bytes());
  }
  fn write_u64(buf: &mut Vec<u8>, v: u64) {
    buf.extend_from_slice(&v.to_be_bytes());
  }

  /// Writes a size-prefixed box: reserves space for the 4-byte size, writes the
  /// 4-byte type, calls `body`, then back-patches the size.
  fn write_box<F: FnOnce(&mut Vec<u8>)>(buf: &mut Vec<u8>, box_type: &[u8; 4], body: F) {
    let start = buf.len();
    write_u32(buf, 0); // placeholder size
    buf.extend_from_slice(box_type);
    body(buf);
    let end = buf.len();
    let size = (end - start) as u32;
    buf[start..start + 4].copy_from_slice(&size.to_be_bytes());
  }

  /// Full-box: version (1 byte) + flags (3 bytes) prepended before `body`.
  fn write_fullbox<F: FnOnce(&mut Vec<u8>)>(
    buf: &mut Vec<u8>,
    box_type: &[u8; 4],
    version: u8,
    flags: u32,
    body: F,
  ) {
    write_box(buf, box_type, |b| {
      b.push(version);
      b.extend_from_slice(&flags.to_be_bytes()[1..]); // 3 bytes
      body(b);
    });
  }

  // ftyp

  pub(super) fn write_ftyp(buf: &mut Vec<u8>) {
    write_box(buf, b"ftyp", |b| {
      b.extend_from_slice(b"iso5"); // major_brand
      write_u32(b, 1); // minor_version
      b.extend_from_slice(b"cmf2"); // compatible_brands[0]
      b.extend_from_slice(b"iso5"); // compatible_brands[1]
    });
  }

  // moov

  pub(super) fn write_moov(buf: &mut Vec<u8>, extradata: &[u8], width: u16, height: u16) {
    write_box(buf, b"moov", |b| {
      write_mvhd(b);
      write_trak(b, extradata, width, height);
      write_mvex(b);
    });
  }

  fn write_mvhd(buf: &mut Vec<u8>) {
    write_fullbox(buf, b"mvhd", 0, 0, |b| {
      write_u32(b, 0); // creation_time
      write_u32(b, 0); // modification_time
      write_u32(b, 90000); // timescale
      write_u32(b, 0); // duration
      write_u32(b, 0x00010000); // rate (1.0)
      write_u16(b, 0x0100); // volume (1.0)
      b.extend_from_slice(&[0u8; 10]); // reserved
      // unity matrix
      write_u32(b, 0x00010000);
      write_u32(b, 0);
      write_u32(b, 0);
      write_u32(b, 0);
      write_u32(b, 0x00010000);
      write_u32(b, 0);
      write_u32(b, 0);
      write_u32(b, 0);
      write_u32(b, 0x40000000);
      b.extend_from_slice(&[0u8; 24]); // pre_defined
      write_u32(b, 2); // next_track_ID
    });
  }

  fn write_trak(buf: &mut Vec<u8>, extradata: &[u8], width: u16, height: u16) {
    write_box(buf, b"trak", |b| {
      write_tkhd(b, width, height);
      write_mdia(b, extradata, width, height);
    });
  }

  fn write_tkhd(buf: &mut Vec<u8>, width: u16, height: u16) {
    // flags: track_enabled(1) | track_in_movie(2) | track_in_preview(4)
    write_fullbox(buf, b"tkhd", 0, 0x00000003, |b| {
      write_u32(b, 0); // creation_time
      write_u32(b, 0); // modification_time
      write_u32(b, 1); // track_ID
      write_u32(b, 0); // reserved
      write_u32(b, 0); // duration
      b.extend_from_slice(&[0u8; 8]); // reserved
      write_u16(b, 0); // layer
      write_u16(b, 0); // alternate_group
      write_u16(b, 0); // volume
      write_u16(b, 0); // reserved
      // unity matrix
      write_u32(b, 0x00010000);
      write_u32(b, 0);
      write_u32(b, 0);
      write_u32(b, 0);
      write_u32(b, 0x00010000);
      write_u32(b, 0);
      write_u32(b, 0);
      write_u32(b, 0);
      write_u32(b, 0x40000000);
      // width/height as 16.16 fixed point
      write_u32(b, (width as u32) << 16);
      write_u32(b, (height as u32) << 16);
    });
  }

  fn write_mdia(buf: &mut Vec<u8>, extradata: &[u8], width: u16, height: u16) {
    write_box(buf, b"mdia", |b| {
      write_mdhd(b);
      write_hdlr(b);
      write_minf(b, extradata, width, height);
    });
  }

  fn write_mdhd(buf: &mut Vec<u8>) {
    write_fullbox(buf, b"mdhd", 0, 0, |b| {
      write_u32(b, 0); // creation_time
      write_u32(b, 0); // modification_time
      write_u32(b, 90000); // timescale
      write_u32(b, 0); // duration
      write_u16(b, 0x55C4); // language: 'und'
      write_u16(b, 0); // pre_defined
    });
  }

  fn write_hdlr(buf: &mut Vec<u8>) {
    write_fullbox(buf, b"hdlr", 0, 0, |b| {
      write_u32(b, 0); // pre_defined
      b.extend_from_slice(b"vide"); // handler_type
      b.extend_from_slice(&[0u8; 12]); // reserved
      b.extend_from_slice(b"VideoHandler\0"); // name
    });
  }

  fn write_minf(buf: &mut Vec<u8>, extradata: &[u8], width: u16, height: u16) {
    write_box(buf, b"minf", |b| {
      write_vmhd(b);
      write_dinf(b);
      write_stbl(b, extradata, width, height);
    });
  }

  fn write_vmhd(buf: &mut Vec<u8>) {
    write_fullbox(buf, b"vmhd", 0, 1, |b| {
      write_u16(b, 0); // graphicsMode
      write_u16(b, 0); // opcolor[0]
      write_u16(b, 0); // opcolor[1]
      write_u16(b, 0); // opcolor[2]
    });
  }

  fn write_dinf(buf: &mut Vec<u8>) {
    write_box(buf, b"dinf", |b| {
      write_fullbox(b, b"dref", 0, 0, |b2| {
        write_u32(b2, 1); // entry_count
        // url entry with self-contained flag
        write_fullbox(b2, b"url ", 0, 1, |_| {});
      });
    });
  }

  fn write_stbl(buf: &mut Vec<u8>, extradata: &[u8], width: u16, height: u16) {
    write_box(buf, b"stbl", |b| {
      write_stsd(b, extradata, width, height);
      // Empty stts, stsc, stsz, stco — required by ISO but empty for fragmented MP4
      write_fullbox(b, b"stts", 0, 0, |b2| write_u32(b2, 0));
      write_fullbox(b, b"stsc", 0, 0, |b2| write_u32(b2, 0));
      write_fullbox(b, b"stsz", 0, 0, |b2| {
        write_u32(b2, 0); // sample_size
        write_u32(b2, 0); // sample_count
      });
      write_fullbox(b, b"stco", 0, 0, |b2| write_u32(b2, 0));
    });
  }

  fn write_stsd(buf: &mut Vec<u8>, extradata: &[u8], width: u16, height: u16) {
    write_fullbox(buf, b"stsd", 0, 0, |b| {
      write_u32(b, 1); // entry_count
      write_hvc1(b, extradata, width, height);
    });
  }

  fn write_hvc1(buf: &mut Vec<u8>, extradata: &[u8], width: u16, height: u16) {
    write_box(buf, b"hvc1", |b| {
      b.extend_from_slice(&[0u8; 6]); // reserved
      write_u16(b, 1); // data_reference_index
      b.extend_from_slice(&[0u8; 16]); // pre_defined + reserved
      write_u16(b, width);
      write_u16(b, height);
      write_u32(b, 0x00480000); // horizresolution (72 dpi)
      write_u32(b, 0x00480000); // vertresolution (72 dpi)
      write_u32(b, 0); // reserved
      write_u16(b, 1); // frame_count
      b.extend_from_slice(&[0u8; 32]); // compressorname
      write_u16(b, 0x0018); // depth (24-bit color)
      write_u16(b, 0xFFFF); // pre_defined (-1)
      write_hvcc(b, extradata);
    });
  }

  fn write_hvcc(buf: &mut Vec<u8>, extradata: &[u8]) {
    write_box(buf, b"hvcC", |b| {
      b.extend_from_slice(extradata);
    });
  }

  // mvex / trex

  fn write_mvex(buf: &mut Vec<u8>) {
    write_box(buf, b"mvex", |b| {
      write_fullbox(b, b"trex", 0, 0, |b2| {
        write_u32(b2, 1); // track_ID
        write_u32(b2, 1); // default_sample_description_index
        write_u32(b2, 0); // default_sample_duration
        write_u32(b2, 0); // default_sample_size
        write_u32(b2, 0); // default_sample_flags
      });
    });
  }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_codec_string_fallback_on_short_extradata() {
    let result = codec_string_from_extradata(&[]);
    assert_eq!(result, "hvc1.1.6.L93.B0");
  }

  #[test]
  fn test_hvcc_passthrough() {
    // Already HVCC format (starts with 0x01)
    let mut hvcc = vec![0u8; 23];
    hvcc[0] = 0x01; // configurationVersion
    hvcc[1] = 0x01; // Main profile
    hvcc[2] = 0x60;
    hvcc[3] = 0;
    hvcc[4] = 0;
    hvcc[5] = 0; // compat
    hvcc[12] = 93; // level
    let result = ensure_hvcc(&hvcc);
    assert_eq!(result, hvcc);
  }

  #[test]
  fn test_codec_string_from_hevc_annex_b() {
    // HEVC Annex B: VPS(type=32) + SPS(type=33) + PPS(type=34)
    // SPS NAL: [0-1]=header, [2]=vps_id+subs+temporal, [3]=profile_space+tier+profile_idc
    // [4-7]=compat_flags, [8-13]=constraints, [14]=level_idc
    let mut extra = vec![];
    // VPS (type 32): first byte = (32 << 1) = 0x40
    extra.extend_from_slice(&[0, 0, 0, 1]); // start code
    extra.extend_from_slice(&[0x40, 0x01, 0x0C, 0x01, 0xFF, 0xFF]); // VPS NAL
    // SPS (type 33): first byte = (33 << 1) = 0x42
    extra.extend_from_slice(&[0, 0, 0, 1]); // start code
    let mut sps = vec![0x42, 0x01]; // NAL header
    sps.push(0x01); // vps_id(4)+max_sub_layers(3)+temporal(1)
    sps.push(0x01); // profile_space=0, tier=0, profile_idc=1
    sps.extend_from_slice(&[0x60, 0x00, 0x00, 0x00]); // compat flags
    sps.extend_from_slice(&[0x90, 0x00, 0x00, 0x00, 0x00, 0x00]); // constraints
    sps.push(93); // level_idc = 93
    extra.extend_from_slice(&sps);
    // PPS (type 34): first byte = (34 << 1) = 0x44
    extra.extend_from_slice(&[0, 0, 0, 1]);
    extra.extend_from_slice(&[0x44, 0x01, 0xC1, 0x72]);

    let codec = codec_string_from_extradata(&extra);
    assert_eq!(codec, "hvc1.1.6.L93.B0");
  }

  #[test]
  fn test_build_init_segment_starts_with_ftyp() {
    // Fake HVCC record (starts with 0x01)
    let mut extra = vec![0u8; 23];
    extra[0] = 0x01;
    let seg = build_init_segment(&extra, 1280, 720);
    assert!(seg.len() > 32);
    assert_eq!(&seg[4..8], b"ftyp");
  }

  #[test]
  fn test_build_catalog_json_round_trips() {
    let tracks = vec![CatalogTrack {
      name: "video-720p".to_owned(),
      codec: "hvc1.1.6.L93.B0".to_owned(),
      width: 1280,
      height: 720,
      bitrate_bps: 2_000_000,
      framerate: 30.0,
      role: "video".to_owned(),
      target_latency_ms: 1500,
      init_segment: vec![0, 1, 2, 3],
    }];
    let json_bytes = build_catalog_json(&tracks).unwrap();
    let v: serde_json::Value = serde_json::from_slice(&json_bytes).unwrap();
    assert_eq!(v["version"], 1);
    assert!(v["generatedAt"].is_u64());
    assert_eq!(v["tracks"][0]["name"], "video-720p");
    assert_eq!(v["tracks"][0]["packaging"], "cmaf");
    assert_eq!(v["tracks"][0]["role"], "video");
    assert_eq!(v["tracks"][0]["targetLatency"], 1500);
    assert_eq!(v["tracks"][0]["renderGroup"], 1);
    assert_eq!(v["tracks"][0]["altGroup"], 1);
    assert_eq!(v["tracks"][0]["isLive"], true);
    assert_eq!(v["tracks"][0]["framerate"], 30.0);
    assert_eq!(v["tracks"][0]["timescale"], 90000);
    assert_eq!(v["tracks"][0]["initData"], "AAECAw==");
  }
}
