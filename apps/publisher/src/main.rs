mod adaptive;
mod cli;
mod connection;
mod decoder;
mod encoder;
mod scaler;
mod sender;
mod video;

use anyhow::Result;
use clap::Parser;
use cli::Cli;
use connection::MoqConnection;
use tokio_util::sync::CancellationToken;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::filter::LevelFilter;

#[tokio::main]
async fn main() -> Result<()> {
  init_logging();

  let cli = Cli::parse();

  ffmpeg_next::init().expect("failed to initialize ffmpeg");

  let video_info = video::get_video_info(&cli.video_path).await?;
  info!(
    "Source: {}x{} @ {:.2} fps",
    video_info.width, video_info.height, video_info.framerate
  );

  let variants = adaptive::quality_variants(&video_info)?;
  info!("{} adaptive variants", variants.len());
  for v in &variants {
    info!(
      "  {} — {}x{} @ {} kbps (CBR/HEVC)",
      v.quality, v.width, v.height, v.bitrate_kbps
    );
  }

  let mut moq = MoqConnection::establish(&cli.endpoint, cli.validate_cert).await?;

  let namespace = &cli.namespace;
  let mut track_aliases = Vec::with_capacity(variants.len());
  for (i, v) in variants.iter().enumerate() {
    let track_name = format!("video-{}", v.quality);
    let track_alias = (i as u64) + 1;
    moq
      .publish_track(namespace, &track_name, track_alias)
      .await?;
    track_aliases.push(track_alias);
  }

  info!("All {} tracks published", track_aliases.len());

  // Cancellation token for graceful shutdown — if any stage fails, all stop.
  let cancel = CancellationToken::new();

  // One decoder broadcasts raw GOPs to all encoder variants.
  // Buffer size = 2 GOPs so the decoder can stay slightly ahead.
  // Bytes inside RawGop is Arc-based, so broadcast clones are cheap (no frame data copied).
  let (raw_tx, _) = tokio::sync::broadcast::channel(2);

  // Spawn one scale → encode → send pipeline per variant.
  let mut tasks = Vec::with_capacity(variants.len() + 1);
  let source_width = video_info.width as u32;
  let source_height = video_info.height as u32;

  for (i, variant) in variants.into_iter().enumerate() {
    let track_alias = track_aliases[i];
    let conn = moq.connection.clone();
    let raw_rx = raw_tx.subscribe();
    let cancel = cancel.clone();

    let task = tokio::spawn(async move {
      let (scaled_tx, scaled_rx) = tokio::sync::mpsc::channel(2);
      let (gop_tx, gop_rx) = tokio::sync::mpsc::channel(2);

      info!(
        "Starting pipeline: {} (alias={})",
        variant.quality, track_alias
      );

      let scale_handle = tokio::spawn(scaler::scale(
        source_width,
        source_height,
        variant.width,
        variant.height,
        raw_rx,
        scaled_tx,
      ));
      let encode_handle = tokio::spawn(encoder::encode(variant.bitrate_kbps, scaled_rx, gop_tx));
      let send_handle = tokio::spawn(sender::send_track(conn, track_alias, gop_rx));

      let (scale_result, encode_result, send_result) =
        tokio::join!(scale_handle, encode_handle, send_handle);

      let result = (|| {
        scale_result??;
        encode_result??;
        send_result??;
        Ok::<(), anyhow::Error>(())
      })();

      if let Err(ref e) = result {
        error!("Pipeline {} failed: {:?}", variant.quality, e);
        cancel.cancel();
      }

      result
    });

    tasks.push(task);
  }

  // Spawn the single decoder that feeds all encoders.
  let video_path = cli.video_path.clone();
  let framerate = video_info.framerate;
  let cancel_for_decoder = cancel.clone();
  let decode_task = tokio::spawn(async move {
    tokio::select! {
      result = decoder::decode(video_path, framerate, raw_tx) => {
        if let Err(ref e) = result {
          error!("Decoder failed: {:?}", e);
          cancel_for_decoder.cancel();
        }
        result
      }
      _ = cancel_for_decoder.cancelled() => {
        info!("Decoder cancelled");
        Ok(())
      }
    }
  });
  tasks.push(decode_task);

  for task in tasks {
    task.await??;
  }

  info!("All pipelines finished, closing connection...");
  moq.connection.close(0u32.into(), b"Done");

  Ok(())
}

fn init_logging() {
  let env_filter = EnvFilter::builder()
    .with_default_directive(LevelFilter::INFO.into())
    .from_env_lossy();

  tracing_subscriber::fmt()
    .with_target(true)
    .with_level(true)
    .with_env_filter(env_filter)
    .init();
}
