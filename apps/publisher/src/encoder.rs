use anyhow::{Context, Result};
use bytes::Bytes;
use ffmpeg_next::codec::encoder;
use ffmpeg_next::format::Pixel;
use tracing::{info, warn};

use crate::scaler::ScaledGop;

/// GOP size in frames so that each GOP holds exactly 1 second of video.
pub fn gop_size(framerate: f64) -> u32 {
  framerate.ceil() as u32
}

/// A single encoded GOP (Group of Pictures), representing one second of video.
/// Each GOP maps to one MoQ group.
#[derive(Debug)]
pub struct EncodedGop {
  pub group_id: u64,
  pub frames: Vec<Bytes>,
}

/// Hardware encoder type detected on the system.
#[derive(Debug, Clone, Copy)]
enum HardwareEncoder {
  NvencHevc,  // NVIDIA NVENC
  QsvHevc,    // Intel Quick Sync Video
  VaapiHevc,  // VA-API (AMD/Intel on Linux)
  VideoToolboxHevc, // Apple VideoToolbox
}

/// Detects available hardware encoders, preferring NVIDIA > Intel > VA-API > Apple.
fn detect_hardware_encoder() -> Option<HardwareEncoder> {
  // Check for NVIDIA NVENC
  if ffmpeg_next::encoder::find_by_name("hevc_nvenc").is_some() {
    return Some(HardwareEncoder::NvencHevc);
  }

  // Check for Intel QSV
  if ffmpeg_next::encoder::find_by_name("hevc_qsv").is_some() {
    return Some(HardwareEncoder::QsvHevc);
  }

  // Check for VA-API (Linux AMD/Intel)
  if ffmpeg_next::encoder::find_by_name("hevc_vaapi").is_some() {
    return Some(HardwareEncoder::VaapiHevc);
  }

  // Check for Apple VideoToolbox
  if ffmpeg_next::encoder::find_by_name("hevc_videotoolbox").is_some() {
    return Some(HardwareEncoder::VideoToolboxHevc);
  }

  None
}

/// Encodes pre-scaled GOPs for a given quality variant using HEVC (CBR).
///
/// Receives ScaledGops (already at the target resolution) from the scaler,
/// encodes them, and sends the resulting EncodedGop to the sender via mpsc.
///
/// Multiple variants run in parallel — each with its own scaler feeding it.
pub async fn encode(
  bitrate_kbps: u32,
  mut scaled_rx: tokio::sync::mpsc::Receiver<ScaledGop>,
  on_gop: tokio::sync::mpsc::Sender<EncodedGop>,
) -> Result<()> {
  // Move to blocking thread for CPU/GPU-bound encoding work
  tokio::task::spawn_blocking(move || {
    encode_blocking(bitrate_kbps, &mut scaled_rx, &on_gop)
  })
  .await
  .context("encoder task panicked")?
}

