use anyhow::{Context, Result};
use bytes::Bytes;
use ffmpeg_next::codec::encoder;
use ffmpeg_next::format::Pixel;
use tracing::{info, warn};

use crate::cmaf;
use crate::scaler::ScaledGop;
use crate::video::yuv420p_to_video_frame;

/// GOP size in frames so that each GOP holds exactly 1 second of video.
pub fn gop_size(framerate: f64) -> u32 {
  framerate.ceil() as u32
}

/// A single encoded GOP (Group of Pictures), representing one second of video.
/// Each GOP maps to one MoQ group.
#[derive(Debug)]
pub struct EncodedGop {
  pub group_id: u64,
  /// Encoded HEVC packets in display order, one per input frame.
  pub packets: Vec<Bytes>,
}

/// Hardware encoder type detected on the system.
#[derive(Debug, Clone, Copy)]
pub enum HardwareEncoder {
  Nvenc,        // NVIDIA NVENC
  Amf,          // AMD AMF (Windows)
  Qsv,          // Intel Quick Sync Video
  Vaapi,        // VA-API (AMD/Intel on Linux)
  VideoToolbox, // Apple VideoToolbox
}

/// Detects available HEVC hardware encoders, preferring NVIDIA > AMD > Intel > VA-API > Apple.
/// Actually attempts to open each encoder with a minimal config to confirm the hardware
/// is usable at runtime, not just registered in FFmpeg.
/// Call this ONCE at startup and share the result across all encoder pipelines.
pub fn detect_hardware_encoder() -> Option<HardwareEncoder> {
  let candidates = [
    ("hevc_nvenc", HardwareEncoder::Nvenc),
    ("hevc_amf", HardwareEncoder::Amf),
    ("hevc_qsv", HardwareEncoder::Qsv),
    ("hevc_vaapi", HardwareEncoder::Vaapi),
    ("hevc_videotoolbox", HardwareEncoder::VideoToolbox),
  ];

  for (name, variant) in candidates {
    if probe_encoder(name) {
      return Some(variant);
    }
  }
  None
}

/// Attempts to open a named encoder with a minimal config to verify the hardware is
/// actually usable (not just registered). Returns false if the codec is missing or
/// if the open fails (e.g. no GPU, missing driver, no hardware device).
fn probe_encoder(encoder_name: &str) -> bool {
  let codec = match ffmpeg_next::encoder::find_by_name(encoder_name) {
    Some(c) => c,
    None => return false,
  };

  let Ok(mut enc) = ffmpeg_next::codec::Context::new_with_codec(codec)
    .encoder()
    .video()
  else {
    return false;
  };

  // Minimal parameters — just enough to attempt an open.
  enc.set_width(320);
  enc.set_height(240);
  enc.set_format(Pixel::YUV420P);
  enc.set_time_base((1, 30));
  enc.set_bit_rate(500_000);

  let opts = build_encoder_opts(encoder_name, 500);
  match enc.open_with(opts) {
    Ok(_) => {
      info!("Hardware encoder probe succeeded: {}", encoder_name);
      true
    }
    Err(e) => {
      warn!(
        "Hardware encoder '{}' registered but failed to open (hardware unavailable?): {}",
        encoder_name, e
      );
      false
    }
  }
}

/// Opens an encoder with the given parameters, reads `extradata` (HVCC record),
/// then immediately closes it.  Returns the raw HVCC bytes needed to build an
/// MP4 init segment.  Falls back to a zero-length slice if the encoder provides
/// no extradata (should not happen for any HEVC encoder).
pub async fn get_extradata(
  framerate: f64,
  width: u32,
  height: u32,
  bitrate_kbps: u32,
  hw_encoder: Option<HardwareEncoder>,
) -> Result<bytes::Bytes> {
  tokio::task::spawn_blocking(move || {
    get_extradata_blocking(framerate, width, height, bitrate_kbps, hw_encoder)
  })
  .await
  .context("get_extradata task panicked")?
}

