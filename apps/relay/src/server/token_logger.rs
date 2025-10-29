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

use bytes::Bytes;
use std::fs::OpenOptions;
use std::io::Write;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::error;

/// Log token information to a CSV file
///
/// This function logs the token value, connection ID, client port number, and timestamp
/// to a comma-delimited file when an authorization token with Register type is encountered.
///
/// # Arguments
/// * `token_value` - The token value as bytes
/// * `connection_id` - The connection ID
/// * `client_port` - The client's port number
/// * `log_path` - The path to the log file
///
pub fn log_token_registration(
  token_value: &Bytes,
  connection_id: usize,
  client_port: u16,
  log_path: &str,
) {
  // Get current timestamp
  let timestamp = SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .map(|duration| duration.as_secs())
    .unwrap_or(0);

  // Convert token value to hex string for safe CSV output
  let token_hex = hex::encode(token_value);

  // Format the log entry
  let log_entry = format!(
    "{},{},{},{}\n",
    token_hex, connection_id, client_port, timestamp
  );

  // Write to file
  match OpenOptions::new().create(true).append(true).open(log_path) {
    Ok(mut file) => {
      if let Err(e) = file.write_all(log_entry.as_bytes()) {
        error!("Failed to write token log entry: {}", e);
      }
    }
    Err(e) => {
      error!("Failed to open token log file {}: {}", log_path, e);
    }
  }
}
