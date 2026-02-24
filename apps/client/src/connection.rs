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

use anyhow::Result;
use moqtail::model::control::constant::SUPPORTED_VERSIONS;
use moqtail::model::control::control_message::ControlMessage;
use moqtail::model::error::TerminationCode;
use moqtail::model::{
  control::client_setup::ClientSetup, parameter::setup_parameter::SetupParameter,
};
use moqtail::transport::control_stream_handler::ControlStreamHandler;
use std::sync::Arc;
use std::time::Duration;
use tracing::{error, info};
use wtransport::endpoint::ConnectOptions;
use wtransport::quinn::TransportConfig;
use wtransport::quinn::congestion::BbrConfig;
use wtransport::{ClientConfig, Endpoint, tls};

const CLIENT_SUPPORTED_VERSIONS: &str = "moqt-16";

pub struct MoqConnection {
  pub connection: Arc<wtransport::Connection>,
  pub control_stream: ControlStreamHandler,
}

impl MoqConnection {
  pub async fn establish(server: &str, no_cert_validation: bool) -> Result<Self> {
    let c = ClientConfig::builder().with_bind_default();

    let mut tls_config = if no_cert_validation {
      tls::client::build_default_tls_config(
        Arc::new(tls::rustls::RootCertStore::empty()),
        Some(Arc::new(tls::client::NoServerVerification::new())),
      )
    } else {
      tls::client::build_default_tls_config(Arc::new(tls::build_native_cert_store()), None)
    };

    /*
      We use WebTransport, so we don't use ALPN at the QUIC level.
      If we use raw QUIC, we need to remove h3 and just leave the moq version.
    */
    for version in CLIENT_SUPPORTED_VERSIONS.replace(" ", "").split(",") {
      tls_config.alpn_protocols.push(version.as_bytes().to_vec());
    }

    info!("alpn protocols: {:?}", tls_config.alpn_protocols);

    let mut transport_config = TransportConfig::default();
    transport_config.congestion_controller_factory(Arc::new(BbrConfig::default()));

    let config = c
      .with_custom_tls_and_transport(tls_config, transport_config)
      .keep_alive_interval(Some(Duration::from_secs(3)))
      .max_idle_timeout(Some(Duration::from_secs(7)))
      .unwrap()
      .build();

    info!("Connecting to relay server at {}", server);
    let endpoint = Endpoint::client(config)?;
    let options = ConnectOptions::builder(server)
      .add_header("wt-available-protocols", CLIENT_SUPPORTED_VERSIONS)
      .build();

    let connection = Arc::new(endpoint.connect(options).await?);

    info!("Connected! Connection ID: {}", connection.stable_id());

    // Open bidirectional stream for control messages
    info!("Opening control stream...");
    let (send_stream, recv_stream) = connection.open_bi().await?.await?;
    let mut control_stream = ControlStreamHandler::new(send_stream, recv_stream);

    // Send ClientSetup
    info!("Sending ClientSetup...");
    let max_request_id_param = SetupParameter::new_max_request_id(1000000)
      .try_into()
      .unwrap();

    let client_setup = ClientSetup::new(vec![max_request_id_param]);
    control_stream.send_impl(&client_setup).await?;

    // Receive ServerSetup
    // If the server does not support the requested version, the connection will
    // be terminated with VersionNegotiationFailed rather than receiving a ServerSetup.
    info!("Waiting for ServerSetup...");
    match control_stream.next_message().await {
      Ok(ControlMessage::ServerSetup(m)) => {
        info!("ServerSetup received: {:?}", m);
      }
      Ok(m) => {
        error!("Unexpected message: {:?}", m);
        anyhow::bail!("Expected ServerSetup, got {:?}", m);
      }
      Err(TerminationCode::VersionNegotiationFailed) => {
        anyhow::bail!(
          "Version negotiation failed: server does not support versions: {}",
          SUPPORTED_VERSIONS
        );
      }
      Err(e) => {
        error!("Failed to receive ServerSetup: {:?}", e);
        anyhow::bail!("Failed to receive ServerSetup: {:?}", e);
      }
    };

    Ok(MoqConnection {
      connection,
      control_stream,
    })
  }
}
