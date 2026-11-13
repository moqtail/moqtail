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

mod cli;
mod connection;
mod fetcher;
mod filter_spec;
mod prefix_subscriber;
mod publisher;
mod routing;
mod stats;
mod subscriber;
mod utils;

use clap::Parser;
use cli::{Cli, CliFilter, Command, PrefixKind};
use connection::MoqConnection;
use moqtail::model::control::goaway::GoAway;
use moqtail::model::parameter::message_parameter::MessageParameter;
use moqtail::transport::qlog;
use std::future::Future;
use tokio::sync::oneshot;
use tracing::info;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::filter::LevelFilter;

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
  init_logging();

  let cli = Cli::parse();

  info!(
    "Starting moqtail client: server={}, namespace={}, track={}",
    cli.server, cli.namespace, cli.track_name
  );

  if let Some(dir) = &cli.qlog_folder {
    qlog::init(dir, "client")?;
  }

  let result = run(cli).await;
  qlog::flush(std::time::Duration::from_secs(2));
  result
}

/// Runs the command, and on GOAWAY closes the session and, when the server named a
/// new session URI and the command can resume, runs it again there.
async fn run(cli: Cli) -> Result<(), anyhow::Error> {
  let mut server = cli.server.clone();
  loop {
    let (moq_conn, goaway_rx) =
      MoqConnection::establish(&server, cli.no_cert_validation, cli.transport).await?;
    let connection = moq_conn.connection.clone();
    let Some(goaway) = until_goaway(run_command(moq_conn, cli.clone()), goaway_rx).await? else {
      return Ok(());
    };
    // The command future is dropped by now, so no new requests go out; close
    // before the server's GOAWAY timeout rather than waiting to be cut off.
    connection.close(0, b"GOAWAY");
    match next_server(&goaway, cli.command) {
      Some(uri) => {
        info!("Moving to {uri} after GOAWAY");
        server = uri;
      }
      None => {
        info!("Leaving after GOAWAY");
        return Ok(());
      }
    }
  }
}

/// Runs `command` until it ends (`None`) or the server sends GOAWAY (`Some`),
/// whichever comes first. A GOAWAY drops the command, ending its requests.
async fn until_goaway(
  command: impl Future<Output = Result<(), anyhow::Error>>,
  goaway_rx: oneshot::Receiver<GoAway>,
) -> Result<Option<GoAway>, anyhow::Error> {
  tokio::select! {
    result = command => result.map(|()| None),
    // A closed channel means the control stream ended; let the command finish.
    Ok(goaway) = goaway_rx => Ok(Some(goaway)),
  }
}

/// Where to reconnect after GOAWAY: the new session URI, for commands that run
/// until stopped. A fetch is one request, and with no URI there is nowhere to go.
fn next_server(goaway: &GoAway, command: Command) -> Option<String> {
  match command {
    Command::Fetch => None,
    _ => goaway.new_session_uri.clone(),
  }
}

