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

use anyhow::Result;
use clap::Parser;
use moqtail::model::{
  common::{location::Location, tuple::Tuple},
  control::{
    client_setup::ClientSetup, constant, control_message::ControlMessage,
    subscribe::Subscribe,
  },
  data::datagram_object::DatagramObject,
};
use moqtail::transport::control_stream_handler::ControlStreamHandler;
use std::time::Instant;
use tracing::{debug, error, info};
use wtransport::ClientConfig;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Cli {
  /// Server address
  #[arg(long, default_value = "https://localhost:4433")]
  server: String,

  /// Track namespace
  #[arg(short, long, default_value = "test")]
  namespace: String,

  /// Track name
  #[arg(short, long, default_value = "video")]
  track_name: String,

  /// Duration to listen in seconds
  #[arg(short, long, default_value_t = 15)]
  duration: u64,
}

#[tokio::main]
async fn main() -> Result<()> {
  // Initialize tracing
  tracing_subscriber::fmt()
    .with_env_filter(
      tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
    )
    .init();

  let cli = Cli::parse();

  info!(
    "Test Subscriber starting - server: {}, namespace: {}, track: {}, duration: {}s",
    cli.server, cli.namespace, cli.track_name, cli.duration
  );

  // Create client config with relaxed certificate validation for testing
  let config = ClientConfig::builder()
    .with_bind_default()
    .with_native_certs()
    .build();

  info!("Connecting to relay server at {}", cli.server);
  let connection = wtransport::Endpoint::client(config)?
    .connect(&cli.server)
    .await?;

  info!("Connected! Connection ID: {}", connection.stable_id());

  // Open bidirectional stream for control messages
  info!("Opening control stream...");
  let (send_stream, recv_stream) = connection.open_bi().await?.await?;
  let mut control_stream = ControlStreamHandler::new(send_stream, recv_stream);

  // Send ClientSetup
  info!("Sending ClientSetup...");
  let client_setup = ClientSetup::new(vec![constant::DRAFT_14], vec![]);
  control_stream
    .send(&ControlMessage::ClientSetup(Box::new(client_setup)))
    .await?;

  // Receive ServerSetup
  info!("Waiting for ServerSetup...");
  match control_stream.next_message().await? {
    ControlMessage::ServerSetup(msg) => {
      info!("Received ServerSetup, version: {}", msg.selected_version);
    }
    msg => {
      error!("Unexpected message: {:?}", msg);
      anyhow::bail!("Expected ServerSetup, got {:?}", msg);
    }
  };

  // Subscribe to track directly (no namespace subscription needed)
  info!("Subscribing to track: {}/{}", cli.namespace, cli.track_name);
  let namespace = Tuple::from_utf8_path(&cli.namespace);
  let subscribe = Subscribe::new_absolute_start(
    1, // request_id
    namespace.clone(),
    cli.track_name.clone(),
    0, // subscriber_priority
    constant::GroupOrder::Ascending,
    true, // forward
    Location::new(0, 0), // start_location
    vec![],
  );
  control_stream
    .send(&ControlMessage::Subscribe(Box::new(subscribe)))
    .await?;

  // Wait for SubscribeOk
  info!("Waiting for SubscribeOk...");
  let track_alias = match control_stream.next_message().await? {
    ControlMessage::SubscribeOk(msg) => {
      info!(
        "Track subscription accepted, track_alias: {}, expires: {}",
        msg.track_alias, msg.content_exists
      );
      msg.track_alias
    }
    msg => {
      error!("Unexpected message: {:?}", msg);
      anyhow::bail!("Expected SubscribeOk, got {:?}", msg);
    }
  };

  info!(
    "Successfully subscribed! Listening for datagrams for {} seconds...",
    cli.duration
  );

  // Spawn task to receive datagrams
  let connection_clone = connection.clone();
  let datagram_task = tokio::spawn(async move {
    let mut count = 0u64;
    let start = Instant::now();
    let mut last_group = 0u64;
    let mut last_object = 0u64;
    let mut errors = 0u64;

    loop {
      match connection_clone.receive_datagram().await {
        Ok(datagram) => {
          let bytes = bytes::Bytes::from(datagram.payload().to_vec());
          let mut bytes_mut = bytes.clone();

          match DatagramObject::deserialize(&mut bytes_mut) {
            Ok(datagram_obj) => {
              count += 1;
              let elapsed = start.elapsed().as_millis();

              if datagram_obj.track_alias == track_alias {
                // Verify payload size
                let payload_size = datagram_obj.payload.len();
                
                // Sanity check: group and object IDs should be reasonable
                let parse_ok = datagram_obj.group_id < 10000 && datagram_obj.object_id < 10000;
                if !parse_ok {
                  error!(
                    "PARSE ERROR in datagram {}: track_alias={}, group={}, object={}, size={} bytes - INVALID VALUES! Raw bytes len={}",
                    count,
                    datagram_obj.track_alias,
                    datagram_obj.group_id,
                    datagram_obj.object_id,
                    payload_size,
                    bytes.len()
                  );
                  errors += 1;
                  continue;
                }
                
                // Check for expected sequence
                let expected_object = if datagram_obj.group_id == last_group {
                  last_object + 1
                } else if datagram_obj.group_id == last_group + 1 && datagram_obj.object_id == 0 {
                  0
                } else {
                  datagram_obj.object_id // Don't validate on first or after gap
                };
                
                let sequence_ok = if count == 1 {
                  true // First datagram
                } else if datagram_obj.group_id == last_group && datagram_obj.object_id == expected_object {
                  true
                } else if datagram_obj.group_id == last_group + 1 && datagram_obj.object_id == 0 {
                  true // New group
                } else {
                  errors += 1;
                  false
                };

                if count % 10 == 0 || count <= 5 || !sequence_ok {
                  info!(
                    "Received datagram {}: track_alias={}, group={}, object={}, size={} bytes, elapsed={}ms, seq={}",
                    count,
                    datagram_obj.track_alias,
                    datagram_obj.group_id,
                    datagram_obj.object_id,
                    payload_size,
                    elapsed,
                    if sequence_ok { "OK" } else { "GAP/ERROR" }
                  );
                } else {
                  debug!(
                    "Received datagram {}: group={}, object={}, seq=OK",
                    count, datagram_obj.group_id, datagram_obj.object_id
                  );
                }

                last_group = datagram_obj.group_id;
                last_object = datagram_obj.object_id;
              } else {
                debug!(
                  "Received datagram for different track_alias: {}",
                  datagram_obj.track_alias
                );
              }
            }
            Err(e) => {
              error!("Failed to parse datagram: {:?}", e);
              errors += 1;
            }
          }
        }
        Err(e) => {
          info!("Datagram receive error: {:?}", e);
          break;
        }
      }
    }

    info!(
      "Datagram reception ended. Total received: {}, last: group={}, object={}, errors/gaps: {}",
      count, last_group, last_object, errors
    );
    (count, errors)
  });

  // Wait for specified duration
  tokio::time::sleep(tokio::time::Duration::from_secs(cli.duration)).await;

  info!("Duration elapsed, closing connection...");
  connection.close(0u32.into(), b"Done");

  // Wait for datagram task to finish
  let (total_count, total_errors) = datagram_task.await?;
  info!(
    "Test complete! Total datagrams received: {}, Parsing errors/gaps: {}",
    total_count, total_errors
  );

  if total_errors == 0 {
    info!(" All datagrams parsed successfully with correct sequence!");
  } else {
    info!("  Found {} parsing errors or sequence gaps", total_errors);
  }

  Ok(())
}
