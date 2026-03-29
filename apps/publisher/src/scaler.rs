use anyhow::{Context, Result};
use bytes::Bytes;
use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::decoder::RawGop;
use crate::video::yuv420p_to_video_frame;

/// A GOP of scaled raw frames at a specific target resolution.
/// Frames are in YUV420P format at the target resolution.
#[derive(Debug, Clone)]
pub struct ScaledGop {
  pub gop_id: u64,
  pub frames: Vec<Bytes>,
}

/// Scales raw decoded GOPs from source resolution to a target resolution.
///
/// Receives RawGops from the decoder broadcast channel, scales each frame
/// to the target width/height, and sends the ScaledGop to the encoder via mpsc.
///
/// If source and target resolutions match, frames pass through without scaling.
/// One scaler runs per quality variant, all in parallel.
pub async fn scale(
  source_width: u32,
  source_height: u32,
  target_width: u16,
  target_height: u16,
  mut raw_rx: mpsc::Receiver<RawGop>,
  scaled_tx: mpsc::Sender<ScaledGop>,
) -> Result<()> {
  let dst_w = target_width as u32;
  let dst_h = target_height as u32;
  let passthrough = source_width == dst_w && source_height == dst_h;

  if passthrough {
    info!(
      "Scaler {}x{}: passthrough (no scaling needed)",
      dst_w, dst_h
    );

    while let Some(raw_gop) = raw_rx.recv().await {
      let scaled = ScaledGop {
        gop_id: raw_gop.gop_id,
        frames: raw_gop.frames,
      };
      if scaled_tx.send(scaled).await.is_err() {
        warn!("Encoder dropped, stopping scaler");
        return Ok(());
      }
    }
    info!("Scaler {}x{}: decoder finished", dst_w, dst_h);
    Ok(())
  } else {
    info!(
      "Scaler {}x{} → {}x{}",
      source_width, source_height, dst_w, dst_h
    );

    // Feed GOPs to a blocking task that reuses a single scaling context,
    // and stream results back via a std::sync::mpsc channel.
    let (block_tx, block_rx) = std::sync::mpsc::channel::<RawGop>();
    let (result_tx, result_rx) = std::sync::mpsc::channel::<ScaledGop>();

    let scale_loop = tokio::task::spawn_blocking(move || {
      scale_loop_blocking(
        block_rx,
        result_tx,
        source_width,
        source_height,
        dst_w,
        dst_h,
      )
    });

    // Forward scaled GOPs from the blocking thread to the async encoder channel.
    let forward_handle = tokio::spawn(async move {
      while let Ok(scaled) = tokio::task::block_in_place(|| result_rx.recv()) {
        if scaled_tx.send(scaled).await.is_err() {
          warn!("Encoder dropped, stopping scaler {}x{}", dst_w, dst_h);
          return;
        }
      }
    });

    while let Some(raw_gop) = raw_rx.recv().await {
      if block_tx.send(raw_gop).is_err() {
        break;
      }
    }

    // Drop sender to signal the blocking loop to exit.
    drop(block_tx);

    scale_loop
      .await
      .context("scaler blocking task panicked")??;
    forward_handle
      .await
      .context("scaler forward task panicked")?;

    info!("Scaler {}x{}: done", dst_w, dst_h);
    Ok(())
  }
}

/// Runs the scaling loop on a blocking thread, reusing a single scaling context.
fn scale_loop_blocking(
  rx: std::sync::mpsc::Receiver<RawGop>,
  result_tx: std::sync::mpsc::Sender<ScaledGop>,
  src_w: u32,
  src_h: u32,
  dst_w: u32,
  dst_h: u32,
) -> Result<()> {
  let mut scaler = ffmpeg_next::software::scaling::Context::get(
    ffmpeg_next::format::Pixel::YUV420P,
    src_w,
    src_h,
    ffmpeg_next::format::Pixel::YUV420P,
    dst_w,
    dst_h,
    ffmpeg_next::software::scaling::Flags::BILINEAR,
  )
  .context("failed to create scaler")?;

  while let Ok(raw_gop) = rx.recv() {
    let mut scaled_frames = Vec::with_capacity(raw_gop.frames.len());

    for frame_data in &raw_gop.frames {
      let src_frame = yuv420p_to_video_frame(frame_data, src_w, src_h);
      let mut dst_frame = ffmpeg_next::frame::Video::empty();
      scaler.run(&src_frame, &mut dst_frame)?;
      scaled_frames.push(extract_yuv420p(&dst_frame, dst_w, dst_h));
    }

    if result_tx
      .send(ScaledGop {
        gop_id: raw_gop.gop_id,
        frames: scaled_frames,
      })
      .is_err()
    {
      break;
    }
  }

  Ok(())
}

/// Extracts YUV420P plane data from a video frame into a contiguous buffer.
fn extract_yuv420p(frame: &ffmpeg_next::frame::Video, width: u32, height: u32) -> Bytes {
  let w = width as usize;
  let h = height as usize;
  let y_size = w * h;
  let uv_size = (w / 2) * (h / 2);
  let total = y_size + 2 * uv_size;

  let mut buf = Vec::with_capacity(total);

  for row in 0..h {
    let start = row * frame.stride(0);
    buf.extend_from_slice(&frame.data(0)[start..start + w]);
  }

  let half_h = h / 2;
  let half_w = w / 2;
  for row in 0..half_h {
    let start = row * frame.stride(1);
    buf.extend_from_slice(&frame.data(1)[start..start + half_w]);
  }

  for row in 0..half_h {
    let start = row * frame.stride(2);
    buf.extend_from_slice(&frame.data(2)[start..start + half_w]);
  }

  Bytes::from(buf)
}
