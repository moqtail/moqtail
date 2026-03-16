mod adaptive;
mod catalog;
mod cli;
mod cmaf;
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

  // Detect hardware encoder once rather than once per pipeline variant.
  let hw_encoder = encoder::detect_hardware_encoder();

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

  // Gather HVCC extradata for each variant (opens + closes one encoder per variant).
  info!(
    "Probing encoder extradata for {} variants...",
    variants.len()
  );
  let mut catalog_tracks: Vec<catalog::CatalogTrack> = Vec::with_capacity(variants.len());
  for v in &variants {
    let extra = encoder::get_extradata(
      video_info.framerate,
      v.width as u32,
      v.height as u32,
      v.bitrate_kbps,
      hw_encoder,
    )
    .await?;
    let init_seg = catalog::build_init_segment(&extra, v.width, v.height);
    let codec = catalog::codec_string_from_extradata(&extra);
    catalog_tracks.push(catalog::CatalogTrack {
      name: format!("video-{}", v.quality),
      codec,
      width: v.width,
      height: v.height,
      bitrate_bps: v.bitrate_kbps * 1000,
      init_segment: init_seg,
    });
  }
  let catalog_json = catalog::build_catalog_json(&catalog_tracks)?;
  info!("Catalog JSON built ({} bytes)", catalog_json.len());

  let mut moq = MoqConnection::establish(&cli.endpoint, cli.validate_cert).await?;

  let namespace = &cli.namespace;

  // Publish the catalog track first (alias 0).
  const CATALOG_ALIAS: u64 = 0;
  moq
    .publish_track(namespace, "catalog", CATALOG_ALIAS)
    .await?;
  moq
    .send_catalog_object(CATALOG_ALIAS, 0, catalog_json.clone())
    .await?;
  info!("Catalog track published and object sent");

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

  // +2: one per video pipeline, one decoder, one catalog refresh
  let mut tasks = Vec::with_capacity(variants.len() + 2);

  // Periodically resend the catalog so late-joining subscribers receive it.
  // The relay does not replay cached objects to new subscribers; instead we
  // push a new catalog group every 2 seconds. A subscriber who arrives will
  // receive it within ~2 seconds.
  let catalog_conn = moq.connection.clone();
  let catalog_json_refresh = catalog_json.clone();
  let cancel_catalog = cancel.clone();
  tasks.push(tokio::spawn(async move {
    let mut group_id: u64 = 1; // group 0 already sent above
    loop {
      tokio::select! {
        _ = tokio::time::sleep(tokio::time::Duration::from_secs(2)) => {
          if let Err(e) = connection::MoqConnection::send_catalog_object_static(
            &catalog_conn, CATALOG_ALIAS, group_id, catalog_json_refresh.clone()
          ).await {
            tracing::warn!("Catalog refresh (group {}): {:#}", group_id, e);
          }
          group_id += 1;
        }
        _ = cancel_catalog.cancelled() => {
          info!("Catalog refresh task cancelled");
          break;
        }
      }
    }
    Ok::<(), anyhow::Error>(())
  }));
  let source_width = video_info.width as u32;
  let source_height = video_info.height as u32;
  let framerate = video_info.framerate;
  // variant_count is used to assign descending priorities: the highest-quality
  // variant (index 0) gets the highest priority number (dropped first under
  // congestion) so that lower-quality tracks survive network stress.
  let variant_count = variants.len();

  for (i, variant) in variants.into_iter().enumerate() {
    let track_alias = track_aliases[i];
    let conn = moq.connection.clone();
    let raw_rx = raw_tx.subscribe();
    let cancel = cancel.clone();

    // Priority: lower-quality variants get a lower number (higher delivery priority).
    let publisher_priority = (variant_count as u8).saturating_sub(i as u8);

    let task = tokio::spawn(async move {
      let (scaled_tx, scaled_rx) = tokio::sync::mpsc::channel(2);
      let (gop_tx, gop_rx) = tokio::sync::mpsc::channel(2);

      info!(
        "Starting pipeline: {} (alias={}, priority={})",
        variant.quality, track_alias, publisher_priority
      );

      let scale_handle = tokio::spawn(scaler::scale(
        source_width,
        source_height,
        variant.width,
        variant.height,
        raw_rx,
        scaled_tx,
      ));
      let encode_handle = tokio::spawn(encoder::encode(
        framerate,
        variant.width as u32,
        variant.height as u32,
        variant.bitrate_kbps,
        hw_encoder,
        scaled_rx,
        gop_tx,
      ));
      let send_handle = tokio::spawn(sender::send_track(
        conn,
        track_alias,
        publisher_priority,
        gop_rx,
      ));

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
  // `framerate` was already captured above for the encoder pipelines.
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