fn get_extradata_blocking(
  framerate: f64,
  width: u32,
  height: u32,
  bitrate_kbps: u32,
  hw_encoder: Option<HardwareEncoder>,
) -> Result<bytes::Bytes> {
  // Use the SAME encoder that the real pipeline will use, so the SPS/PPS
  // in the init segment's avcC box match the actual encoded bitstream.
  // The Annex B → avcC conversion in catalog.rs handles the format.
  let encoder_name = encoder_name_for(hw_encoder);
  let codec = ffmpeg_next::encoder::find_by_name(encoder_name)
    .with_context(|| format!("encoder '{}' not found", encoder_name))?;

  let mut enc = ffmpeg_next::codec::Context::new_with_codec(codec)
    .encoder()
    .video()?;

  enc.set_width(width);
  enc.set_height(height);
  enc.set_format(Pixel::YUV420P);
  enc.set_time_base((1, framerate.ceil() as i32));

  let bitrate_bps = (bitrate_kbps as i64) * 1000;
  enc.set_bit_rate(bitrate_bps as usize);
  enc.set_max_bit_rate(bitrate_bps as usize);
  enc.set_gop(gop_size(framerate));

  // GLOBAL_HEADER: put SPS/PPS in extradata (avcC record) instead of in-band.
  // Set via raw FFI to ensure the flag is actually applied.
  unsafe {
    // AV_CODEC_FLAG_GLOBAL_HEADER = 1 << 22 = 0x00400000
    (*enc.as_mut_ptr()).flags |= 0x00400000;
  }

  info!(
    "get_extradata: opening encoder '{}' for {}x{} (flags={:#010X})",
    encoder_name,
    width,
    height,
    unsafe { (*enc.as_ptr()).flags }
  );

  let opts = build_encoder_opts(encoder_name, bitrate_kbps);
  let opened = enc.open_with(opts)?;

  // Read extradata (AVCDecoderConfigurationRecord with SPS/PPS)
  // libx264 with GLOBAL_HEADER populates extradata immediately after open.
  let extra: bytes::Bytes = unsafe {
    let ctx = opened.as_ptr();
    let ptr = (*ctx).extradata;
    let size = (*ctx).extradata_size as usize;
    if ptr.is_null() || size == 0 {
      bytes::Bytes::new()
    } else {
      bytes::Bytes::copy_from_slice(std::slice::from_raw_parts(ptr, size))
    }
  };

  if extra.len() >= 4 {
    let hex_preview: String = extra
      .iter()
      .take(16)
      .map(|b| format!("{:02X}", b))
      .collect::<Vec<_>>()
      .join(" ");
    info!(
      "Extradata: {} bytes, first bytes=[{}], profile={:#04X} compat={:#04X} level={:#04X} (encoder={})",
      extra.len(),
      hex_preview,
      extra[1],
      extra[2],
      extra[3],
      encoder_name
    );
  } else {
    warn!(
      "Extradata is only {} bytes (expected ≥4) from encoder {}",
      extra.len(),
      encoder_name
    );
  }

  Ok(extra)
}

/// Returns the encoder codec name for the detected hardware (or software fallback).
fn encoder_name_for(hw_encoder: Option<HardwareEncoder>) -> &'static str {
  match hw_encoder {
    Some(HardwareEncoder::Nvenc) => "hevc_nvenc",
    Some(HardwareEncoder::Amf) => "hevc_amf",
    Some(HardwareEncoder::Qsv) => "hevc_qsv",
    Some(HardwareEncoder::Vaapi) => "hevc_vaapi",
    Some(HardwareEncoder::VideoToolbox) => "hevc_videotoolbox",
    None => "libx265",
  }
}

/// Builds the encoder option dictionary for the given encoder and bitrate.
fn build_encoder_opts(encoder_name: &str, bitrate_kbps: u32) -> ffmpeg_next::Dictionary<'_> {
  let mut opts = ffmpeg_next::Dictionary::new();
  let vbv_bufsize = bitrate_kbps * 2;
  let vbv_maxrate = bitrate_kbps;

  if encoder_name == "libx265" {
    opts.set("preset", "medium");
    opts.set("tune", "zerolatency");
    opts.set("profile", "main");
    opts.set(
      "x265-params",
      &format!(
        "scenecut=0:open-gop=0:bframes=0:rc-lookahead=0:vbv-bufsize={vbv_bufsize}:vbv-maxrate={vbv_maxrate}"
      ),
    );
  } else if encoder_name == "hevc_nvenc" {
    opts.set("preset", "p4");
    opts.set("tune", "ll");
    opts.set("rc", "cbr");
    opts.set("cbr", "1");
    opts.set("bf", "0");
    opts.set("forced-idr", "1");
    opts.set("strict_gop", "1");
  } else if encoder_name == "hevc_amf" {
    opts.set("usage", "ultralowlatency");
    opts.set("quality", "speed");
    opts.set("rc", "cbr");
    opts.set("bf", "0");
    opts.set("enforce_hrd", "1");
  } else if encoder_name == "hevc_qsv" {
    opts.set("preset", "medium");
    opts.set("look_ahead", "0");
    opts.set("async_depth", "1");
    opts.set("rc_mode", "cbr");
    opts.set("bf", "0");
  } else if encoder_name == "hevc_vaapi" {
    opts.set("rc_mode", "CBR");
    opts.set("qp", "23");
    opts.set("bf", "0");
  } else if encoder_name == "hevc_videotoolbox" {
    opts.set("realtime", "1");
    opts.set("profile", "main");
    opts.set("allow_sw", "0");
  }

  opts
}

