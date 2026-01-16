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

use anyhow::Result;
use tokio::sync::mpsc;

mod core;
mod ipc;

#[tokio::main]
async fn main() -> Result<()> {
  // 1. Create Channels
  // cmd_tx: IPC -> Core (User Commands)
  // evt_tx: Core -> IPC (Responses & Stats)
  let (cmd_tx, cmd_rx) = mpsc::channel(32);
  let (evt_tx, evt_rx) = mpsc::channel(32);

  // 2. Spawn the IPC Thread (Blocking Stdin)
  // We clone evt_tx because IPC might need to send errors too,
  // but in this design IPC mostly receives events to print.
  let ipc_cmd_tx = cmd_tx.clone();
  std::thread::spawn(move || {
    ipc::run_loop(ipc_cmd_tx, evt_rx);
  });

  // 3. Initialize Core
  let mut controller = core::controller::ClientController::new(evt_tx);

  // 4. Run Core Loop
  controller.run(cmd_rx).await;

  Ok(())
}
