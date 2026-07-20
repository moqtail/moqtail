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

use crate::server::client::MOQTClient;
use crate::server::session::Session;
use crate::server::session_context::SessionContext;
use core::result::Result;
use moqtail::model::error::{RequestErrorCode, TerminationCode};
use moqtail::model::{
  common::reason_phrase::ReasonPhrase, control::control_message::ControlMessage,
  control::request_error::RequestError, control::request_ok::RequestOk,
  control::track_status::TrackStatus, parameter::message_parameter::MessageParameter,
};
use moqtail::transport::control_stream_handler::ControlStreamHandler;
use std::sync::Arc;
use tracing::{debug, error, info, warn};

pub async fn handle(
  stream_handler: &mut ControlStreamHandler,
  msg: ControlMessage,
  context: Arc<SessionContext>,
) -> Result<(), TerminationCode> {
  match msg {
    ControlMessage::TrackStatus(m) => {
      info!("received TrackStatus message: {:?}", m);
      let status_req = *m;
      let track_namespace = status_req.track_namespace.clone();
      let request_id = status_req.request_id;
      let full_track_name = status_req.get_full_track_name();

      // B. Check Local Track Existence
      if let Some(track_arc) = context.track_manager.get_track(&full_track_name).await {
        info!("track found: {:?}", full_track_name);
        let track = track_arc.read().await;
        let largest_location = track.largest_location.read().await;

        let params = vec![MessageParameter::new_largest_object(
          largest_location.clone(),
        )];

        let ok_msg = RequestOk::new(params);
        stream_handler
          .send(&ControlMessage::RequestOk(Box::new(ok_msg)))
          .await
          .unwrap();
        return Ok(());
      }

      // C. Find Upstream Publisher
      // TODO: send to every interested publisher
      let publisher = {
        debug!("Finding publisher for TrackStatus...");
        let m = context.client_manager.read().await;
        match m.get_publisher_by_full_track_name(&full_track_name).await {
          Some(p) => Some(p),
          None => {
            m.get_publisher_by_announced_track_namespace(&track_namespace)
              .await
          }
        }
      };

      let publisher = if let Some(p) = publisher {
        p.clone()
      } else {
        info!("No publisher found for {:?}", track_namespace);
        let err = RequestError::new(
          RequestErrorCode::DoesNotExist,
          0, //TODO: Maybe decide on another retry interval?
          ReasonPhrase::try_new("No publisher found".to_string()).unwrap(),
        );
        stream_handler.send_impl(&err).await.unwrap();
        return Ok(());
      };

      // D. Forward to the publisher on its own bidi stream and read the response
      // there, then deliver it to the downstream requester's request stream.
      let mut new_req = status_req.clone();
      new_req.request_id =
        Session::get_next_relay_request_id(context.relay_next_request_id.clone()).await;

      let downstream = context.get_client().await;
      tokio::spawn(async move {
        forward_track_status_upstream(publisher, downstream, request_id, new_req, context).await;
      });

      Ok(())
    }

    ControlMessage::RequestUpdate(m) => {
      warn!(
        "REQUEST_UPDATE is not valid for a TRACK_STATUS request (id {})",
        m.existing_request_id
      );
      let err = track_status_update_error();
      stream_handler
        .send(&ControlMessage::RequestError(Box::new(err)))
        .await
    }

    _ => Ok(()),
  }
}

/// Forward a TRACK_STATUS to the publisher on its own bidirectional request
/// stream, read the response there, and deliver it to the downstream requester's
/// request stream.
async fn forward_track_status_upstream(
  publisher: Arc<MOQTClient>,
  downstream: Option<Arc<MOQTClient>>,
  downstream_request_id: u64,
  new_req: TrackStatus,
  _context: Arc<SessionContext>,
) {
  let (send, recv) = match publisher.connection.open_bi().await {
    Ok(streams) => streams,
    Err(e) => {
      error!("Failed to open upstream track-status stream: {:?}", e);
      return;
    }
  };
  let mut upstream = ControlStreamHandler::new(send, recv);
  if let Err(e) = upstream
    .send(&ControlMessage::TrackStatus(Box::new(new_req)))
    .await
  {
    error!("Failed to send upstream TRACK_STATUS: {:?}", e);
    return;
  }

  let response = match upstream.next_message().await {
    Ok(m @ ControlMessage::RequestOk(_)) => m,
    Ok(m @ ControlMessage::RequestError(_)) => m,
    Ok(other) => {
      warn!(
        "Unexpected {:?} on upstream track-status stream",
        other.get_type()
      );
      return;
    }
    Err(_) => {
      debug!("Upstream track-status stream closed");
      return;
    }
  };

  if let Some(downstream) = downstream
    && !downstream
      .send_response(downstream_request_id, response)
      .await
  {
    warn!(
      "no request stream for track-status requester {}",
      downstream_request_id
    );
  }
}

fn track_status_update_error() -> RequestError {
  RequestError::new(
    RequestErrorCode::NotSupported,
    0,
    ReasonPhrase::try_new("TRACK_STATUS does not support REQUEST_UPDATE".to_string()).unwrap(),
  )
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn request_update_on_track_status_is_not_supported() {
    let err = track_status_update_error();
    assert_eq!(err.error_code, RequestErrorCode::NotSupported);
  }
}
