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
use bytes::Bytes;
use clap::Parser;
use moqtail::model::{
  common::{location::Location, tuple::Tuple},
  control::{
    client_setup::ClientSetup, constant, control_message::ControlMessage,
    publish::Publish, publish_namespace::PublishNamespace,
  },
  data::{
    datagram_object::DatagramObject,
    subgroup_header::SubgroupHeader,
    subgroup_object::SubgroupObject,
  },
};
use moqtail::transport::control_stream_handler::ControlStreamHandler;
use std::time::Duration;
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

  /// Number of datagrams to send
  #[arg(short, long, default_value_t = 100)]
  count: u64,

  /// Interval between datagrams in milliseconds
  #[arg(short, long, default_value_t = 1000)]
  interval: u64,

  
  #[arg(long, default_value_t = true)]
  datagram: bool,
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
    "Test Client starting - server: {}, namespace: {}, track: {}, count: {}, interval: {}ms, datagram: {}",
    cli.server, cli.namespace, cli.track_name, cli.count, cli.interval, cli.datagram
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
  control_stream.send_impl(&client_setup).await?;

  // Receive ServerSetup
  info!("Waiting for ServerSetup...");
  let server_setup = match control_stream.next_message().await? {
    ControlMessage::ServerSetup(msg) => {
      info!("Received ServerSetup: version={}", msg.selected_version);
      msg
    }
    msg => {
      error!("Unexpected message: {:?}", msg);
      anyhow::bail!("Expected ServerSetup, got {:?}", msg);
    }
  };

  debug!("Server setup parameters: {:?}", server_setup.setup_parameters);

  // Publish namespace
  info!("Publishing namespace: {}", cli.namespace);
  let namespace = Tuple::from_utf8_path(&cli.namespace);
  let publish_namespace = PublishNamespace::new(0, namespace.clone(), &[]);
  control_stream
    .send(&ControlMessage::PublishNamespace(Box::new(
      publish_namespace,
    )))
    .await?;

  // Wait for PublishNamespaceOk
  info!("Waiting for PublishNamespaceOk...");
  match control_stream.next_message().await? {
    ControlMessage::PublishNamespaceOk(_) => {
      info!("Namespace published successfully");
    }
    msg => {
      error!("Unexpected message: {:?}", msg);
      anyhow::bail!("Expected PublishNamespaceOk, got {:?}", msg);
    }
  };

  // Publish track
  info!("Publishing track: {}/{}", cli.namespace, cli.track_name);
  let track_alias = 1u64; // Client assigns track alias
  let publish = Publish::new(
    1, // request_id
    namespace.clone(),
    cli.track_name.clone(),
    track_alias,
    constant::GroupOrder::Ascending,
    1, // content_exists
    Some(Location::new(0, 0)), // largest_location
    1, // forward
    vec![],
  );
  control_stream
    .send(&ControlMessage::Publish(Box::new(publish)))
    .await?;

  // Wait for PublishOk
  info!("Waiting for PublishOk...");
  match control_stream.next_message().await? {
    ControlMessage::PublishOk(msg) => {
      info!("Track published successfully, request_id: {}", msg.request_id);
    }
    msg => {
      error!("Unexpected message: {:?}", msg);
      anyhow::bail!("Expected PublishOk, got {:?}", msg);
    }
  };

  if cli.datagram {
    // Send datagrams
    info!("Starting to send {} datagrams...", cli.count);
    send_datagrams(&connection, track_alias, cli.count, cli.interval).await?;
  } else {
    // Send via streams
    info!("Starting to send {} objects via streams...", cli.count);
    send_via_streams(&connection, track_alias, cli.count, cli.interval).await?;
  }

  info!("All datagrams sent successfully!");

  // Keep connection alive for a bit to ensure delivery
  info!("Waiting 2 seconds before closing...");
  tokio::time::sleep(Duration::from_secs(10)).await;

  info!("Closing connection...");
  connection.close(0u32.into(), b"Done");

  Ok(())
}

