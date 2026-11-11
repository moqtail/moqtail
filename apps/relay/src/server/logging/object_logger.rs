// Copyright 2025 The MOQtail Authors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use moqtail::model::data::object::Object;
use std::path::Path;
use tokio::fs::OpenOptions;
use tokio::io::AsyncWriteExt;
use tracing::error;

/// Object logger for tracking object properties in MoQ relay
#[derive(Debug, Clone)]
pub struct ObjectLogger {
  log_folder: String,
}

impl ObjectLogger {
  /// Create a new ObjectLogger instance
  pub fn new(log_folder: String) -> Self {
    Self { log_folder }
  }

  /// Log object properties for subscription-level logging
  /// Filename format: <track_alias>_<subscriber_connection_id>.log
  /// Fields: group_id, subgroup_id, object_id, payload_size, send_status, object_received_time
  pub async fn log_subscription_object(
    &self,
    track_alias: u64,
    subscriber_connection_id: usize,
    object: &Object,
    send_status: bool,
    object_received_time: u128,
  ) {
    let group_id = object.location.group;
    let object_id = object.location.object;
    let subgroup_id = object.subgroup_id.unwrap_or(0);
    let payload_size = object.payload.as_ref().map(|p| p.len()).unwrap_or(0);

    let log_filename = format!("sub_{}_{}.log", track_alias, subscriber_connection_id);

    let log_entry = format!(
      "{},{},{},{},{},{}\n",
      group_id, subgroup_id, object_id, payload_size, send_status, object_received_time
    );

    self.write_log_entry(&log_filename, &log_entry).await;
  }

  /// Log object properties for track-level logging
  /// Filename format: <track_alias>.log
  /// Fields: group_id, subgroup_id, object_id, payload_len, object_received_time
  pub async fn log_track_object(
    &self,
    track_alias: u64,
    object: &Object,
    object_received_time: u128,
  ) {
    let group_id = object.location.group;
    let object_id = object.location.object;
    let subgroup_id = object.subgroup_id.unwrap_or(0);
    let payload_len = object.payload.as_ref().map(|p| p.len()).unwrap_or(0);

    let log_filename = format!("track_{}.log", track_alias);

    let log_entry = format!(
      "{},{},{},{},{}\n",
      group_id, subgroup_id, object_id, payload_len, object_received_time
    );

    self.write_log_entry(&log_filename, &log_entry).await;
  }

  /// Log object properties for fetch stream logging
  /// Filename format: fetch_<track_alias>_<request_id>.log
  /// Fields: group_id, subgroup_id, object_id, payload_len, send_status, sending_time
  pub async fn log_fetch_object(
    &self,
    track_alias: u64,
    subscriber_connection_id: usize,
    request_id: u64,
    object: &Object,
    send_status: bool,
    sending_time: u128,
  ) {
    let group_id = object.location.group;
    let object_id = object.location.object;
    let subgroup_id = object.subgroup_id.unwrap_or(0);
    let payload_len = object.payload.as_ref().map(|p| p.len()).unwrap_or(0);

    let log_filename = format!(
      "fetch_{}_{}_{}.log",
      track_alias, subscriber_connection_id, request_id
    );

    let log_entry = format!(
      "{},{},{},{},{},{}\n",
      group_id, subgroup_id, object_id, payload_len, send_status, sending_time
    );

    self.write_log_entry(&log_filename, &log_entry).await;
  }

  /// Helper function to write log entries to files
  async fn write_log_entry(&self, log_filename: &str, log_entry: &str) {
    let log_path = Path::new(&self.log_folder).join(log_filename);

    // Create logs directory if it doesn't exist
    if let Err(e) = tokio::fs::create_dir_all(&self.log_folder).await {
      error!(
        "Failed to create log directory {}: {:?}",
        self.log_folder, e
      );
      return;
    }

    // Append to log file
    match OpenOptions::new()
      .create(true)
      .append(true)
      .open(&log_path)
      .await
    {
      Ok(mut file) => {
        if let Err(e) = file.write_all(log_entry.as_bytes()).await {
          error!("Failed to write to log file {:?}: {:?}", log_path, e);
        }
      }
      Err(e) => {
        error!("Failed to open log file {:?}: {:?}", log_path, e);
      }
    }
  }
}
