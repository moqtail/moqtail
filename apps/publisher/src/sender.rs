use anyhow::Result;
use std::sync::Arc;

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
  _connection: Arc<wtransport::Connection>,
  _track_alias: u64,
  _gop_rx: tokio::sync::mpsc::Receiver<EncodedGop>,
) -> Result<()> {
  // TODO: Implement MoQ track sending
  //
  // High-level flow:
  // 1. For each EncodedGop received from gop_rx:
  //    a. Open a new unidirectional stream via connection.open_uni()
  //    b. Write SubgroupHeader with track_alias and group_id
  //    c. For each frame in the GOP, write a SubgroupObject:
  //       - object_id = frame index
  //       - payload = encoded HEVC frame bytes
  //    d. Flush the stream
  //
  // The channel-based design means:
  // - Sending starts as soon as the first GOP is encoded (no full-file buffering)
  // - Backpressure from the network naturally throttles the encoder via channel capacity
  // - Each variant's sender is independent and runs in parallel

  todo!("MoQ track sending not yet implemented")
}
