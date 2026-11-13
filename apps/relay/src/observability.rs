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

//! Connects the relay core to its observability layer at startup. This is the only
//! module of the relay that may use `relay_api`; the test below keeps it that way.

use crate::server::Server;
use crate::server::inspect::RelayInspect;
use anyhow::Result;
use relay_api::config::{ApiConfig, RelaySettings};
use std::sync::Arc;
use tracing::warn;

/// Starts the HTTP API when `api.enable` is set. Errors in the `api:` settings stop
/// the relay at startup rather than leaving it running without the API.
pub fn start(server: &Server) -> Result<()> {
  let c = server.app_config;
  if !c.api.enable {
    return Ok(());
  }
  let (config, warnings) = ApiConfig::from_settings(
    c.api.clone(),
    RelaySettings {
      qlog_folder: c.qlog_folder.as_ref().map(Into::into),
      upstream_timeout: c.upstream_subscribe_timeout,
      relay_addr: Some(loopback_of(c.bind)),
    },
  )?;
  for warning in warnings {
    warn!("{warning}");
  }
  let reporter = {
    let relay = Arc::new(RelayInspect(server.clone()));
    relay_api::start(config, relay.clone(), relay)?
  };
  relay_hooks::install(reporter);
  Ok(())
}

/// The address this machine reaches the relay's QUIC listener at.
fn loopback_of(bind: std::net::SocketAddr) -> std::net::SocketAddr {
  let ip = if bind.ip().is_unspecified() {
    std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)
  } else {
    bind.ip()
  };
  std::net::SocketAddr::new(ip, bind.port())
}

/// `relay api-token`: prints a new MCP token and the config line holding its hash.
/// `relay api-token --hash` hashes a token read from standard input instead.
/// Returns `false` when the command line is not this command.
pub fn api_token_command() -> Result<bool> {
  let args: Vec<String> = std::env::args().skip(1).collect();
  let token = match args.iter().map(String::as_str).collect::<Vec<_>>()[..] {
    ["api-token"] => {
      let token = relay_api::token::new_token();
      println!("Token (shown once; give it to the MCP client): {token}");
      token
    }
    ["api-token", "--hash"] => {
      let mut token = String::new();
      std::io::stdin().read_line(&mut token)?;
      let token = token.trim().to_string();
      anyhow::ensure!(!token.is_empty(), "no token on standard input");
      token
    }
    _ => return Ok(false),
  };
  println!(
    "Under api: in the relay's configuration:\n  mcp_token_hash: \"{}\"",
    relay_api::token::TokenHash::new(&token)
  );
  Ok(true)
}

#[cfg(test)]
mod tests {
  use std::path::Path;

  /// The relay core reaches the observability layer only through `relay_hooks`.
  /// Everything under `src/server` is core, so none of it may name the API crate or
  /// the libraries it is built on.
  #[test]
  fn core_does_not_import_the_observability_layer() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = vec![root.join("server.rs")];
    let mut dirs = vec![root.join("server")];
    while let Some(dir) = dirs.pop() {
      for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
          dirs.push(path);
        } else if path.extension().is_some_and(|e| e == "rs") {
          files.push(path);
        }
      }
    }
    for file in files {
      let text = std::fs::read_to_string(&file).unwrap();
      for forbidden in ["relay_api", "axum", "prometheus"] {
        assert!(
          !text.contains(forbidden),
          "{} mentions `{forbidden}`; core files must reach the observability layer \
           only through relay_hooks",
          file.display()
        );
      }
    }
  }
}
