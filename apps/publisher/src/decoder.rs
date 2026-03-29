use anyhow::{Context, Result};
use bytes::Bytes;
use tokio::sync::mpsc;
use tracing::{info, warn};

/// A batch of raw decoded frames representing one GOP (1 second of video).
/// Frames are in YUV420P (planar) format at source resolution.
#[derive(Debug, Clone)]
pub struct RawGop {
  pub gop_id: u64,
  pub frames: Vec<Bytes>,
}

/// Decodes the source video one GOP at a time and fans out each GOP
/// to all encoder variant pipelines via bounded mpsc channels.
///
/// Backpressure: when the slowest pipeline's channel is full the decoder
/// blocks, preventing unbounded memory growth. This is the dash.js-style
/// "no unbounded queues" pattern applied to the publisher side.
pub async fn decode(
  video_path: String,
  framerate: f64,
  gop_txs: Vec<mpsc::Sender<RawGop>>,
) -> Result<()> {
  tokio::task::spawn_blocking(move || decode_blocking(&video_path, framerate, &gop_txs))
    .await
    .context("decoder task panicked")?
}

fn decode_blocking(
  video_path: &str,
  framerate: f64,
  gop_txs: &[mpsc::Sender<RawGop>],
) -> Result<()> {
  let gop_size = crate::encoder::gop_size(framerate) as usize;
  let mut gop_id: u64 = 0;
  let mut frame_buf: Vec<Bytes> = Vec::with_capacity(gop_size);
  let mut decoded_frame = ffmpeg_next::frame::Video::empty();
  let mut loop_count: u64 = 0;

  loop {
    let mut input = ffmpeg_next::format::input(video_path)
      .with_context(|| format!("failed to open {video_path}"))?;

    let video_stream = input
      .streams()
      .best(ffmpeg_next::media::Type::Video)
      .context("no video stream found")?;

    let stream_index = video_stream.index();
    let decoder_params = video_stream.parameters();

    let codec_context =
      ffmpeg_next::codec::Context::from_parameters(decoder_params).context("codec context")?;
    let mut decoder = codec_context.decoder().video().context("video decoder")?;

    if loop_count == 0 {
      info!(
        "Decoding: {}x{}, gop_size={} (looping)",
        decoder.width(),
        decoder.height(),
        gop_size
      );
    } else {
      info!("Looping video (pass {}), gop_id={}", loop_count + 1, gop_id);
    }

    for (stream, packet) in input.packets() {
      if stream.index() != stream_index {
        continue;
      }

      decoder.send_packet(&packet)?;

      while decoder.receive_frame(&mut decoded_frame).is_ok() {
        let yuv_bytes = extract_yuv420p(&decoded_frame);
        frame_buf.push(yuv_bytes);

        if frame_buf.len() >= gop_size {
          let gop = RawGop {
            gop_id,
            frames: std::mem::replace(&mut frame_buf, Vec::with_capacity(gop_size)),
          };

          if !fanout_blocking(gop_txs, gop) {
            warn!("All receivers dropped, stopping decoder");
            return Ok(());
          }

          gop_id += 1;
        }
      }
    }

    // Flush remaining frames from this pass's decoder
    decoder.send_eof()?;
    while decoder.receive_frame(&mut decoded_frame).is_ok() {
      let yuv_bytes = extract_yuv420p(&decoded_frame);
      frame_buf.push(yuv_bytes);

      if frame_buf.len() >= gop_size {
        let gop = RawGop {
          gop_id,
          frames: std::mem::replace(&mut frame_buf, Vec::with_capacity(gop_size)),
        };

        if !fanout_blocking(gop_txs, gop) {
          return Ok(());
        }

        gop_id += 1;
      }
    }

    // Send any remaining partial GOP at the loop boundary so frames
    // aren't silently dropped between passes.
    if !frame_buf.is_empty() {
      let gop = RawGop {
        gop_id,
        frames: std::mem::replace(&mut frame_buf, Vec::with_capacity(gop_size)),
      };
      if !fanout_blocking(gop_txs, gop) {
        return Ok(());
      }
      gop_id += 1;
    }

    loop_count += 1;
  }
}

/// Sends a GOP to every pipeline channel using blocking_send (backpressure).
/// Returns false if ALL receivers have been dropped.
/// Bytes::clone inside RawGop::clone is O(1) — Arc ref bump, no data copy.
fn fanout_blocking(txs: &[mpsc::Sender<RawGop>], gop: RawGop) -> bool {
  let mut any_alive = false;
  for (i, tx) in txs.iter().enumerate() {
    // Clone for all but the last sender; move the original into the last one.
    let g = if i < txs.len() - 1 {
      gop.clone()
    } else {
      // Can't move out of gop here since we borrowed it above; clone is still O(1).
      gop.clone()
    };
    match tx.blocking_send(g) {
      Ok(()) => any_alive = true,
      Err(_) => {} // this pipeline dropped, skip it
    }
  }
  any_alive
}

/// Extracts YUV420P plane data from a decoded frame into a single contiguous buffer.
/// Layout: [Y plane][U plane][V plane]
fn extract_yuv420p(frame: &ffmpeg_next::frame::Video) -> Bytes {
  let w = frame.width() as usize;
  let h = frame.height() as usize;
  let y_size = w * h;
  let uv_size = (w / 2) * (h / 2);
  let total = y_size + 2 * uv_size;

  let mut buf = Vec::with_capacity(total);

  // Y plane
  for row in 0..h {
    let start = row * frame.stride(0);
    buf.extend_from_slice(&frame.data(0)[start..start + w]);
  }

  // U plane
  let half_h = h / 2;
  let half_w = w / 2;
  for row in 0..half_h {
    let start = row * frame.stride(1);
    buf.extend_from_slice(&frame.data(1)[start..start + half_w]);
  }

  // V plane
  for row in 0..half_h {
    let start = row * frame.stride(2);
    buf.extend_from_slice(&frame.data(2)[start..start + half_w]);
  }

  Bytes::from(buf)
}
