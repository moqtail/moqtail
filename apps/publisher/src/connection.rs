use anyhow::Result;
use moqtail::model::common::location::Location;
use moqtail::model::common::tuple::{Tuple, TupleField};
use moqtail::model::control::constant;
use moqtail::model::control::constant::GroupOrder;
use moqtail::model::control::control_message::ControlMessage;
use moqtail::model::control::publish::Publish;
use moqtail::model::error::TerminationCode;
use moqtail::model::{
  control::client_setup::ClientSetup, parameter::setup_parameter::SetupParameter,
};
use moqtail::transport::control_stream_handler::ControlStreamHandler;
use std::sync::Arc;
use std::time::Duration;
use tracing::{error, info};
use wtransport::{ClientConfig, Endpoint};

pub const SUPPORTED_VERSION: u32 = constant::DRAFT_14;

pub struct MoqConnection {
  pub connection: Arc<wtransport::Connection>,
  pub control_stream: ControlStreamHandler,
}

impl MoqConnection {
  pub async fn establish(endpoint: &str, validate_cert: bool) -> Result<Self> {
    let c = ClientConfig::builder().with_bind_default();
    let config = if validate_cert {
      c.with_native_certs()
        .keep_alive_interval(Some(Duration::from_secs(3)))
        .max_idle_timeout(Some(Duration::from_secs(120)))
        .unwrap()
        .build()
    } else {
      c.with_no_cert_validation()
        .keep_alive_interval(Some(Duration::from_secs(3)))
        .max_idle_timeout(Some(Duration::from_secs(120)))
        .unwrap()
        .build()
    };

    info!("Connecting to relay at {}", endpoint);
    let connection = Arc::new(Endpoint::client(config)?.connect(endpoint).await?);
    info!("Connected! Connection ID: {}", connection.stable_id());

    info!("Opening control stream...");
    let (send_stream, recv_stream) = connection.open_bi().await?.await?;
    let mut control_stream = ControlStreamHandler::new(send_stream, recv_stream);

    info!("Sending ClientSetup...");
    let max_request_id_param = SetupParameter::new_max_request_id(1000000)
      .try_into()
      .unwrap();
    let client_setup = ClientSetup::new(vec![SUPPORTED_VERSION], vec![max_request_id_param]);
    control_stream.send_impl(&client_setup).await?;

    info!("Waiting for ServerSetup...");
    match control_stream.next_message().await {
      Ok(ControlMessage::ServerSetup(m)) => {
        info!("ServerSetup received: version={}", m.selected_version);
      }
      Ok(m) => {
        error!("Unexpected message: {:?}", m);
        anyhow::bail!("Expected ServerSetup, got {:?}", m);
      }
      Err(TerminationCode::VersionNegotiationFailed) => {
        anyhow::bail!(
          "Version negotiation failed: server does not support version 0x{:X}",
          SUPPORTED_VERSION
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

  /// Publishes a track on the relay and waits for PublishOk.
  /// Returns the track_alias that was registered.
  pub async fn publish_track(
    &mut self,
    namespace: &str,
    track_name: &str,
    track_alias: u64,
  ) -> Result<u64> {
    let ns = Tuple::from_utf8_path(namespace);

    info!(
      "Publishing track: namespace={}, name={}, alias={}",
      namespace, track_name, track_alias
    );

    let publish = Publish::new(
      track_alias, // request_id
      ns,
      TupleField::from_utf8(track_name),
      track_alias,
      GroupOrder::Ascending,
      1, // content_exists
      Some(Location::new(0, 0)),
      1, // forward
      vec![],
    );
    self
      .control_stream
      .send(&ControlMessage::Publish(Box::new(publish)))
      .await?;

    match self.control_stream.next_message().await {
      Ok(ControlMessage::PublishOk(m)) => {
        info!(
          "Track published: request_id={}, alias={}",
          m.request_id, track_alias
        );
        Ok(track_alias)
      }
      Ok(m) => anyhow::bail!("Expected PublishOk, got {:?}", m),
      Err(e) => anyhow::bail!("Failed waiting for PublishOk: {:?}", e),
    }
  }
}
