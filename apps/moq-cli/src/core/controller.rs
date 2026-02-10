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
use bytes::Bytes;
use moqtail::model::common::location::Location;
use moqtail::model::common::reason_phrase::ReasonPhrase;
use moqtail::model::common::tuple::Tuple;
use moqtail::model::control::client_setup::ClientSetup;
use moqtail::model::control::constant::{self, FilterType, GroupOrder};
use moqtail::model::control::control_message::ControlMessage;
use moqtail::model::control::fetch::{Fetch, StandAloneFetchProps};

use moqtail::model::control::publish::Publish;
use moqtail::model::control::publish_namespace::PublishNamespace;
use moqtail::model::control::publish_ok::PublishOk;
use moqtail::model::control::subscribe::Subscribe;
use moqtail::model::control::subscribe_error::SubscribeError;
use moqtail::model::control::subscribe_namespace::SubscribeNamespace;
use moqtail::model::control::subscribe_namespace_ok::SubscribeNamespaceOk;
use moqtail::model::control::subscribe_ok::SubscribeOk;
use moqtail::model::control::unsubscribe::Unsubscribe;
// use moqtail::model::control::namespace::Namespace;

use moqtail::model::data::object::Object;
use moqtail::model::data::subgroup_header::SubgroupHeader;
use moqtail::model::data::subgroup_object::SubgroupObject;

use moqtail::transport::control_stream_handler::ControlStreamHandler;
use moqtail::transport::data_stream_handler::{HeaderInfo, RecvDataStream, SendDataStream};

use std::collections::HashSet;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use tokio::sync::{Mutex, mpsc};
use tokio::task::JoinHandle;
use wtransport::{ClientConfig, Endpoint};

use crate::ipc::messages::{
  FetchParams, LogParams, PeerPublishParams, PublishParams, RpcNotification, RpcRequest,
  RpcResponse, StatParams, SubscribeNamespaceParams, SubscribeParams,
};

enum ControlAction {
  SendSubscribe(Subscribe),
  SendUnsubscribe(Unsubscribe),
  SendAnnounce(PublishNamespace),
  SendFetch(Fetch),
  SendPublish(Publish),
  SendSubscribeNamespace(SubscribeNamespace),
}

pub struct ClientController {
  event_tx: mpsc::Sender<serde_json::Value>,
  action_tx: Option<mpsc::Sender<ControlAction>>,
  next_subscribe_id: u64,
  connection: Option<Arc<wtransport::Connection>>,
  announced_namespaces: Arc<Mutex<HashSet<String>>>,
}

impl ClientController {
  pub fn new(event_tx: mpsc::Sender<serde_json::Value>) -> Self {
    Self {
      event_tx,
      action_tx: None,
      next_subscribe_id: 0,
      connection: None,
      announced_namespaces: Arc::new(Mutex::new(HashSet::new())),
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
      RpcRequest::Disconnect { id } => {
        // TODO: Implement graceful disconnect
        RpcResponse::success(id, serde_json::json!({"status": "disconnected"}))
      }
      RpcRequest::Subscribe { params, id } => {
        // Pass the full params to do_subscribe now
        match self.do_subscribe(params).await {
          Ok(sub_id) => RpcResponse::success(id, serde_json::json!({"subscription_id": sub_id})),
          Err(e) => RpcResponse::success(id, serde_json::json!({"error": e.to_string()})),
        }
      }
      RpcRequest::UpdateSubscription { params: _, id } => {
        // TODO: Implement UpdateSubscription
        RpcResponse::success(id, serde_json::json!({"error": "Not implemented yet"}))
      }
      RpcRequest::Unsubscribe { params, id } => {
        match self.do_unsubscribe(params.subscription_id).await {
          Ok(_) => RpcResponse::success(id, serde_json::json!({"status": "unsubscribed"})),
          Err(e) => RpcResponse::success(id, serde_json::json!({"error": e.to_string()})),
        }
      }
      RpcRequest::Fetch { params, id } => match self.do_fetch(params).await {
        Ok(fetch_id) => RpcResponse::success(
          id,
          serde_json::json!({
              "status": "fetching",
              "fetch_id": fetch_id
          }),
        ),
        Err(e) => RpcResponse::success(id, serde_json::json!({"error": e.to_string()})),
      },
      RpcRequest::FetchCancel { params: _, id } => {
        // TODO: Implement FetchCancel
        RpcResponse::success(id, serde_json::json!({"status": "cancelled"}))
      }
      // RENAMED: Announce -> PublishNamespace
      RpcRequest::PublishNamespace { params, id } => {
        match self.do_publish_namespace(&params.namespace).await {
          Ok(_) => RpcResponse::success(id, serde_json::json!({"status": "namespace_published"})),
          Err(e) => RpcResponse::success(id, serde_json::json!({"error": e.to_string()})),
        }
      }
      RpcRequest::Publish { params, id } => match self.do_publish(params).await {
        Ok(pub_id) => RpcResponse::success(
          id,
          serde_json::json!({"status": "publishing", "publish_id": pub_id}),
        ),
        Err(e) => RpcResponse::success(id, serde_json::json!({"error": e.to_string()})),
      },
      RpcRequest::SubscribeNamespace { params, id } => {
        match self.do_subscribe_namespace(params).await {
          Ok(req_id) => RpcResponse::success(
            id,
            serde_json::json!({"status": "subscribed_to_namespace", "request_id": req_id}),
          ),
          Err(e) => RpcResponse::success(id, serde_json::json!({"error": e.to_string()})),
        }
      }
      RpcRequest::UnpublishNamespace { params: _, id } => {
        // TODO: Implement UnpublishNamespace
        RpcResponse::success(id, serde_json::json!({"status": "namespace_unpublished"}))
      }
      RpcRequest::TrackStatus { params: _, id } => {
        // TODO: Implement TrackStatus
        RpcResponse::success(id, serde_json::json!({"status": "track_status_requested"}))
      }
    };
    let _ = self.event_tx.send(serde_json::to_value(res).unwrap()).await;
  }

