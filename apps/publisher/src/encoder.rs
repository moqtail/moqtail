use anyhow::Result;
use bytes::Bytes;

use crate::scaler::ScaledGop;

/// GOP size in frames so that each GOP holds exactly 1 second of video.
pub fn gop_size(framerate: f64) -> u32 {
  framerate.ceil() as u32
}

/// A single encoded GOP (Group of Pictures), representing one second of video.
/// Each GOP maps to one MoQ group.
pub struct EncodedGop {
  pub group_id: u64,
  pub frames: Vec<Bytes>,
}

/// Encodes pre-scaled GOPs for a given quality variant using HEVC (CBR).
///
/// Receives ScaledGops (already at the target resolution) from the scaler,
/// encodes them, and sends the resulting EncodedGop to the sender via mpsc.
///
/// Multiple variants run in parallel — each with its own scaler feeding it.
pub async fn encode(
  _bitrate_kbps: u32,
  _scaled_rx: tokio::sync::mpsc::Receiver<ScaledGop>,
  _on_gop: tokio::sync::mpsc::Sender<EncodedGop>,
) -> Result<()> {
  // TODO: Implement HEVC CBR encoding
  //
  // High-level flow:
  // 1. For each ScaledGop received from scaled_rx:
  //    a. Feed frames to HEVC encoder with:
  //       - CBR at bitrate_kbps
  //       - Keyframe at the start of each GOP
  //    b. Collect encoded NAL units into EncodedGop
  //    c. Send the EncodedGop through on_gop channel
  // 2. When scaled_rx is closed, flush the encoder and exit
  //
  // Use spawn_blocking for the actual encode work since it's CPU-bound.
  // llok into both cpu and gpu encode check if gpu is avaliable  on system
  todo!("HEVC encoding not yet implemented")
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
}