/// Encodes pre-scaled GOPs at a fixed resolution using HEVC (CBR).
///
/// `framerate`, `width`, and `height` describe the pre-scaled input frames.
/// `hw_encoder` is the result of a single `detect_hardware_encoder()` call
/// shared across all pipeline variants.
///
/// # Resolution reconfiguration
/// The encoder is configured once for the given dimensions. If the source
/// resolution changes mid-stream (e.g. adaptive live ingest), the pipeline
/// must be torn down and restarted with the new dimensions. Dynamic encoder
/// reconfiguration is a future enhancement.
pub async fn encode(
  framerate: f64,
  width: u32,
  height: u32,
  bitrate_kbps: u32,
  hw_encoder: Option<HardwareEncoder>,
  mut scaled_rx: tokio::sync::mpsc::Receiver<ScaledGop>,
  on_gop: tokio::sync::mpsc::Sender<EncodedGop>,
) -> Result<()> {
  tokio::task::spawn_blocking(move || {
    encode_blocking(
      framerate,
      width,
      height,
      bitrate_kbps,
      hw_encoder,
      &mut scaled_rx,
      &on_gop,
    )
  })
  .await
  .context("encoder task panicked")?
}

fn encode_blocking(
  framerate: f64,
  width: u32,
  height: u32,
  bitrate_kbps: u32,
  hw_encoder: Option<HardwareEncoder>,
  scaled_rx: &mut tokio::sync::mpsc::Receiver<ScaledGop>,
  on_gop: &tokio::sync::mpsc::Sender<EncodedGop>,
) -> Result<()> {
  let encoder_name = encoder_name_for(hw_encoder);

  match hw_encoder {
    Some(HardwareEncoder::Nvenc) => info!("Using NVIDIA NVENC HEVC hardware encoder"),
    Some(HardwareEncoder::Amf) => info!("Using AMD AMF HEVC hardware encoder"),
    Some(HardwareEncoder::Qsv) => info!("Using Intel QSV HEVC hardware encoder"),
    Some(HardwareEncoder::Vaapi) => info!("Using VA-API HEVC hardware encoder"),
    Some(HardwareEncoder::VideoToolbox) => info!("Using Apple VideoToolbox HEVC hardware encoder"),
    None => info!("No hardware encoder detected, using software libx265"),
  }

  info!(
    "Encoder: {}x{} @ {:.2} fps, {} kbps CBR (HEVC/{})",
    width, height, framerate, bitrate_kbps, encoder_name
  );

  let codec = ffmpeg_next::encoder::find_by_name(encoder_name)
    .with_context(|| format!("encoder '{}' not found", encoder_name))?;

  let mut encoder = ffmpeg_next::codec::Context::new_with_codec(codec)
    .encoder()
    .video()?;

  encoder.set_width(width);
  encoder.set_height(height);
  encoder.set_format(Pixel::YUV420P);
  encoder.set_time_base((1, framerate.ceil() as i32));

  let bitrate_bps = (bitrate_kbps as i64) * 1000;
  encoder.set_bit_rate(bitrate_bps as usize);
  encoder.set_max_bit_rate(bitrate_bps as usize);
  encoder.set_gop(gop_size(framerate));
  // GLOBAL_HEADER via raw FFI — keeps SPS/PPS out of the bitstream
  unsafe {
    (*encoder.as_mut_ptr()).flags |= 0x00400000;
  }

  let opts = build_encoder_opts(encoder_name, bitrate_kbps);
  let mut encoder = encoder.open_with(opts)?;

  // Hold one GOP back so end-of-stream flush packets can be attached
  // to the final GOP instead of being discarded.
  let mut pending_last_gop: Option<EncodedGop> = None;
  let mut sequence_number: u32 = 0;

  while let Some(gop) = scaled_rx.blocking_recv() {
    let encoded = encode_gop(
      &mut encoder,
      &gop,
      width,
      height,
      framerate,
      &mut sequence_number,
    )?;

    if let Some(previous) = pending_last_gop.replace(encoded)
      && on_gop.blocking_send(previous).is_err()
    {
      warn!("Encoder: receiver dropped, stopping");
      return Ok(());
    }
  }

  // Flush frames buffered inside the encoder (e.g. reorder buffer) and append
  // trailing packets to the final GOP before sending it downstream.
  encoder.send_eof()?;
  let sample_duration = (TIMESCALE as f64 / framerate).round() as u32;
  let mut flushed_packets = Vec::new();
  let mut packet = ffmpeg_next::Packet::empty();
  while encoder.receive_packet(&mut packet).is_ok() {
    let raw = packet.data().unwrap_or(&[]);
    let hvcc_data = cmaf::annex_b_to_hvcc(raw);
    let pts = packet.pts().unwrap_or(0) as u64;
    let decode_time = pts * sample_duration as u64;
    let chunk = cmaf::wrap_cmaf_chunk(
      sequence_number,
      decode_time,
      sample_duration,
      false,
      &hvcc_data,
    );
    sequence_number += 1;
    flushed_packets.push(chunk);
  }

  if let Some(mut final_gop) = pending_last_gop {
    let flushed_count = flushed_packets.len();
    if flushed_count > 0 {
      final_gop.packets.extend(flushed_packets);
      info!(
        "Encoder: appended {} trailing packet(s) to final GOP {}",
        flushed_count, final_gop.group_id
      );
    }

    if on_gop.blocking_send(final_gop).is_err() {
      warn!("Encoder: receiver dropped, stopping");
      return Ok(());
    }
  } else if !flushed_packets.is_empty() {
    warn!(
      "Encoder: received {} trailing packet(s) at flush with no GOP to attach",
      flushed_packets.len()
    );
  }

  info!("Encoder: all GOPs processed");
  Ok(())
}