fn encode_blocking(
  bitrate_kbps: u32,
  scaled_rx: &mut tokio::sync::mpsc::Receiver<ScaledGop>,
  on_gop: &tokio::sync::mpsc::Sender<EncodedGop>,
) -> Result<()> {
  // Detect hardware encoder
  let hw_encoder = detect_hardware_encoder();
  let encoder_name = match hw_encoder {
    Some(HardwareEncoder::NvencHevc) => {
      info!("Using NVIDIA NVENC hardware encoder");
      "hevc_nvenc"
    }
    Some(HardwareEncoder::QsvHevc) => {
      info!("Using Intel QSV hardware encoder");
      "hevc_qsv"
    }
    Some(HardwareEncoder::VaapiHevc) => {
      info!("Using VA-API hardware encoder");
      "hevc_vaapi"
    }
    Some(HardwareEncoder::VideoToolboxHevc) => {
      info!("Using Apple VideoToolbox hardware encoder");
      "hevc_videotoolbox"
    }
    None => {
      info!("No hardware encoder detected, using software libx265");
      "libx265"
    }
  };

  // Get the first GOP to determine frame dimensions
  let first_gop = match scaled_rx.blocking_recv() {
    Some(gop) => gop,
    None => {
      info!("Encoder: channel closed before receiving any GOPs");
      return Ok(());
    }
  };

  if first_gop.frames.is_empty() {
    anyhow::bail!("received empty GOP");
  }

  // Infer dimensions from first frame
  let (width, height) = infer_yuv420p_dimensions(&first_gop.frames[0])?;
  info!(
    "Encoder: {}x{} @ {} kbps CBR (HEVC/{})",
    width, height, bitrate_kbps, encoder_name
  );

  // Create encoder
  let codec = ffmpeg_next::encoder::find_by_name(encoder_name)
    .with_context(|| format!("encoder '{}' not found", encoder_name))?;

  let mut encoder = ffmpeg_next::codec::Context::new_with_codec(codec)
    .encoder()
    .video()?;

  // Set basic parameters
  encoder.set_width(width);
  encoder.set_height(height);
  encoder.set_format(Pixel::YUV420P);
  encoder.set_time_base((1, 30)); // Assuming 30fps, adjust if needed

  // CBR rate control
  let bitrate_bps = (bitrate_kbps as i64) * 1000;
  encoder.set_bit_rate(bitrate_bps as usize);
  encoder.set_max_bit_rate(bitrate_bps as usize);

  // GOP structure: 1-second closed GOPs
  let gop_frames = 30; // TODO: derive from actual framerate
  encoder.set_gop(gop_frames as u32);

  // Encoder-specific options
  let mut opts = ffmpeg_next::Dictionary::new();

  if encoder_name == "libx265" {
    // Software HEVC (libx265) settings
    opts.set("preset", "medium");
    opts.set("tune", "zerolatency");
    opts.set("profile", "main");
    
    // Critical for streaming: closed GOPs, no scene cuts, no B-frames
    opts.set(
      "x265-params",
      "scenecut=0:open-gop=0:bframes=0:rc-lookahead=0:vbv-bufsize=2000:vbv-maxrate=2000",
    );
  } else if encoder_name == "hevc_nvenc" {
    // NVIDIA NVENC settings
    opts.set("preset", "p4"); // Medium quality/speed tradeoff
    opts.set("tune", "ll"); // Low-latency
    opts.set("rc", "cbr");
    opts.set("cbr", "1");
    opts.set("bf", "0"); // No B-frames
    opts.set("forced-idr", "1"); // Force IDR at every GOP start
    opts.set("strict_gop", "1"); // Strict GOP boundaries
  } else if encoder_name == "hevc_qsv" {
    // Intel QSV settings
    opts.set("preset", "medium");
    opts.set("look_ahead", "0");
    opts.set("async_depth", "1");
    opts.set("rc_mode", "cbr");
    opts.set("bf", "0"); // No B-frames
  } else if encoder_name == "hevc_vaapi" {
    // VA-API settings
    opts.set("rc_mode", "CBR");
    opts.set("qp", "23");
    opts.set("bf", "0"); // No B-frames
  } else if encoder_name == "hevc_videotoolbox" {
    // Apple VideoToolbox settings
    opts.set("realtime", "1");
    opts.set("profile", "main");
    opts.set("allow_sw", "0"); // Hardware only
  }

  let mut encoder = encoder.open_with(opts)?;

  // Encode the first GOP
  let encoded = encode_gop(&mut encoder, &first_gop, width, height)?;
  if on_gop.blocking_send(encoded).is_err() {
    warn!("Encoder: receiver dropped, stopping");
    return Ok(());
  }

  // Encode remaining GOPs
  while let Some(gop) = scaled_rx.blocking_recv() {
    let encoded = encode_gop(&mut encoder, &gop, width, height)?;
    if on_gop.blocking_send(encoded).is_err() {
      warn!("Encoder: receiver dropped, stopping");
      return Ok(());
    }
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
) -> Result<EncodedGop> {
  let mut encoded_frames = Vec::with_capacity(gop.frames.len());

  for (frame_idx, frame_data) in gop.frames.iter().enumerate() {
    let mut video_frame = yuv420p_to_video_frame(frame_data, width, height);

    // Set PTS for proper timing
    video_frame.set_pts(Some((gop.gop_id * 30 + frame_idx as u64) as i64));

    // Mark first frame as keyframe (IDR)
    if frame_idx == 0 {
      video_frame.set_kind(ffmpeg_next::util::picture::Type::I);
    }

    // Send frame to encoder
    encoder.send_frame(&video_frame)?;

    // Receive encoded packets
    let mut packet = ffmpeg_next::Packet::empty();
    while encoder.receive_packet(&mut packet).is_ok() {
      encoded_frames.push(Bytes::copy_from_slice(packet.data().unwrap_or(&[])));
    }
  }

  Ok(EncodedGop {
    group_id: gop.gop_id,
    frames: encoded_frames,
  })
}

/// Reconstructs an ffmpeg Video frame from packed YUV420P bytes.
fn yuv420p_to_video_frame(data: &[u8], width: u32, height: u32) -> ffmpeg_next::frame::Video {
  let mut frame = ffmpeg_next::frame::Video::new(Pixel::YUV420P, width, height);

  let w = width as usize;
  let h = height as usize;
  let y_size = w * h;
  let uv_size = (w / 2) * (h / 2);

  // Copy Y plane
  let y_stride = frame.stride(0);
  for row in 0..h {
    let src_start = row * w;
    let dst_start = row * y_stride;
    frame.data_mut(0)[dst_start..dst_start + w]
      .copy_from_slice(&data[src_start..src_start + w]);
  }

  // Copy U plane
  let half_w = w / 2;
  let half_h = h / 2;
  let u_stride = frame.stride(1);
  let u_offset = y_size;
  for row in 0..half_h {
    let src_start = u_offset + row * half_w;
    let dst_start = row * u_stride;
    frame.data_mut(1)[dst_start..dst_start + half_w]
      .copy_from_slice(&data[src_start..src_start + half_w]);
  }

  // Copy V plane
  let v_stride = frame.stride(2);
  let v_offset = y_size + uv_size;
  for row in 0..half_h {
    let src_start = v_offset + row * half_w;
    let dst_start = row * v_stride;
    frame.data_mut(2)[dst_start..dst_start + half_w]
      .copy_from_slice(&data[src_start..src_start + half_w]);
  }

  frame
}

/// Infers width and height from YUV420P buffer size.
/// Returns (width, height) or error if dimensions cannot be determined.
fn infer_yuv420p_dimensions(data: &[u8]) -> Result<(u32, u32)> {
  // YUV420P layout: Y plane (w*h) + U plane (w*h/4) + V plane (w*h/4)
  // Total size: w*h + 2*(w*h/4) = w*h * 1.5
  let size = data.len();
  let pixel_count = (size * 2) / 3;

  // Try common resolutions first
  let common_resolutions = [
    (1920, 1080),
    (1280, 720),
    (854, 480),
    (640, 360),
  ];

  for (w, h) in common_resolutions {
    if w * h == pixel_count {
      return Ok((w as u32, h as u32));
    }
  }

  // Fallback: brute-force search
  let _sqrt_pixels = (pixel_count as f64).sqrt() as usize;
  for h in (360..=2160).step_by(2) {
    if pixel_count % h == 0 {
      let w = pixel_count / h;
      if (w * h) == pixel_count && w >= 640 && h >= 360 {
        return Ok((w as u32, h as u32));
      }
    }
  }

  anyhow::bail!(
    "cannot infer dimensions from {} bytes (expected w*h*1.5)",
    size
  );
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
  fn test_infer_yuv420p_dimensions_common_resolutions() {
    // 1080p: 1920*1080*1.5 = 3110400 bytes
    assert_eq!(
      infer_yuv420p_dimensions(&vec![0u8; 3110400]).unwrap(),
      (1920, 1080)
    );

    // 720p: 1280*720*1.5 = 1382400 bytes
    assert_eq!(
      infer_yuv420p_dimensions(&vec![0u8; 1382400]).unwrap(),
      (1280, 720)
    );

    // 480p: 854*480*1.5 = 614880 bytes
    assert_eq!(
      infer_yuv420p_dimensions(&vec![0u8; 614880]).unwrap(),
      (854, 480)
    );

    // 360p: 640*360*1.5 = 345600 bytes
    assert_eq!(
      infer_yuv420p_dimensions(&vec![0u8; 345600]).unwrap(),
      (640, 360)
    );
  }

  #[test]
  fn test_infer_yuv420p_dimensions_invalid_size() {
    // Random size that doesn't match any standard resolution
    assert!(infer_yuv420p_dimensions(&vec![0u8; 12345]).is_err());
  }
}
