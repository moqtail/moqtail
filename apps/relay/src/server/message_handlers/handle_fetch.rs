use crate::server::client::MOQTClient;
use crate::server::session_context::SessionContext;
use crate::server::utils::build_stream_id;
use core::result::Result::{Err, Ok};
use moqtail::model::common::location::Location;
use moqtail::model::control::constant::FetchErrorCode;
use moqtail::model::control::control_message::ControlMessage;
use moqtail::model::control::fetch_error::FetchError;
use moqtail::model::control::fetch_ok::FetchOk;
use moqtail::model::data::fetch_header::FetchHeader;
use moqtail::model::error::TerminationCode;
use moqtail::model::{common::reason_phrase::ReasonPhrase, control::constant::FetchType};
use moqtail::transport::control_stream_handler::ControlStreamHandler;
use moqtail::transport::data_stream_handler::HeaderInfo;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{error, info, warn};

pub async fn handle_fetch_messages(
  client: Arc<RwLock<MOQTClient>>,
  _control_stream_handler: &mut ControlStreamHandler,
  msg: ControlMessage,
  context: Arc<SessionContext>,
) -> Result<(), TerminationCode> {
  match msg {
    ControlMessage::Fetch(m) => {
      info!("received Fetch message: {:?}", m);
      let fetch = *m;
      let request_id = fetch.clone().request_id;

      let fn_ = async {
        if let Some(joining_fetch_props) = fetch.clone().joining_fetch_props {
          let sub_request_id = joining_fetch_props.joining_request_id;
          let sub_requests = context.client_subscribe_requests.read().await;
          // the original request id is the request id of the subscribe request that created the subscription
          let existing_sub = sub_requests
            .iter()
            .find(|e| e.1.original_request_id == sub_request_id);
          if existing_sub.is_none() {
            error!(
              "handle_fetch_messages | Joining fetch request id not found: {:?} {:?}",
              sub_request_id, sub_requests
            );
            // return Err(TerminationCode::InternalError);
            return (None, None, None);
          }
          let existing_sub = existing_sub.unwrap().1;

          let tracks = context.tracks.read().await;
          let track = tracks.get(&existing_sub.subscribe_request.track_alias);

          if let Some(track) = track {
            let largest_location = track.largest_location.read().await;

            // TODO: validate the range
            if largest_location.group < joining_fetch_props.joining_start {
              error!(
                "handle_fetch_messages | Joining fetch start location is larger than the track's largest location: {:?} {:?}",
                largest_location, joining_fetch_props.joining_start
              );
              send_fetch_error(
                client.clone(),
                request_id,
                FetchErrorCode::InvalidRange,
                ReasonPhrase::try_new(String::from("Invalid range")).unwrap(),
              )
              .await;
              return (None, None, None);
            }

            let start_group = if fetch.fetch_type == FetchType::RelativeFetch {
              largest_location.group - joining_fetch_props.joining_start
            } else {
              joining_fetch_props.joining_start
            };

            let start_location = Location::new(start_group, 0);
            let end_location = Location::new(largest_location.group, 0);
            (
              Some(track.clone()),
              Some(start_location),
              Some(end_location),
            )
          } else {
            (None, None, None)
          }
        } else {
          // standalone fetch
          let props = fetch.standalone_fetch_props.clone().unwrap();

          // let's see whether the track is in the cache
          let track = {
            let tracks = context.tracks.read().await;
            tracks
              .iter()
              .find(|e| {
                e.1.track_namespace == props.track_namespace && e.1.track_name == props.track_name
              })
              .map(|track| track.1.clone())
          };

          if let Some(track) = track {
            (
              Some(track),
              Some(props.start_location.clone()),
              Some(props.end_location.clone()),
            )
          } else {
            (None, None, None)
          }
        }
      };

      let (track, start_location, end_location) = fn_.await;

      // TODO: send fetch message to the publisher
      if track.is_none() {
        // TODO: send fetch message to the possible publishers
        // for now just return FETCH_ERROR
        send_fetch_error(
          client.clone(),
          request_id,
          FetchErrorCode::TrackDoesNotExist,
          ReasonPhrase::try_new(String::from("Track does not exist")).unwrap(),
        )
        .await;
        return Ok(());
      }

      let track = track.unwrap();

      // TODO: verify the range exist. Currently we just return what we have...

      info!(
        "handle_fetch_messages | Fetching objects from {:?} to {:?}",
        start_location.clone().unwrap(),
        end_location.clone().unwrap()
      );

      let mut object_rx = track
        .cache
        .read_objects(start_location.unwrap(), end_location.clone().unwrap())
        .await;

      let fetch_header = FetchHeader::new(request_id);
      let header_info = HeaderInfo::Fetch {
        header: fetch_header,
        fetch_request: fetch,
      };

      let the_client = client.read().await;

      let stream_id = build_stream_id(track.track_alias, &header_info);
      let stream_result = the_client
        .open_stream(&stream_id, fetch_header.serialize().unwrap(), 0)
        .await;

      let send_stream = match stream_result {
        Ok(send_stream) => send_stream,
        Err(e) => {
          error!("handle_fetch_messages | Error opening stream: {:?}", e);
          return Err(TerminationCode::InternalError);
        }
      };
      let mut object_count = 0;
      loop {
        match object_rx.recv().await {
          Some(object) => {
            let object_id = object.object_id;
            if let Err(e) = the_client
              .write_object_to_stream(
                &stream_id,
                object_id,
                object.serialize().unwrap(),
                Some(send_stream.clone()),
              )
              .await
            {
              error!(
                "handle_fetch_messages | Error writing object to stream: {:?}",
                e
              );
              return Err(TerminationCode::InternalError);
            }
            object_count += 1;
          }
          None => {
            warn!("handle_fetch_messages | No object.");
            break;
          }
        }
      }
      if object_count == 0 {
        send_fetch_error(
          client.clone(),
          request_id,
          FetchErrorCode::NoObjects,
          ReasonPhrase::try_new(String::from("No objects available")).unwrap(),
        )
        .await;
      } else {
        info!("handle_fetch_messages | Fetched {} objects", object_count);
        if let Err(e) = the_client.close_stream(&stream_id).await {
          error!("handle_fetch_messages | Error closing stream: {:?}", e);
          // return Err(TerminationCode::InternalError);
        }
      }

      // TODO: implement descending fetch
      // TODO: end of track is correct?
      let largest_location = track.largest_location.read().await;
      let end_of_track = largest_location.group == end_location.clone().unwrap().group;
      let fetch_ok =
        FetchOk::new_ascending(request_id, end_of_track, end_location.unwrap(), vec![]);

      the_client
        .queue_message(ControlMessage::FetchOk(Box::new(fetch_ok)))
        .await;
      Ok(())
    }
    ControlMessage::FetchOk(m) => {
      info!("received FetchOk message: {:?}", m);
      let msg = *m;

      // TODO: When the relay sends a fetch request to the publisher,
      // it will wait for Fetch OK. However this is not implemented yet.
      // Here is just a preliminary attempt for this, validating request id
      let requests = context.relay_fetch_requests.read().await;
      if !requests.contains_key(&msg.request_id) {
        error!("handle_fetch_messages | FetchOk | request_id does not exist");
        return Err(TerminationCode::InternalError);
      }

      Ok(())
    }
    _ => {
      // no-op
      Ok(())
    }
  }
}

async fn send_fetch_error(
  client: Arc<RwLock<MOQTClient>>,
  request_id: u64,
  error_code: FetchErrorCode,
  reason_phrase: ReasonPhrase,
) {
  let fetch_error = FetchError::new(request_id, error_code, reason_phrase);
  let client = client.read().await;
  client
    .queue_message(ControlMessage::FetchError(Box::new(fetch_error)))
    .await;
}