/// CMAF timescale matching the catalog's `timescale: 90000`.
const TIMESCALE: u32 = 90000;

/// Encodes a single GOP using the provided encoder context.
/// Each encoded packet is wrapped in a CMAF chunk (moof+mdat) so it can be
/// appended directly to the browser's MSE SourceBuffer.
fn encode_gop(
  encoder: &mut encoder::Video,
  gop: &ScaledGop,
  width: u32,
  height: u32,
  framerate: f64,
  sequence_number: &mut u32,
) -> Result<EncodedGop> {
  let gop_frames = gop_size(framerate) as u64;
  let sample_duration = (TIMESCALE as f64 / framerate).round() as u32;
  let mut encoded_packets = Vec::with_capacity(gop.frames.len());

  for (frame_idx, frame_data) in gop.frames.iter().enumerate() {
    let mut video_frame = yuv420p_to_video_frame(frame_data, width, height);

    // Global frame index as PTS, consistent with the 1/fps time base.
    let global_frame = gop.gop_id * gop_frames + frame_idx as u64;
    video_frame.set_pts(Some(global_frame as i64));

    // Mark the first frame of every GOP as an IDR keyframe.
    let is_keyframe = frame_idx == 0;
    if is_keyframe {
      video_frame.set_kind(ffmpeg_next::util::picture::Type::I);
    }

    encoder.send_frame(&video_frame)?;

    let mut packet = ffmpeg_next::Packet::empty();
    while encoder.receive_packet(&mut packet).is_ok() {
      let raw = packet.data().unwrap_or(&[]);
      // Convert Annex B → HVCC length-prefixed if needed
      let hvcc_data = cmaf::annex_b_to_hvcc(raw);

      let decode_time = global_frame * sample_duration as u64;
      let chunk = cmaf::wrap_cmaf_chunk(
        *sequence_number,
        decode_time,
        sample_duration,
        is_keyframe,
        &hvcc_data,
      );
      *sequence_number += 1;

      encoded_packets.push(chunk);
    }
  }

  Ok(EncodedGop {
    group_id: gop.gop_id,
    packets: encoded_packets,
  })
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_gop_size_exact_framerate() {
    assert_eq!(gop_size(30.0), 30);
    assert_eq!(gop_size(60.0), 60);
    assert_eq!(gop_size(24.0), 24);
  }

  #[test]
  fn test_gop_size_fractional_framerate_rounds_up() {
    // 29.97 fps → 30 frames per GOP
    assert_eq!(gop_size(29.97), 30);
    // 23.976 fps → 24 frames per GOP
    assert_eq!(gop_size(23.976), 24);
    // 59.94 fps → 60 frames per GOP
    assert_eq!(gop_size(59.94), 60);
  }

  #[test]
  fn test_gop_size_always_at_least_one() {
    assert_eq!(gop_size(0.5), 1);
    assert_eq!(gop_size(1.0), 1);
  }

  #[test]
  fn test_encoded_gop_uses_packets_field() {
    // Ensures the renamed field compiles and is accessible.
    let gop = EncodedGop {
      group_id: 0,
      packets: vec![],
    };
    assert_eq!(gop.packets.len(), 0);
  }
}
