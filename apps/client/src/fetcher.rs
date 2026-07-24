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
use moqtail::model::common::tuple::{Tuple, TupleField};
use moqtail::model::control::control_message::ControlMessage;
use moqtail::model::control::fetch::{Fetch, StandaloneFetchProps};
use moqtail::model::error::StreamResetCode;
use moqtail::model::parameter::message_parameter::MessageParameter;
use moqtail::model::property::object_property::ObjectProperty;
use moqtail::transport::control_stream_handler::ControlStreamHandler;
use moqtail::transport::data_stream_handler::{FetchRequest, RecvDataStream};
use std::collections::BTreeMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{error, info};

/// Render any Prior Group/Object ID Gap properties for logging.
fn format_prior_gaps(properties: &Option<Vec<ObjectProperty>>) -> String {
  let Some(props) = properties else {
    return String::new();
  };
  let mut out = String::new();
  for p in props {
    match p {
      ObjectProperty::PriorGroupIdGap { gap } => out.push_str(&format!(" [prior_group_gap={gap}]")),
      ObjectProperty::PriorObjectIdGap { gap } => {
        out.push_str(&format!(" [prior_object_gap={gap}]"))
      }
      _ => {}
    }
  }
  out
}

enum FetchReceiveSignal {
  // The configured cancel_after object threshold was reached.
  CancelAfter,
  // The FETCH response was detected as a malformed track.
  MalformedTrack,
}

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
  // Keep `moq` alive for the whole function: its control stream carries only
  // SETUP now, but must stay open for the session's lifetime. Requests use their
  // own bidi streams; data arrives on uni streams.
  let connection = moq.connection.clone();

  let pending_fetches = Arc::new(RwLock::new(BTreeMap::new()));

  // Send Fetch request
  let request_id = 0u64;
  let ns = Tuple::from_utf8_path(&config.namespace);
  let standalone_fetch_props = StandaloneFetchProps {
    track_namespace: ns,
    track_name: TupleField::from_utf8(&config.track_name),
    start_location: Location::new(config.start_group, config.start_object),
    end_location: Location::new(config.end_group, config.end_object),
  };

  let parameters = vec![MessageParameter::new_subscriber_priority(200)];

  let fetch = Fetch::new_standalone(request_id, standalone_fetch_props, parameters);

  info!(
    "Sending Fetch: groups {}:{} to {}:{}",
    config.start_group, config.start_object, config.end_group, config.end_object
  );

  // FETCH opens its own bidi request stream; the response returns on it.
  let (send, recv) = connection.open_bi().await?;
  let mut request_stream = ControlStreamHandler::new(send, recv);
  request_stream
    .send(&ControlMessage::Fetch(Box::new(fetch.clone())))
    .await
    .map_err(|e| anyhow::anyhow!("Failed to send FETCH: {:?}", e))?;

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
  let (signal_tx, mut signal_rx) = tokio::sync::mpsc::unbounded_channel::<FetchReceiveSignal>();

  let cancel_after = config.cancel_after;
  let receive_task = tokio::spawn(async move {
    let mut total_objects = 0u64;
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
                  "Fetched object: group={}, object={}{}",
                  obj.location.group,
                  obj.location.object,
                  format_prior_gaps(&obj.properties)
                );
                total_objects += 1;
                handler = next_handler;

                // Signal cancellation if threshold reached
                if cancel_after > 0 && total_objects >= cancel_after {
                  info!("Received {} objects, signaling FETCH_CANCEL", total_objects);
                  let _ = signal_tx.send(FetchReceiveSignal::CancelAfter);
                  return;
                }
              }
              None => {
                if stream_handler.is_malformed_track() {
                  error!("Fetch response is malformed; signaling FETCH_CANCEL");
                  let _ = signal_tx.send(FetchReceiveSignal::MalformedTrack);
                  return;
                }
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

  // Wait for the fetch response on the request stream (FetchOk / RequestError).
  match request_stream.next_message().await {
    Ok(ControlMessage::RequestOk(m)) => {
      info!("Received RequestOk: {:?}", m);
    }
    Ok(ControlMessage::RequestError(m)) => {
      error!("Received RequestError: {:?}", m);
    }
    Ok(m) => {
      info!("Received fetch response: {:?}", m);
    }
    Err(e) => {
      error!("Error receiving fetch response: {:?}", e);
    }
  }

  let receive_signal = if cancel_after > 0 {
    signal_rx.recv().await
  } else {
    tokio::time::timeout(tokio::time::Duration::from_secs(5), signal_rx.recv())
      .await
      .ok()
      .flatten()
  };

  if let Some(signal) = receive_signal {
    match signal {
      FetchReceiveSignal::CancelAfter => {
        info!("Received cancel_after threshold, sending FETCH_CANCEL");
      }
      FetchReceiveSignal::MalformedTrack => {
        error!("Received malformed FETCH response, sending FETCH_CANCEL");
      }
    }

    info!("Cancelling fetch by resetting the request stream");
    request_stream.reset_and_stop(StreamResetCode::Cancelled.to_u64());
  }

  // Wait for fetch data to arrive
  if cancel_after > 0 {
    tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
  }

  info!("Closing connection...");
  connection.close(0u32, b"Done");

  let _ = receive_task.await;
  info!("Fetch complete");

  Ok(())
}
