// Copyright 2026 The MOQtail Authors
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

//! `/api/v1/events`: every connection's qlog events as they happen, as
//! Server-Sent Events.
//!
//! The events come from the qlog live feed, served on the API's runtime, so no
//! event traffic goes through the relay's delivery path. Without `qlog_folder` the
//! events are built only while someone listens.

use crate::ApiState;
use crate::error::ApiError;
use crate::list::parse_query;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use moqtail::transport::qlog::{self, LiveEvent};
use serde::Deserialize;
use std::convert::Infallible;
use std::sync::Arc;
use tokio::sync::broadcast::error::RecvError;

#[derive(Debug, Default, Deserialize)]
pub struct EventsQuery {
  /// Only this connection's events.
  conn: Option<u64>,
  /// Only events whose name starts with this, such as `moqt:control_message`.
  name: Option<String>,
}

pub async fn events(
  State(_state): State<Arc<ApiState>>,
  query: Result<Query<EventsQuery>, QueryRejection>,
) -> Result<Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>>, ApiError> {
  let query = parse_query(query)?;
  let live = qlog::subscribe().ok_or_else(|| {
    ApiError::new(StatusCode::CONFLICT, "events_off").detail("live events are not running")
  })?;
  let stream = futures_util::stream::unfold((live, query), |(mut live, query)| async move {
    loop {
      let event = match live.recv().await {
        Ok(LiveEvent::Event { conn, json }) => {
          if query.conn.is_some_and(|c| c != conn as u64) {
            continue;
          }
          let Some(data) = with_conn_id(conn, &json) else {
            continue;
          };
          if let Some(prefix) = &query.name
            && !event_name(&json).is_some_and(|n| n.starts_with(prefix.as_str()))
          {
            continue;
          }
          Event::default().event("qlog").data(data)
        }
        Ok(LiveEvent::Closed(conn)) => {
          if query.conn.is_some_and(|c| c != conn as u64) {
            continue;
          }
          Event::default()
            .event("closed")
            .data(format!("{{\"conn_id\":{conn}}}"))
        }
        // A listener that falls behind is told how much it missed, and goes on.
        Err(RecvError::Lagged(missed)) => Event::default()
          .event("lagged")
          .data(format!("{{\"missed\":{missed}}}")),
        Err(RecvError::Closed) => return None,
      };
      return Some((Ok(event), (live, query)));
    }
  });
  Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

/// The event's JSON with the connection id added first:
/// `{"conn_id":7,"time":...,"name":...,"data":...}`.
fn with_conn_id(conn: usize, json: &[u8]) -> Option<String> {
  let text = std::str::from_utf8(json).ok()?;
  let rest = text.strip_prefix('{')?;
  Some(format!("{{\"conn_id\":{conn},{rest}"))
}

fn event_name(json: &[u8]) -> Option<String> {
  #[derive(Deserialize)]
  struct Named {
    name: String,
  }
  serde_json::from_slice::<Named>(json).ok().map(|n| n.name)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn adds_the_connection_id_and_reads_the_name() {
    let json = br#"{"time":1.0,"name":"moqt:control_message_parsed","data":{}}"#;
    assert_eq!(
      with_conn_id(7, json).unwrap(),
      r#"{"conn_id":7,"time":1.0,"name":"moqt:control_message_parsed","data":{}}"#
    );
    assert_eq!(event_name(json).unwrap(), "moqt:control_message_parsed");
  }
}
