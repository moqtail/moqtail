use anyhow::{Context, Result};
use bytes::Bytes;
use moqtail::model::data::object::Object;
use moqtail::model::data::subgroup_header::SubgroupHeader;
use moqtail::model::data::subgroup_object::SubgroupObject;
use moqtail::transport::data_stream_handler::{HeaderInfo, SendDataStream};
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{debug, info};

use crate::encoder::EncodedGop;

/// Sends encoded GOPs over a MoQ track using subgroup streams.
///
/// Each GOP maps to one MoQ group. Each frame within the GOP maps to one
/// MoQ object within that group. This provides a natural alignment:
/// - group_id = GOP index
/// - object_id = frame index within the GOP
///
/// Receives GOPs from the encoder channel and transmits them in order.
/// This runs as an async task — one per quality variant, all in parallel.
pub async fn send_track(
  connection: Arc<wtransport::Connection>,
  track_alias: u64,
  mut gop_rx: tokio::sync::mpsc::Receiver<EncodedGop>,
) -> Result<()> {
  info!("Sender (alias={}): starting", track_alias);

  let mut groups_sent = 0u64;

  while let Some(gop) = gop_rx.recv().await {
    send_group(&connection, track_alias, &gop)
      .await
      .with_context(|| format!("failed to send group {}", gop.group_id))?;

    groups_sent += 1;

    if groups_sent % 30 == 0 {
      debug!("Sender (alias={}): {} groups sent", track_alias, groups_sent);
    }
  }

  info!(
    "Sender (alias={}): finished, {} groups sent",
    track_alias, groups_sent
  );
  Ok(())
}

/// Sends a single MoQ group (GOP) using a subgroup stream.
///
/// Opens a new unidirectional stream, writes the subgroup header,
/// then writes each frame as a subgroup object.
async fn send_group(
  connection: &Arc<wtransport::Connection>,
  track_alias: u64,
  gop: &EncodedGop,
) -> Result<()> {
  let stream = connection
    .open_uni()
    .await?
    .await
    .context("failed to open uni stream")?;

  let subgroup_header =
    SubgroupHeader::new_with_explicit_id(track_alias, gop.group_id, 1u64, 1u8, true, true);
  let header_info = HeaderInfo::Subgroup {
    header: subgroup_header,
  };

  let stream = Arc::new(Mutex::new(stream));
  let mut handler = SendDataStream::new(stream, header_info)
    .await
    .context("failed to initialize subgroup stream handler")?;

  let mut prev_object_id: Option<u64> = None;
  for (object_id, frame_data) in gop.frames.iter().enumerate() {
    let subgroup_object = SubgroupObject {
      object_id: object_id as u64,
      extension_headers: Some(vec![]),
      object_status: None,
      payload: Some(Bytes::copy_from_slice(frame_data.as_ref())),
    };

    let object = Object::try_from_subgroup(subgroup_object, track_alias, gop.group_id, Some(gop.group_id), 1)
      .with_context(|| format!("failed to build object {}", object_id))?;

    handler
      .send_object(&object, prev_object_id)
      .await
      .with_context(|| format!("failed to write object {}", object_id))?;

    prev_object_id = Some(object_id as u64);
  }

  handler.flush().await.context("failed to flush subgroup stream")?;

  Ok(())
}
