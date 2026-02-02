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

use crate::connection::MoqConnection;
use anyhow::Result;
use moqtail::model::common::location::Location;
use moqtail::model::common::pair::KeyValuePair;
use moqtail::model::common::tuple::Tuple;
use moqtail::model::control::constant::GroupOrder;
use moqtail::model::control::control_message::ControlMessage;
use moqtail::model::control::fetch::{Fetch, StandAloneFetchProps};
use moqtail::transport::data_stream_handler::{FetchRequest, RecvDataStream};
use std::collections::BTreeMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{error, info};

pub async fn run(
  moq: MoqConnection,
  namespace: &str,
  track_name: &str,
  start_group: u64,
  start_object: u64,
  end_group: u64,
  end_object: u64,
) -> Result<()> {
  let MoqConnection {
    connection,
    mut control_stream,
  } = moq;

  let pending_fetches = Arc::new(RwLock::new(BTreeMap::new()));

  // Send Fetch request
  let request_id = 0u64;
  let ns = Tuple::from_utf8_path(namespace);
  let standalone_fetch_props = StandAloneFetchProps {
    track_namespace: ns,
    track_name: track_name.to_string(),
    start_location: Location::new(start_group, start_object),
    end_location: Location::new(end_group, end_object),
  };

  let parameters = vec![KeyValuePair::try_new_varint(100, 200)?];

  let fetch = Fetch::new_standalone(
    request_id,
    1, // subscriber_priority
    GroupOrder::Ascending,
    standalone_fetch_props,
    parameters,
  );

  info!(
    "Sending Fetch: groups {}:{} to {}:{}",
    start_group, start_object, end_group, end_object
  );

  control_stream
    .send(&ControlMessage::Fetch(Box::new(fetch.clone())))
    .await?;

  pending_fetches
    .write()
    .await
    .insert(request_id, FetchRequest::new(request_id, 1, fetch, 0));

  // Listen for incoming streams with fetch response
  let conn = connection.clone();
  let pending_fetches_clone = pending_fetches.clone();

  let receive_task = tokio::spawn(async move {
    loop {
      match conn.accept_uni().await {
        Ok(stream) => {
          info!("Accepted unidirectional stream for fetch");
          let stream_handler = RecvDataStream::new(stream, pending_fetches_clone.clone());
          let mut handler = &stream_handler;

          loop {
            let (next_handler, object) = handler.next_object().await;
            match object {
              Some(obj) => {
                info!(
                  "Fetched object: group={}, object={}",
                  obj.location.group, obj.location.object
                );
                handler = next_handler;
              }
              None => {
                info!("Fetch stream closed");
                break;
              }
            }
          }
        }
        Err(e) => {
          info!("Stream accept ended: {:?}", e);
          break;
        }
      }
    }
  });

  // Wait for control messages (FetchOk/FetchError)
  match control_stream.next_message().await {
    Ok(ControlMessage::FetchOk(m)) => {
      info!("Received FetchOk: {:?}", m);
    }
    Ok(ControlMessage::FetchError(m)) => {
      error!("Received FetchError: {:?}", m);
    }
    Ok(m) => {
      info!("Received message: {:?}", m);
    }
    Err(e) => {
      error!("Error receiving control message: {:?}", e);
    }
  }

  // Wait for fetch data to arrive
  tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;

  info!("Closing connection...");
  connection.close(0u32.into(), b"Done");

  let _ = receive_task.await;
  info!("Fetch complete");

  Ok(())
}