async fn send_datagrams(
  connection: &wtransport::Connection,
  track_alias: u64,
  count: u64,
  interval_ms: u64,
) -> Result<()> {
  let interval = Duration::from_millis(interval_ms);
  let mut group_id = 0u64;

  for i in 0..count {
    // Create datagram object
    let object_id = i % 30; // 30 objects per group (simulating 30fps video)
    
    if object_id == 0 && i > 0 {
      group_id += 1;
    }

    // Create sample payload (simulating video frame)
    let payload_size = 1200; // Typical MTU size
    let payload: Vec<u8> = (0..payload_size).map(|_| rand::random::<u8>()).collect();

    let datagram_obj = DatagramObject::new(
      track_alias,
      group_id,
      object_id,
      0, // publisher_priority
      Bytes::from(payload),
    );

    // Serialize and send
    let serialized = datagram_obj.serialize()?;
    
    match connection.send_datagram(serialized) {
      Ok(_) => {
        if i % 10 == 0 || i < 5 || i >= count - 5 {
          info!(
            "Sent datagram {}/{}: group={}, object={}, size={} bytes",
            i + 1,
            count,
            group_id,
            object_id,
            payload_size
          );
        } else {
          debug!(
            "Sent datagram {}/{}: group={}, object={}",
            i + 1,
            count,
            group_id,
            object_id
          );
        }
      }
      Err(e) => {
        error!(
          "Failed to send datagram {}/{}: {:?}",
          i + 1,
          count,
          e
        );
      }
    }

    // Wait for interval (except for last datagram)
    if i < count - 1 {
      tokio::time::sleep(interval).await;
    }
  }

  Ok(())
}

async fn send_via_streams(
  connection: &wtransport::Connection,
  track_alias: u64,
  count: u64,
  interval_ms: u64,
) -> Result<()> {
  let interval = Duration::from_millis(interval_ms);
  let mut group_id = 0u64;
  let mut current_stream: Option<wtransport::SendStream> = None;
  let mut objects_in_group = 0u64;

  for i in 0..count {
    let object_id = i % 30; // 30 objects per group
    
    // Start new group when object_id wraps to 0
    if object_id == 0 {
      if i > 0 {
        // Close previous stream
        if let Some(mut stream) = current_stream.take() {
          stream.finish().await?;
          info!("Closed stream for group {}", group_id);
        }
        group_id += 1;
        objects_in_group = 0;
      }
      
      // Open new stream for new group
      let mut stream = connection.open_uni().await?.await?;
      
      // Write subgroup header (using fixed zero subgroup ID)
      let subgroup_header = SubgroupHeader::new_fixed_zero_id(
        track_alias,
        group_id,
        0, // publisher_priority
        false, // has_extensions
        false, // contains_end_of_group
      );
      
      let header_bytes = subgroup_header.serialize()?;
      stream.write_all(&header_bytes).await?;
      current_stream = Some(stream);
      
      info!(
        "Opened new stream for group {}, wrote header ({} bytes)",
        group_id,
        header_bytes.len()
      );
    }

    // Create subgroup object
    let payload_size = 1200;
    let payload: Vec<u8> = (0..payload_size).map(|_| rand::random::<u8>()).collect();

    let subgroup_obj = SubgroupObject {
      object_id,
      extension_headers: None,
      object_status: Some(moqtail::model::data::constant::ObjectStatus::Normal),
      payload: Some(Bytes::from(payload)),
    };

    // Serialize object (with previous_object_id for delta encoding)
    let previous_object_id = if object_id > 0 { Some(object_id - 1) } else { None };
    let object_bytes = subgroup_obj.serialize(previous_object_id, false)?;

    // Write to stream
    if let Some(ref mut stream) = current_stream {
      stream.write_all(&object_bytes).await?;
      objects_in_group += 1;
      
      if i % 10 == 0 || i < 5 || i >= count - 5 {
        info!(
          "Sent object {}/{}: group={}, object={}, size={} bytes (total {} objects in group)",
          i + 1,
          count,
          group_id,
          object_id,
          payload_size,
          objects_in_group
        );
      } else {
        debug!(
          "Sent object {}/{}: group={}, object={}",
          i + 1,
          count,
          group_id,
          object_id
        );
      }
    }

    // Wait for interval (except for last object)
    if i < count - 1 {
      tokio::time::sleep(interval).await;
    }
  }

  // Close final stream
  if let Some(mut stream) = current_stream.take() {
    stream.finish().await?;
    info!("Closed final stream for group {}", group_id);
  }

  Ok(())
}

// Simple random number generator for payload
mod rand {
  use std::cell::Cell;
  
  thread_local! {
    static SEED: Cell<u64> = Cell::new(0x123456789abcdef0);
  }
  
  pub fn random<T: From<u8>>() -> T {
    SEED.with(|seed| {
      let mut s = seed.get();
      s ^= s << 13;
      s ^= s >> 7;
      s ^= s << 17;
      seed.set(s);
      T::from((s & 0xFF) as u8)
    })
  }
}
