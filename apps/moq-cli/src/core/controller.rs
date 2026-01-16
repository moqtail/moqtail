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

use anyhow::{Context, Result};
use moqtail::model::common::tuple::Tuple;
use moqtail::model::control::client_setup::ClientSetup;
use moqtail::model::control::constant::{self, GroupOrder};
use moqtail::model::control::control_message::ControlMessage;
use moqtail::model::control::subscribe::Subscribe;
use moqtail::model::control::unsubscribe::Unsubscribe;
use moqtail::transport::control_stream_handler::ControlStreamHandler;
use moqtail::transport::data_stream_handler::RecvDataStream;
use std::collections::BTreeMap;
use std::sync::Arc;
use tokio::sync::{Mutex, mpsc};
use wtransport::{ClientConfig, Endpoint};

use crate::ipc::messages::{LogParams, RpcNotification, RpcRequest, RpcResponse, StatParams};

pub struct ClientController {
  event_tx: mpsc::Sender<serde_json::Value>,
  connection: Option<Arc<wtransport::Connection>>,
  control_handler: Option<Arc<Mutex<ControlStreamHandler>>>,
  next_subscribe_id: u64,
}

impl ClientController {
  pub fn new(event_tx: mpsc::Sender<serde_json::Value>) -> Self {
    Self {
      event_tx,
      connection: None,
      control_handler: None,
      next_subscribe_id: 0,
    }
  }

  async fn log(&self, level: &str, msg: String) {
    let note = RpcNotification::Log {
      params: LogParams {
        level: level.to_string(),
        message: msg,
      },
    };
    // Ignore send errors (e.g., if IPC is closed)
    let _ = self
      .event_tx
      .send(serde_json::to_value(note).unwrap())
      .await;
  }

  pub async fn run(&mut self, mut cmd_rx: mpsc::Receiver<RpcRequest>) {
    // The Main Loop now ONLY handles commands.
    // Network packets are handled by background tasks spawned in do_connect.
    while let Some(req) = cmd_rx.recv().await {
      self.handle_request(req).await;
    }
  }

  async fn handle_request(&mut self, req: RpcRequest) {
    let res = match req {
      RpcRequest::Connect { params, id } => match self.do_connect(&params.url).await {
        Ok(_) => RpcResponse::success(id, serde_json::json!({"status": "connected"})),
        Err(e) => RpcResponse::success(id, serde_json::json!({"error": e.to_string()})),
      },
      RpcRequest::Subscribe { params, id } => {
        match self.do_subscribe(&params.namespace, &params.track).await {
          Ok(sub_id) => RpcResponse::success(id, serde_json::json!({"subscription_id": sub_id})),
          Err(e) => RpcResponse::success(id, serde_json::json!({"error": e.to_string()})),
        }
      }
      RpcRequest::Unsubscribe { params, id } => {
        match self.do_unsubscribe(params.subscription_id).await {
          Ok(_) => RpcResponse::success(id, serde_json::json!({"status": "unsubscribed"})),
          Err(e) => RpcResponse::success(id, serde_json::json!({"error": e.to_string()})),
        }
      } // Add other commands here
    };
    let _ = self.event_tx.send(serde_json::to_value(res).unwrap()).await;
  }

  async fn do_connect(&mut self, url: &str) -> Result<()> {
    self.log("info", format!("Connecting to {}", url)).await;

    let config = ClientConfig::builder()
      .with_bind_default()
      .with_no_cert_validation()
      .build();
    let endpoint = Endpoint::client(config)?;
    let connection = Arc::new(endpoint.connect(url).await?);

    // 1. Handshake (Control Stream)
    let (send, recv) = connection.open_bi().await?.await?;
    let mut handler = ControlStreamHandler::new(send, recv);

    let client_setup = ClientSetup::new([constant::DRAFT_14].to_vec(), vec![]);
    handler.send_impl(&client_setup).await?;

    // Wait for ServerSetup
    match handler.next_message().await? {
      ControlMessage::ServerSetup(_) => {
        self.log("info", "Handshake Complete".to_string()).await;
      }
      msg => return Err(anyhow::anyhow!("Expected ServerSetup, got {:?}", msg)),
    }

    // 2. Store State
    self.connection = Some(connection.clone());
    self.control_handler = Some(Arc::new(Mutex::new(handler)));

    // 3. SPAWN THE DATA LISTENER IMMEDIATELY
    // This runs in the background for the lifetime of the connection
    let evt_tx_clone = self.event_tx.clone();
    let conn_clone = connection.clone();

    tokio::spawn(async move {
      Self::background_data_listener(conn_clone, evt_tx_clone).await;
    });

    Ok(())
  }

  // This runs in a separate task, accepting unidirectional streams (Data)
  async fn background_data_listener(
    conn: Arc<wtransport::Connection>,
    event_tx: mpsc::Sender<serde_json::Value>,
  ) {
    loop {
      // Accept new streams (blocking only this task, not the main loop)
      match conn.accept_uni().await {
        Ok(stream) => {
          // Spawn a task for THIS specific track/object stream
          let tx = event_tx.clone();
          tokio::spawn(async move {
            let pending = Arc::new(tokio::sync::RwLock::new(BTreeMap::new()));
            let stream_handler = RecvDataStream::new(stream, pending);

            // Read objects from this stream
            while let (_, Some(object)) = stream_handler.next_object().await {
              let len = object.payload.as_ref().map(|b| b.len()).unwrap_or(0);

              let stat = RpcNotification::OnStatUpdate {
                params: StatParams {
                  object_size: len,
                  group_id: object.subgroup_id.expect(""),
                  object_id: object.location.object,
                },
              };
              // Send stats to Python
              let _ = tx.send(serde_json::to_value(stat).unwrap()).await;
            }
          });
        }
        Err(e) => {
          // Connection closed or failed
          let _ = event_tx
            .send(serde_json::json!({
                "method": "log",
                "params": { "level": "error", "message": format!("Listener died: {}", e) }
            }))
            .await;
          break;
        }
      }
    }
  }

  async fn do_subscribe(&mut self, ns: &str, track: &str) -> Result<u64> {
    // Reuse the EXISTING control stream
    let handler_arc = self.control_handler.as_ref().context("Not connected")?;

    // Lock the mutex just long enough to send the message
    let mut handler = handler_arc.lock().await;

    let sub_id = self.next_subscribe_id;
    self.next_subscribe_id += 1;

    let sub = Subscribe::new_latest_object(
      sub_id,
      Tuple::from_utf8_path(ns),
      track.to_string(),
      1,
      GroupOrder::Ascending,
      true,
      vec![],
    );

    // This uses the EXISTING stream inside the handler
    handler.send_impl(&sub).await?;

    self
      .log("info", format!("Sent Subscribe ID {}", sub_id))
      .await;
    Ok(sub_id)
  }

  // Inside ClientController implementation
  async fn do_unsubscribe(&mut self, sub_id: u64) -> Result<()> {
    let handler_arc = self.control_handler.as_ref().context("Not connected")?;

    // Lock the control stream
    let mut handler = handler_arc.lock().await;

    // Create the MoQ Unsubscribe message
    // The library expects the Subscription ID to identify what to stop
    let msg = Unsubscribe::new(sub_id);

    handler.send_impl(&msg).await?;

    self
      .log("info", format!("Sent Unsubscribe for ID {}", sub_id))
      .await;
    Ok(())
  }
}