  async fn control_actor_loop(
    mut handler: ControlStreamHandler,
    mut action_rx: mpsc::Receiver<ControlAction>,
    event_tx: mpsc::Sender<serde_json::Value>,
    connection: Arc<wtransport::Connection>,
    known_namespaces: Arc<Mutex<HashSet<String>>>,
  ) {
    let mut active_tasks: HashMap<u64, JoinHandle<()>> = HashMap::new();
    // NEW: Keep track of group IDs per track/subscription to avoid collisions
    let mut group_counters: HashMap<u64, u64> = HashMap::new();
    loop {
      tokio::select! {
          // Outgoing: Commands from IPC
        Some(action) = action_rx.recv() => {
          let res = match action {
            ControlAction::SendSubscribe(msg) => handler.send_impl(&msg).await,
            ControlAction::SendUnsubscribe(msg) => handler.send_impl(&msg).await,
            ControlAction::SendAnnounce(msg) => handler.send_impl(&msg).await,
            ControlAction::SendFetch(msg) => handler.send_impl(&msg).await,
            ControlAction::SendPublish(msg) => handler.send_impl(&msg).await,
            ControlAction::SendSubscribeNamespace(msg) => handler.send_impl(&msg).await,
          };
          if let Err(e) = res {
            let _ = event_tx.send(serde_json::json!({"method": "log", "params": {"level": "error", "message": format!("Send failed: {:?}", e)}})).await;
          }
        }

          // Incoming: Messages from Relay/Peer
        msg_res = handler.next_message() => {
          match msg_res {
            Ok(msg) => {
              match msg {
                ControlMessage::Publish(pub_msg) => {
                  // A peer wants to push a track to us.
                  // Implicit Consent: We accept everything.

                  // Notify Driver
                  let _ = event_tx.send(serde_json::to_value(RpcNotification::OnPeerPublish {
                    params: PeerPublishParams {
                      namespace: pub_msg.track_namespace.to_utf8_path(),
                      track: pub_msg.track_name.clone(),
                      start_group: 0, // Information usually not in Publish msg, inferred
                      start_object: 0,
                    }
                  }).unwrap()).await;

                  // Send PUBLISH_OK
                  // Draft-16: PublishOk { RequestID, Params }
                  let ok = PublishOk::new(
                    pub_msg.request_id,
                    1,                          // forward: 1
                    128,                        // subscriber_priority: default
                    GroupOrder::Ascending,      // group_order
                    FilterType::LatestObject,   // filter_type
                    None,                       // start_location (Not needed for LatestObject)
                    None,                       // end_group (None = open-ended)
                    vec![]                      // parameters
                  );
                  if let Err(e) = handler.send_impl(&ok).await {
                    let _ = event_tx.send(serde_json::json!({"method": "log", "params": {"level": "error", "message": format!("Failed to send PublishOk: {}", e)}})).await;
                  }

                  // We don't need to do anything else; the background_data_listener
                  // will automatically pick up the incoming streams the peer starts sending.
                }
                ControlMessage::PublishOk(ok_msg) => {
                  let _ = event_tx.send(serde_json::json!({"method": "log", "params": {"level": "info", "message": format!("Peer accepted PUBLISH ID {}", ok_msg.request_id)}})).await;

                  // Start Blasting Data
                  let conn_clone = connection.clone();
                  let req_id = ok_msg.request_id;
                  // We use the Request ID as the Track Alias for simplicity here
                  let track_alias = req_id;

                  let task_handle = tokio::spawn(async move {
                    // Start from Group 0, run indefinitely (None)
                    let _ = Self::run_mock_blaster(conn_clone, track_alias, 0, None).await;
                  });
                  active_tasks.insert(req_id, task_handle);
                }
                ControlMessage::SubscribeNamespace(sub_ns) => {
                  // Send REQUEST_OK (Draft-16 Section 9.7)
                  // TODO: This message is named RequestOK in the draft 16
                  let ok = SubscribeNamespaceOk::new(sub_ns.request_id);
                  if let Err(e) = handler.send_impl(&ok).await {
                        let _ = event_tx.send(serde_json::json!({"method": "log", "params": {"level": "error", "message": format!("Failed to send RequestOk: {}", e)}})).await;
                  }

                  // Mock Logic: If they subscribe to "meet", and we have "meet/123", we should send NAMESPACE msg.
                  // For now, we just log that they asked.
                  let _ = event_tx.send(serde_json::json!({
                      "method": "log",
                      "params": {
                          "level": "info",
                          "message": format!("Peer subscribed to namespace prefix: {:?}", sub_ns.track_namespace_prefix)
                      }
                  })).await;

                  // TODO: Iterate `known_namespaces` and send `ControlMessage::Namespace`
                  // for any that match the prefix. This would only be used when there is no relay. So wontfix for now.
                }

                ControlMessage::PublishNamespaceOk(ok) => {
                  let _ = event_tx.send(serde_json::json!({
                    "method": "log",
                    "params": {
                        "level": "info",
                        "message": format!("Peer accepted Request ID {}", ok.request_id)
                    }
                  })).await;
                }
                ControlMessage::Subscribe(sub) => {
                  let is_known = {
                    let set = known_namespaces.lock().await;
                    // Convert Tuple to String for check (assuming simple path)
                    // If Tuple is complex, you might need a better helper
                    let ns_str = &sub.track_namespace.to_utf8_path();
                    set.contains(ns_str)
                  };

                  if !is_known {
                      // REJECT: We don't own this namespace
                      let _ = event_tx.send(serde_json::json!({
                          "method": "log",
                          "params": {"level": "warn", "message": format!("Rejected Subscribe for unknown namespace: {:?}", sub.track_namespace)}
                      })).await;

                      // Send 404 Not Found (Code 404 is illustrative, check MoQ spec for exact code)
                      let err_msg = SubscribeError::new(sub.request_id, constant::SubscribeErrorCode::TrackDoesNotExist, ReasonPhrase::try_new("Track does not exist".to_string()).unwrap());
                      let _ = handler.send_impl(&err_msg).await;
                      continue; // Skip the rest
                  }
                    let requested_start_group = match sub.filter_type {
                            FilterType::AbsoluteStart => {
                                // "The filter Start Location is specified explicitly" [cite: 596]
                                sub.start_location.map(|loc| loc.group).unwrap_or(0)
                            },
                            FilterType::AbsoluteRange => {
                                // "The filter Start Location and End Group are specified explicitly" [cite: 600]
                                sub.start_location.map(|loc| loc.group).unwrap_or(0)
                            },
                            // LatestObject (0x2) or NextGroup (0x1) implies starting from "Now"
                            _ => *group_counters.get(&sub.request_id).unwrap_or(&1),
                        };
                        let start_group = if requested_start_group > 0 {
                          requested_start_group
                        } else {
                            *group_counters.get(&sub.request_id).unwrap_or(&1)
                        };
                        let requested_end_group = sub.end_group;

                    // PUBLISHER LOGIC: Handle incoming subscribe
                    let _ = event_tx.send(serde_json::json!({"method": "on_peer_subscribe",
                                                                "params": {
                                                                    "subscribe_id": sub.request_id,
                                                                    "track_name": sub.track_name,
                                                                    //TODO: Add more info about this
                                                                }
                                                            })).await;
                    // 1. Send SubscribeOk
                    let _sub_id = sub.request_id;
                    let track_alias = 1u64; // Hardcoded for Mock

                    let ok_msg = SubscribeOk::new_ascending_with_content(sub.request_id, track_alias, 0, None, None);
                    if let Err(e) = handler.send_impl(&ok_msg).await {
                          let _ = event_tx.send(serde_json::json!({"method": "log", "params": {"level": "error", "message": format!("Failed to send SubOk: {}", e)}})).await;
                    }

                    // 2. Start Blasting Data (Mock Source)
                    let conn = connection.clone();
                    group_counters.insert(sub.request_id, start_group + 100);
                    let task_handle = tokio::spawn(async move {
                        let _ = Self::run_mock_blaster(conn, track_alias, start_group, requested_end_group).await;
                    });

                    if let Some(old) = active_tasks.insert(sub.request_id, task_handle) {
                      old.abort();
                    }
                }
                ControlMessage::PublishNamespaceOk(_) => {
                    let _ = event_tx.send(serde_json::json!({"method": "log", "params": {"level": "info", "message": "Namespace Announced Successfully"}})).await;
                }
                ControlMessage::Fetch(fetch) => {
                    // We only support StandAlone fetches for now
                    if let Some(props) = fetch.standalone_fetch_props {
                        // 1. Log via internal system (replacing info!)
                        let _ = event_tx.send(serde_json::json!({
                            "method": "log",
                            "params": {
                                "level": "info",
                                "message": format!(
                                    "Received Fetch ID {} for range {}..{}",
                                    fetch.request_id,
                                    props.start_location.group,
                                    props.end_location.group
                                )
                            }
                        })).await;

                        let conn_clone = connection.clone();
                        // Unique track alias to avoid collision with live subscriptions
                        let track_alias = 1000 + fetch.request_id;

                        let start = props.start_location.group;

                        // Logic: Draft-16 Fetch End is "Exclusive" (Stop BEFORE this location).
                        // Our Blaster loop is "Inclusive" (Process this group, then check if > limit).
                        // So if End is Group 22, we want to process 20, 21. Limit = 21.
                        let limit = props.end_location.group.saturating_sub(1);
                        let end = Some(limit);

                        // 2. Spawn the Blaster
                        tokio::spawn(async move {
                            let _ = Self::run_mock_blaster(conn_clone, track_alias, start, end).await;
                        });

                    } else {
                        // 1. Log Warning via internal system (replacing warn!)
                        let _ = event_tx.send(serde_json::json!({
                            "method": "log",
                            "params": {
                                "level": "warn",
                                "message": "Received unsupported Joining Fetch"
                            }
                        })).await;
                    }
                }
                _ => {} // Ignore others for now
              }
            }
            Err(e) => {
            let _ = event_tx.send(serde_json::json!({"method": "log", "params": {"level": "error", "message": format!("Control stream died: {}", e)}})).await;
            break;
          }
          }
        }
      }
    }
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

    let evt_tx_clone = self.event_tx.clone();
    let conn_clone = connection.clone();
    tokio::spawn(async move {
      Self::background_data_listener(conn_clone, evt_tx_clone).await;
    });

    // 2. Control Actor (The Handler Manager)
    let (action_tx, action_rx) = mpsc::channel(32);
    self.action_tx = Some(action_tx);

    let evt_tx_clone2 = self.event_tx.clone();
    let conn_clone2 = connection.clone();
    let namespaces_clone = self.announced_namespaces.clone();
    // Move the handler into the background task
    tokio::spawn(async move {
      Self::control_actor_loop(
        handler,
        action_rx,
        evt_tx_clone2,
        conn_clone2,
        namespaces_clone,
      )
      .await;
    });

    Ok(())
  }

