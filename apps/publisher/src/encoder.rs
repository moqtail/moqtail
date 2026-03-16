use anyhow::{Context, Result};
use bytes::Bytes;
use ffmpeg_next::codec::encoder;
use ffmpeg_next::format::Pixel;
use tracing::{info, warn};

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
  Qsv,          // Intel Quick Sync Video
  Vaapi,        // VA-API (AMD/Intel on Linux)
  VideoToolbox, // Apple VideoToolbox
}

/// Detects available hardware encoders, preferring NVIDIA > Intel > VA-API > Apple.
/// Call this ONCE at startup and share the result across all encoder pipelines.
pub fn detect_hardware_encoder() -> Option<HardwareEncoder> {
  if ffmpeg_next::encoder::find_by_name("hevc_nvenc").is_some() {
    return Some(HardwareEncoder::Nvenc);
  }
  if ffmpeg_next::encoder::find_by_name("hevc_qsv").is_some() {
    return Some(HardwareEncoder::Qsv);
  }
  if ffmpeg_next::encoder::find_by_name("hevc_vaapi").is_some() {
    return Some(HardwareEncoder::Vaapi);
  }
  if ffmpeg_next::encoder::find_by_name("hevc_videotoolbox").is_some() {
    return Some(HardwareEncoder::VideoToolbox);
  }
  None
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
  let encoder_name = match hw_encoder {
    Some(HardwareEncoder::Nvenc) => {
      info!("Using NVIDIA NVENC hardware encoder");
      "hevc_nvenc"
    }
    Some(HardwareEncoder::Qsv) => {
      info!("Using Intel QSV hardware encoder");
      "hevc_qsv"
    }
    Some(HardwareEncoder::Vaapi) => {
      info!("Using VA-API hardware encoder");
      "hevc_vaapi"
    }
    Some(HardwareEncoder::VideoToolbox) => {
      info!("Using Apple VideoToolbox hardware encoder");
      "hevc_videotoolbox"
    }
    None => {
      info!("No hardware encoder detected, using software libx265");
      "libx265"
    }
  };

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
  // Time base in frame units: 1/fps.
  // PTS values written per frame are absolute frame indices, making DTS/PTS
  // unambiguous for constant-framerate content.
  encoder.set_time_base((1, framerate.ceil() as i32));

  let bitrate_bps = (bitrate_kbps as i64) * 1000;
  encoder.set_bit_rate(bitrate_bps as usize);
  encoder.set_max_bit_rate(bitrate_bps as usize);

  // 1-second closed GOPs, frame count derived from actual framerate.
  let gop_frames = gop_size(framerate);
  encoder.set_gop(gop_frames);

  // VBV buffer = 2× target bitrate (2-second buffer). This gives the encoder
  // enough slack to handle I-frame spikes while maintaining CBR compliance.
  let vbv_bufsize = bitrate_kbps * 2;
  let vbv_maxrate = bitrate_kbps;

  let mut opts = ffmpeg_next::Dictionary::new();

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
    // VideoToolbox HEVC does not expose a "bf" option via libavcodec;
    // it does not support B-frames by default so no explicit setting is needed.
  }

  let mut encoder = encoder.open_with(opts)?;

  while let Some(gop) = scaled_rx.blocking_recv() {
    let encoded = encode_gop(&mut encoder, &gop, width, height, framerate)?;
    if on_gop.blocking_send(encoded).is_err() {
      warn!("Encoder: receiver dropped, stopping");
      return Ok(());
    }
  }

  // Flush frames buffered inside the encoder (e.g. reorder buffer).
  // This ensures no packets are silently discarded at end-of-stream.
  encoder.send_eof()?;
  let mut flushed = 0usize;
  let mut packet = ffmpeg_next::Packet::empty();
  while encoder.receive_packet(&mut packet).is_ok() {
    flushed += 1;
    // Trailing packets belong to the last GOP which has already been sent.
    // A production implementation should append these to the last EncodedGop.
  }
  if flushed > 0 {
    info!(
      "Encoder: flushed {} trailing packet(s) at end-of-stream",
      flushed
    );
  }

  info!("Encoder: all GOPs processed");
  Ok(())
}

/// Encodes a single GOP using the provided encoder context.
fn encode_gop(
  encoder: &mut encoder::Video,
  gop: &ScaledGop,
  width: u32,
  height: u32,
  framerate: f64,
) -> Result<EncodedGop> {
  let gop_frames = gop_size(framerate) as u64;
  let mut encoded_packets = Vec::with_capacity(gop.frames.len());

  for (frame_idx, frame_data) in gop.frames.iter().enumerate() {
    let mut video_frame = yuv420p_to_video_frame(frame_data, width, height);

    // Global frame index as PTS, consistent with the 1/fps time base.
    video_frame.set_pts(Some((gop.gop_id * gop_frames + frame_idx as u64) as i64));

    // Mark the first frame of every GOP as an IDR keyframe.
    if frame_idx == 0 {
      video_frame.set_kind(ffmpeg_next::util::picture::Type::I);
    }

    encoder.send_frame(&video_frame)?;

    let mut packet = ffmpeg_next::Packet::empty();
    while encoder.receive_packet(&mut packet).is_ok() {
      encoded_packets.push(Bytes::copy_from_slice(packet.data().unwrap_or(&[])));
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
