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
use moqtail::model::control::fetch_cancel::FetchCancel;
use moqtail::transport::data_stream_handler::{FetchRequest, RecvDataStream};
use std::collections::BTreeMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{error, info};

pub struct FetchConfig {
  pub namespace: String,
  pub track_name: String,
  pub start_group: u64,
  pub start_object: u64,
  pub end_group: u64,
  pub end_object: u64,
  pub cancel_after: u64,
}

pub async fn run(moq: MoqConnection, config: FetchConfig) -> Result<()> {
  let MoqConnection {
    connection,
    mut control_stream,
  } = moq;

  let pending_fetches = Arc::new(RwLock::new(BTreeMap::new()));

  // Send Fetch request
  let request_id = 0u64;
  let ns = Tuple::from_utf8_path(&config.namespace);
  let standalone_fetch_props = StandAloneFetchProps {
    track_namespace: ns,
    track_name: config.track_name.clone(),
    start_location: Location::new(config.start_group, config.start_object),
    end_location: Location::new(config.end_group, config.end_object),
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
    config.start_group, config.start_object, config.end_group, config.end_object
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

  // If cancel_after is set, create a oneshot channel to signal the main task
  // cancel_after is the number of objects to receive before sending FETCH_CANCEL
  // make sure that there is a publisher sending enough objects to trigger the cancellation
  // in a reasonable time frame otherwise you may get Track Does Not Exist error or
  // not enough objects received before the test ends
  let (cancel_tx, cancel_rx) = if config.cancel_after > 0 {
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    (Some(tx), Some(rx))
  } else {
    (None, None)
  };

  let cancel_after = config.cancel_after;
  let receive_task = tokio::spawn(async move {
    let mut total_objects = 0u64;
    let mut cancel_tx = cancel_tx;
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
                total_objects += 1;
                handler = next_handler;

                // Signal cancellation if threshold reached
                if cancel_after > 0 && total_objects >= cancel_after {
                  info!("Received {} objects, signaling FETCH_CANCEL", total_objects);
                  if let Some(tx) = cancel_tx.take() {
                    let _ = tx.send(());
                  }
                  break;
                }
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

  // If cancel_after is set, wait for the receive task to signal, then send FETCH_CANCEL
  if let Some(cancel_rx) = cancel_rx {
    match cancel_rx.await {
      Ok(()) => {
        info!("Sending FETCH_CANCEL for request_id: {}", request_id);
        let fetch_cancel = FetchCancel::new(request_id);
        if let Err(e) = control_stream
          .send(&ControlMessage::FetchCancel(Box::new(fetch_cancel)))
          .await
        {
          error!("Error sending FETCH_CANCEL: {:?}", e);
        } else {
          info!("FETCH_CANCEL sent successfully");
        }
      }
      Err(_) => {
        error!("Cancel signal channel closed unexpectedly");
      }
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