  // Logic for generating a proper MoQ Track with multiple Groups (Streams)
  async fn run_mock_blaster(
    connection: Arc<wtransport::Connection>,
    track_alias: u64,
    start_group_id: u64,
    end_group: Option<u64>,
  ) -> Result<()> {
    let mut group_id = start_group_id;

    // OUTER LOOP: The "Time" loop. Each iteration is a new GOP/Group.
    // We run until the task is cancelled (abort() called by actor).
    loop {
      if let Some(limit) = end_group
        && group_id > limit
      {
        //info!("Reached End Group {}, stopping blaster.", limit);
        break;
      }
      // 1. Open a NEW Unidirectional Stream for this Group
      // This maps 1:1 to a QUIC stream as you correctly noted.
      let stream = match connection.open_uni().await {
        Ok(s) => s.await?,
        Err(_) => return Ok(()), // Connection closed
      };

      // 2. Set up the Header for this Group
      // We set object_id=0, group_id=current
      let sub_header = SubgroupHeader::new_with_explicit_id(
        track_alias,
        group_id,
        0, // base_object_id
        1, // publisher_priority
        true,
        true,
      );

      let header_info = HeaderInfo::Subgroup { header: sub_header };
      let stream = Arc::new(Mutex::new(stream));
      let mut stream_handler = SendDataStream::new(stream, header_info).await?;

      // INNER LOOP: The "GOP" loop. Sending objects INSIDE the stream.
      let objects_per_group = 10; // Mimic a small GOP
      let mut prev_id = None;

      for object_id in 0..objects_per_group {
        // Mock Payload
        let payload = format!("Group {} - Obj {}", group_id, object_id);

        let sub_obj = SubgroupObject {
          object_id,
          extension_headers: Some(vec![]),
          object_status: None,
          payload: Some(Bytes::from(payload)),
        };

        let object = Object::try_from_subgroup(sub_obj, track_alias, group_id, Some(group_id), 1)?;

        // Send and handle failure (e.g. if subscriber disconnected mid-stream)
        if let Err(_) = stream_handler.send_object(&object, prev_id).await {
          return Ok(()); // Stop entirely if send fails
        }

        prev_id = Some(object_id);

        // Simulate frame timing (e.g. 100ms per frame)
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
      }

      // 3. Finish the Group (Close the Stream)
      // This tells the relay "This GOP is done".
      let _ = stream_handler.flush().await;

      // Move to next group
      group_id += 1;
    }
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
                  group_id: object.location.group,
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

  async fn do_subscribe(&mut self, params: SubscribeParams) -> Result<u64> {
    let tx = self.action_tx.as_ref().context("Not connected")?;

    let sub_id = self.next_subscribe_id;
    self.next_subscribe_id += 1;

    let priority = params.priority.unwrap_or(128);

    // Default to "ascending" (Original in some older versions, Ascending in newer)
    let order = GroupOrder::Ascending;
    let forward = true;
    let ns = Tuple::from_utf8_path(&params.namespace);
    let track = params.track;
    let empty_params = vec![];

    // Use the Library Constructors to guarantee internal consistency
    let sub = match params.filter_type.as_deref() {
      Some("absolute_start") => {
        let group = params.start_group.unwrap_or(0);
        let object = params.start_object.unwrap_or(0);

        Subscribe::new_absolute_start(
          sub_id,
          ns,
          track,
          priority,
          order,
          forward,
          Location { group, object },
          empty_params,
        )
      }
      Some("absolute_range") => {
        let s_group = params.start_group.unwrap_or(0);
        let s_obj = params.start_object.unwrap_or(0);
        let e_group = params.end_group.unwrap_or(0); // This might fail assertion if < s_group

        Subscribe::new_absolute_range(
          sub_id,
          ns,
          track,
          priority,
          order,
          forward,
          Location {
            group: s_group,
            object: s_obj,
          },
          e_group,
          empty_params,
        )
      }
      // Default: Latest Object
      _ => Subscribe::new_latest_object(sub_id, ns, track, priority, order, forward, empty_params),
    };

    tx.send(ControlAction::SendSubscribe(sub))
      .await
      .context("Actor died")?;
    self
      .log("info", format!("Sent Subscribe ID {}", sub_id))
      .await;
    Ok(sub_id)
  }

  async fn do_unsubscribe(&mut self, sub_id: u64) -> Result<()> {
    let tx = self.action_tx.as_ref().context("Not connected")?;
    let msg = Unsubscribe::new(sub_id);
    tx.send(ControlAction::SendUnsubscribe(msg))
      .await
      .context("Actor died")?;
    Ok(())
  }

  async fn do_fetch(&mut self, params: FetchParams) -> Result<u64> {
    let tx = self.action_tx.as_ref().context("Not connected")?;

    let fetch_id = self.next_subscribe_id;
    self.next_subscribe_id += 1;

    // 1. Prepare Properties
    let ns = Tuple::from_utf8_path(&params.namespace);
    let track = params.track;

    // Map RPC params to Location structs
    let start_loc = Location {
      group: params.start_group,
      object: params.start_object,
    };

    let end_loc = Location {
      group: params.end_group,
      object: params.end_object,
    };

    let props = StandAloneFetchProps {
      track_namespace: ns,
      track_name: track,
      start_location: start_loc,
      end_location: end_loc,
    };

    // 2. Construct Fetch Message using Library Helper
    let fetch_msg = Fetch::new_standalone(
      fetch_id,
      params.priority.unwrap_or(100), // Default priority
      GroupOrder::Ascending,
      props,
      vec![], // No extra parameters
    );

    // 3. Send to Relay
    tx.send(ControlAction::SendFetch(fetch_msg))
      .await
      .context("Actor died")?;

    self
      .log("info", format!("Sent Fetch Request ID {}", fetch_id))
      .await;
    Ok(fetch_id)
  }

  async fn do_publish_namespace(&mut self, ns: &str) -> Result<()> {
    let tx = self.action_tx.as_ref().context("Not connected")?;
    let canonical_ns = Tuple::from_utf8_path(ns).to_utf8_path();
    {
      let mut set = self.announced_namespaces.lock().await;
      set.insert(canonical_ns.to_string());
    }

    // Request ID 0 for announce
    let msg = PublishNamespace::new(0, Tuple::from_utf8_path(ns), &[]);
    tx.send(ControlAction::SendAnnounce(msg))
      .await
      .context("Actor died")?;
    self.log("info", format!("Sent Announce for {}", ns)).await;
    Ok(())
  }

  async fn do_publish(&mut self, params: PublishParams) -> Result<u64> {
    let tx = self.action_tx.as_ref().context("Not connected")?;

    // We use the same ID counter for simplicity
    let req_id = self.next_subscribe_id;
    self.next_subscribe_id += 1;

    let ns = Tuple::from_utf8_path(&params.namespace);
    let track_alias = req_id; // Simple alias mapping

    // Construct Publish Message
    // Note: Check your specific moqtail library version for exact constructor arguments.
    // Draft-16 Publish includes: RequestID, Namespace, Name, Alias, Params.
    let msg = Publish::new(
      req_id,
      ns,
      params.track,
      track_alias,
      GroupOrder::Ascending, // Almost always Ascending
      0,                     // Content Exists (1=Yes, 0=No/Unknown)
      None,                  // Largest Location (Optional, often None for live)
      1,                     // Forward (1=Forward, 0=Do not forward)
      vec![],                // Parameters
    );

    tx.send(ControlAction::SendPublish(msg))
      .await
      .context("Actor died")?;

    self
      .log(
        "info",
        format!("Sent PUBLISH ID {} (waiting for OK)", req_id),
      )
      .await;

    // Note: We don't start the blaster yet! We wait for PUBLISH_OK in the actor loop.
    // We need to store the blaster parameters to use later.
    // Ideally, we'd send these params to the actor, but for this simple impl,
    // we might just start blasting immediately or hardcode the blaster start in the actor logic
    // when PublishOk is received.

    // *Correction*: To properly trigger the blaster with specific start/end groups upon receiving OK,
    // we usually need shared state. For this implementation, we will assume the actor
    // knows default blasting behavior or we rely on the implicit "start blasting"
    // logic we add to the actor.

    Ok(req_id)
  }

  async fn do_subscribe_namespace(&mut self, params: SubscribeNamespaceParams) -> Result<u64> {
    let tx = self.action_tx.as_ref().context("Not connected")?;
    let req_id = self.next_subscribe_id;
    self.next_subscribe_id += 1;

    let prefix = Tuple::from_utf8_path(&params.namespace_prefix);

    // Draft-16: SubscribeNamespace { Request ID, Prefix, Options, Params }
    // Options: 0x0 (Publish), 0x1 (Namespace), 0x2 (Both). Let's ask for Both (0x2).
    let msg = SubscribeNamespace::new(req_id, prefix, vec![]);

    tx.send(ControlAction::SendSubscribeNamespace(msg))
      .await
      .context("Actor died")?;
    self
      .log("info", format!("Sent SubscribeNamespace ID {}", req_id))
      .await;
    Ok(req_id)
  }
}
