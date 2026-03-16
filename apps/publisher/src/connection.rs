use anyhow::{Context, Result};
use bytes::Bytes;
use moqtail::model::common::location::Location;
use moqtail::model::common::tuple::{Tuple, TupleField};
use moqtail::model::control::constant;
use moqtail::model::control::constant::GroupOrder;
use moqtail::model::control::control_message::ControlMessage;
use moqtail::model::control::publish::Publish;
use moqtail::model::data::object::Object;
use moqtail::model::data::subgroup_header::SubgroupHeader;
use moqtail::model::data::subgroup_object::SubgroupObject;
use moqtail::model::error::TerminationCode;
use moqtail::model::{
  control::client_setup::ClientSetup, parameter::setup_parameter::SetupParameter,
};
use moqtail::transport::control_stream_handler::ControlStreamHandler;
use moqtail::transport::data_stream_handler::{HeaderInfo, SendDataStream};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
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

  /// Sends `payload` as a single MoQ object on a new subgroup stream.
  /// `group_id` should increment each time the catalog is resent so that
  /// subscribers waiting for new objects will receive the latest catalog.
  pub async fn send_catalog_object(
    &self,
    track_alias: u64,
    group_id: u64,
    payload: Bytes,
  ) -> Result<()> {
    send_catalog_object_on_connection(&self.connection, track_alias, group_id, payload).await
  }

  /// Same as `send_catalog_object` but callable with just the raw connection
  /// (for use from background tasks that don't hold the full MoqConnection).
  pub async fn send_catalog_object_static(
    connection: &Arc<wtransport::Connection>,
    track_alias: u64,
    group_id: u64,
    payload: Bytes,
  ) -> Result<()> {
    send_catalog_object_on_connection(connection, track_alias, group_id, payload).await
  }
}

/// Inner implementation shared by `send_catalog_object` and `send_catalog_object_static`.
async fn send_catalog_object_on_connection(
  connection: &Arc<wtransport::Connection>,
  track_alias: u64,
  group_id: u64,
  payload: Bytes,
) -> Result<()> {
  let stream = connection
    .open_uni()
    .await?
    .await
    .context("failed to open catalog uni stream")?;

  let header = SubgroupHeader::new_with_explicit_id(
    track_alias,
    group_id,
    0,     // subgroup_id
    0,     // publisher_priority (catalog is highest priority)
    false, // no extension headers
    true,  // end_of_group
  );
  let header_info = HeaderInfo::Subgroup { header };
  let stream = Arc::new(Mutex::new(stream));
  let mut handler = SendDataStream::new(stream, header_info)
    .await
    .context("failed to initialize catalog stream handler")?;

  let subgroup_object = SubgroupObject {
    object_id: 0,
    extension_headers: None,
    object_status: None,
    payload: Some(payload),
  };
  let object = Object::try_from_subgroup(subgroup_object, track_alias, group_id, Some(group_id), 0)
    .context("failed to build catalog object")?;

  handler
    .send_object(&object, None)
    .await
    .context("failed to write catalog object")?;
  handler
    .flush()
    .await
    .context("failed to flush catalog stream")?;
  handler
    .finish()
    .await
    .context("failed to finish catalog stream")?;

  info!(
    "Catalog object sent on alias={} group={}",
    track_alias, group_id
  );
  Ok(())
}