async fn run_command(moq_conn: MoqConnection, cli: Cli) -> Result<(), anyhow::Error> {
  let content = publisher::Content {
    subgroups_per_group: cli.subgroups_per_group,
    priorities: cli.priorities.clone(),
    object_properties: cli.object_property.clone(),
    track_properties: cli.track_property.clone(),
    fetch_markers: cli.fetch_end_of_range.clone(),
    fetch_reset_after: cli.fetch_reset_after,
    end_of_group_objects: cli.end_of_group_objects,
    end_of_track_object: cli.end_of_track_object,
    reset_streams_at: cli.reset_stream_at.clone(),
    dropped_datagrams: cli.drop_datagram.clone(),
    gap_properties: !cli.no_gap_properties,
  };

  match cli.command {
    Command::Publish => {
      let config = publisher::PublishConfig {
        namespace: cli.namespace,
        track_name: cli.track_name,
        delivery_mode: cli.delivery_mode,
        group_count: cli.group_count,
        pre_published_groups: cli.pre_published_groups,
        interval: cli.interval,
        objects_per_group: cli.objects_per_group,
        object_id_step: cli.object_id_step,
        group_id_step: cli.group_id_step,
        payload_size: cli.payload_size,
        track_alias: cli
          .track_alias
          .unwrap_or_else(|| rand::random::<u64>() & ((1u64 << 62) - 1)),
        publisher_priority: cli.publisher_priority,
        content: content.clone(),
      };
      publisher::run(moq_conn, config).await
    }
    Command::PublishNamespace => {
      let config = publisher::PublishNamespaceConfig {
        namespace: cli.namespace,
        withdraw_after: cli.withdraw_after,
        delivery_mode: cli.delivery_mode,
        group_count: cli.group_count,
        pre_published_groups: cli.pre_published_groups,
        interval: cli.interval,
        objects_per_group: cli.objects_per_group,
        object_id_step: cli.object_id_step,
        group_id_step: cli.group_id_step,
        payload_size: cli.payload_size,
        publisher_priority: cli.publisher_priority,
        content: content.clone(),
      };
      publisher::run_namespace(moq_conn, config).await
    }
    Command::Subscribe => {
      let config = subscriber::SubscribeConfig {
        namespace: cli.namespace,
        track_name: cli.track_name,
        delivery_mode: cli.delivery_mode,
        duration: cli.duration,
        subscriber_priority: cli.subscriber_priority,
        group_order: cli.group_order.into(),
        extra_track: cli.extra_track.as_deref().and_then(|s| {
          let (name, prio) = s.rsplit_once(':')?;
          let priority: u8 = prio.parse().ok()?;
          Some((name.to_string(), priority))
        }),
        forward: cli.forward,
        include_properties: cli.include_properties,
        rendezvous_timeout: cli.rendezvous_timeout,
        update_forward_after: cli.update_forward_after,
        filter: cli.filter.unwrap_or(CliFilter::NextObject),
        filter_start_group: cli.filter_start_group,
        filter_start_object: cli.filter_start_object,
        end_group_delta: cli.end_group_delta,
        filter_end_object: cli.filter_end_object,
        fill: cli.fill || cli.fill_start_group.is_some() || !cli.fill_range_filter.is_empty(),
        fill_start_group: cli.fill_start_group,
        switch_after: cli.switch_after,
        switch_track: cli.switch_track,
        switch_mode: cli.switch_mode.into(),
        switch_publish_done: cli.switch_publish_done,
        switch_start_group: cli.switch_start_group,
        range_filters: cli.range_filter,
        fill_range_filters: cli.fill_range_filter,
        update_filters_after: cli.update_filters_after,
        update_range_filters: cli.update_range_filter,
        ignore_filter_limit: cli.ignore_filter_limit,
      };
      subscriber::run(moq_conn, config).await
    }
    Command::SubscribeNamespace | Command::SubscribeTracks => {
      let config = prefix_subscriber::PrefixSubscribeConfig {
        namespace: cli.namespace,
        duration: cli.duration,
        kind: match cli.command {
          Command::SubscribeNamespace => PrefixKind::Namespace,
          _ => PrefixKind::Tracks,
        },
        overlap: cli.overlap,
        include_properties: cli.include_properties,
        pushed_track_parameters: {
          // Each pushed track's Forward State, where it starts, and a fill of what it
          // already has.
          let mut params = Vec::new();
          if !cli.forward {
            params.push(MessageParameter::new_forward(false));
          }
          if let Some(filter) = cli.filter {
            params.push(MessageParameter::new_location_filter(
              subscriber::cli_location_filter(
                filter,
                cli.filter_start_group,
                cli.filter_start_object,
                cli.end_group_delta,
                cli.filter_end_object,
              ),
            ));
          }
          if cli.fill || cli.fill_start_group.is_some() || !cli.fill_range_filter.is_empty() {
            params.push(subscriber::fill_parameter(
              cli.fill_start_group,
              &cli.fill_range_filter,
            ));
          }
          params
        },
        refuse_publish: cli.refuse_publish,
        range_filters: cli.range_filter,
        ignore_filter_limit: cli.ignore_filter_limit,
      };
      prefix_subscriber::run(moq_conn, config).await
    }
    Command::Fetch => {
      let config = fetcher::FetchConfig {
        namespace: cli.namespace,
        track_name: cli.track_name,
        start_group: cli.start_group,
        start_object: cli.start_object,
        end_group: cli.end_group,
        end_object: cli.end_object,
        group_order: cli.group_order.into(),
        cancel_after: cli.cancel_after,
        include_properties: cli.include_properties,
        range_filters: cli.range_filter,
        ignore_filter_limit: cli.ignore_filter_limit,
      };
      fetcher::run(moq_conn, config).await
    }
  }
}

fn init_logging() {
  let env_filter = EnvFilter::builder()
    .with_default_directive(LevelFilter::INFO.into())
    .from_env_lossy();

  tracing_subscriber::fmt()
    .with_target(true)
    .with_level(true)
    .with_env_filter(env_filter)
    .init();
}

#[cfg(test)]
mod tests {
  use super::*;

  #[tokio::test]
  async fn goaway_interrupts_a_running_command() {
    let (tx, rx) = oneshot::channel();
    tx.send(GoAway::new(Some("moqt://next:4433".into()), 5000))
      .unwrap();
    let goaway = until_goaway(std::future::pending(), rx).await.unwrap();
    assert_eq!(
      goaway,
      Some(GoAway::new(Some("moqt://next:4433".into()), 5000))
    );
  }

  #[tokio::test]
  async fn command_ends_without_goaway() {
    let (_tx, rx) = oneshot::channel();
    assert_eq!(until_goaway(async { Ok(()) }, rx).await.unwrap(), None);
  }

  #[tokio::test]
  async fn closed_control_stream_lets_the_command_finish() {
    let (tx, rx) = oneshot::channel::<GoAway>();
    drop(tx);
    assert_eq!(until_goaway(async { Ok(()) }, rx).await.unwrap(), None);
  }

  #[test]
  fn reconnects_only_with_a_uri_and_a_resumable_command() {
    let moved = GoAway::new(Some("moqt://next:4433".into()), 5000);
    let stay = GoAway::new(None, 5000);
    assert_eq!(
      next_server(&moved, Command::Subscribe).as_deref(),
      Some("moqt://next:4433")
    );
    assert_eq!(
      next_server(&moved, Command::Publish).as_deref(),
      Some("moqt://next:4433")
    );
    assert_eq!(next_server(&moved, Command::Fetch), None);
    assert_eq!(next_server(&stay, Command::Subscribe), None);
  }
}
