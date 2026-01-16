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

pub mod messages;
use messages::RpcRequest;
use std::io::{self, BufRead};
use tokio::sync::mpsc;

pub fn run_loop(cmd_tx: mpsc::Sender<RpcRequest>, mut evt_rx: mpsc::Receiver<serde_json::Value>) {
  let stdin = io::stdin();
  let mut handle = stdin.lock();
  let mut buffer = String::new();

  // Spawning a separate thread to print outputs so stdin blocking doesn't stop us printing
  std::thread::spawn(move || {
    while let Some(msg) = evt_rx.blocking_recv() {
      println!("{}", msg);
    }
  });

  loop {
    buffer.clear();
    match handle.read_line(&mut buffer) {
      Ok(0) => break, // EOF
      Ok(_) => {
        let trimmed = buffer.trim();
        if trimmed.is_empty() {
          continue;
        }

        match serde_json::from_str::<RpcRequest>(trimmed) {
          Ok(req) => {
            if cmd_tx.blocking_send(req).is_err() {
              break; // Channel closed
            }
          }
          Err(e) => {
            eprintln!("{{\"error\": \"Parse error: {}\"}}", e);
          }
        }
      }
      Err(e) => eprintln!("{{\"error\": \"IO error: {}\"}}", e),
    }
  }
}
