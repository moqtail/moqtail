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

//! The API's settings, parsed from the relay's `api_*` configuration, and the
//! startup rules that refuse configurations which would expose the API.

use crate::latency::{LatencyDetail, SlowlogConfig};
use anyhow::{Result, bail};
use std::collections::BTreeSet;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

/// A part of the API that can be turned on or off on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Feature {
  /// The endpoint index, liveness and readiness.
  Health,
  /// Snapshots of the relay's state: overview, configuration, connections, tracks.
  State,
  /// Prometheus metrics.
  Metrics,
  /// Object processing time and the slowlog.
  Latency,
  /// The qlog trace files.
  Qlog,
  /// The live event stream.
  Events,
  /// Endpoints that change the relay, such as GOAWAY and drain.
  Actions,
}

impl Feature {
  pub const ALL: [Feature; 7] = [
    Feature::Health,
    Feature::State,
    Feature::Metrics,
    Feature::Latency,
    Feature::Qlog,
    Feature::Events,
    Feature::Actions,
  ];

  /// The features that run when `api_features` is not set.
  pub const DEFAULT: [Feature; 5] = [
    Feature::Health,
    Feature::State,
    Feature::Metrics,
    Feature::Latency,
    Feature::Qlog,
  ];

  pub fn name(self) -> &'static str {
    match self {
      Feature::Health => "health",
      Feature::State => "state",
      Feature::Metrics => "metrics",
      Feature::Latency => "latency",
      Feature::Qlog => "qlog",
      Feature::Events => "events",
      Feature::Actions => "actions",
    }
  }

  fn parse(name: &str) -> Result<Self> {
    Feature::ALL
      .into_iter()
      .find(|f| f.name() == name)
      .ok_or_else(|| {
        let known: Vec<_> = Feature::ALL.iter().map(|f| f.name()).collect();
        anyhow::anyhow!(
          "unknown API feature `{name}`; known features: {}",
          known.join(", ")
        )
      })
  }
}

/// An address range such as `10.0.0.0/8` or `::1/128`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cidr {
  network: IpAddr,
  prefix: u8,
}

impl Cidr {
  pub fn parse(text: &str) -> Result<Self> {
    let (addr, prefix) = match text.split_once('/') {
      Some((addr, prefix)) => (addr, Some(prefix)),
      None => (text, None),
    };
    let network: IpAddr = addr
      .parse()
      .map_err(|_| anyhow::anyhow!("`{text}` is not an IP address or range"))?;
    let max = if network.is_ipv4() { 32 } else { 128 };
    let prefix = match prefix {
      Some(p) => p
        .parse::<u8>()
        .ok()
        .filter(|p| *p <= max)
        .ok_or_else(|| anyhow::anyhow!("`{text}` has a prefix length outside 0..={max}"))?,
      None => max,
    };
    Ok(Cidr { network, prefix })
  }

  pub fn contains(&self, addr: IpAddr) -> bool {
    // An IPv4 client reaching an IPv6 socket shows up as ::ffff:a.b.c.d.
    let addr = match addr {
      IpAddr::V6(v6) => v6.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(addr),
      v4 => v4,
    };
    match (self.network, addr) {
      (IpAddr::V4(net), IpAddr::V4(a)) => prefix_matches(&net.octets(), &a.octets(), self.prefix),
      (IpAddr::V6(net), IpAddr::V6(a)) => prefix_matches(&net.octets(), &a.octets(), self.prefix),
      _ => false,
    }
  }

  /// True for `0.0.0.0/0` and `::/0`, which match every address of their family.
  fn is_everything(&self) -> bool {
    self.prefix == 0
  }
}

fn prefix_matches(network: &[u8], addr: &[u8], prefix: u8) -> bool {
  let full_bytes = (prefix / 8) as usize;
  if network[..full_bytes] != addr[..full_bytes] {
    return false;
  }
  let rest = prefix % 8;
  if rest == 0 {
    return true;
  }
  let mask = 0xffu8 << (8 - rest);
  network[full_bytes] & mask == addr[full_bytes] & mask
}

/// The `api_*` settings as they appear in the relay's configuration file.
#[derive(Debug, Clone, Default)]
pub struct ApiSettings {
  pub bind: Option<SocketAddr>,
  pub features: Option<Vec<String>>,
  pub allow: Option<Vec<String>>,
  pub read_token: Option<String>,
  pub write_token: Option<String>,
  pub metrics_bind: Option<SocketAddr>,
  pub metrics_token: Option<String>,
  pub cors_origins: Vec<String>,
  pub closed_connection_history: usize,
  pub qlog_folder: Option<std::path::PathBuf>,
  pub upstream_timeout: std::time::Duration,
  /// `relay`, `track` or `subscription`; unset means `subscription`.
  pub latency_detail: Option<String>,
  /// Measure every Nth object; 0 and 1 measure every object.
  pub latency_sample: u64,
  /// Deliveries slower than this go to the slowlog.
  pub slowlog_threshold: std::time::Duration,
  /// Subscriptions waiting longer than this for their first object go to the slowlog.
  pub slowlog_first_object_threshold: std::time::Duration,
  /// Slowlog entries kept.
  pub slowlog_size: usize,
}

/// The API's settings after parsing and the startup checks.
#[derive(Debug, Clone)]
pub struct ApiConfig {
  pub bind: SocketAddr,
  pub features: BTreeSet<Feature>,
  /// Client addresses allowed to connect. `None` on a loopback bind, where only
  /// local programs can connect and the check is skipped.
  pub allow: Option<Vec<Cidr>>,
  pub read_token: Option<String>,
  pub write_token: Option<String>,
  /// A second listener serving only `/api/v1/metrics`.
  pub metrics_bind: Option<SocketAddr>,
  /// Client ranges for the metrics listener; `None` when it is loopback or absent.
  pub metrics_allow: Option<Vec<Cidr>>,
  /// A token that reads `/api/v1/metrics` and nothing else.
  pub metrics_token: Option<String>,
  pub cors_origins: Vec<String>,
  /// How many closed connections to keep for `/connections?closed=true`.
  pub closed_connection_history: usize,
  /// Where the relay writes qlog traces; `None` when it writes none.
  pub qlog_folder: Option<std::path::PathBuf>,
  /// How long the relay waits for a publisher to answer; an upstream request
  /// unanswered for longer counts as timed out.
  pub upstream_timeout: std::time::Duration,
  pub latency_detail: LatencyDetail,
  pub latency_sample: u64,
  pub slowlog: SlowlogConfig,
}

pub const DEFAULT_BIND: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 4480);

fn default_allow() -> Vec<Cidr> {
  vec![
    Cidr {
      network: IpAddr::V4(Ipv4Addr::LOCALHOST),
      prefix: 32,
    },
    Cidr {
      network: IpAddr::V6(Ipv6Addr::LOCALHOST),
      prefix: 128,
    },
  ]
}

/// The startup rules for one listener. Returns the client ranges to check, or
/// `None` on a loopback address, where the check is skipped; or an error when the
/// listener must not start. `setting` names the bind setting in messages.
fn listener_allow(
  setting: &str,
  bind: SocketAddr,
  allow_set: bool,
  allow: &[Cidr],
  authenticated: bool,
  warnings: &mut Vec<String>,
) -> Result<Option<Vec<Cidr>>> {
  if bind.ip().is_loopback() {
    // Only local programs and SSH tunnels can reach a loopback listener, and they
    // all arrive from loopback, so an allowlist could only lock them out.
    if allow_set && allow != default_allow() {
      warnings.push(format!(
        "api_allow is ignored for {setting} {bind}, a loopback address"
      ));
    }
    return Ok(None);
  }
  if allow.is_empty() {
    bail!(
      "api_allow is empty, so nobody could connect to {setting} {bind}; \
       list the client ranges to allow"
    );
  }
  if allow.iter().any(Cidr::is_everything) && !authenticated {
    bail!(
      "api_allow lets every address reach {setting} {bind}, and no token is set; \
       set a token or narrow api_allow"
    );
  }
  if allow == default_allow() {
    warnings.push(format!(
      "{setting} {bind} is a network address but api_allow allows only loopback; \
       remote clients will get 403"
    ));
  } else if !authenticated {
    warnings.push(format!(
      "{setting} {bind} serves reads without a token to the ranges in api_allow"
    ));
  }
  Ok(Some(allow.to_vec()))
}

impl ApiConfig {
  /// Parses the settings and applies the startup rules. Returns the configuration
  /// and the warnings to log, or an error when the API must not start.
  pub fn from_settings(settings: ApiSettings) -> Result<(Self, Vec<String>)> {
    let mut warnings = Vec::new();
    let bind = settings.bind.unwrap_or(DEFAULT_BIND);

    let features = match &settings.features {
      Some(names) => names
        .iter()
        .map(|n| Feature::parse(n))
        .collect::<Result<BTreeSet<_>>>()?,
      None => Feature::DEFAULT.into_iter().collect(),
    };

    let allow_set = settings.allow.is_some();
    let allow_ranges = match &settings.allow {
      Some(ranges) => ranges
        .iter()
        .map(|r| Cidr::parse(r))
        .collect::<Result<Vec<_>>>()?,
      None => default_allow(),
    };
    let allow = allow_ranges.clone();

    let latency_detail = match &settings.latency_detail {
      Some(name) => LatencyDetail::parse(name)?,
      None => LatencyDetail::Subscription,
    };
    let read_token = settings.read_token.filter(|t| !t.is_empty());
    let write_token = settings.write_token.filter(|t| !t.is_empty());

    if features.contains(&Feature::Actions) && write_token.is_none() {
      bail!("the `actions` API feature needs `api_write_token`");
    }

    let allow = listener_allow(
      "api_bind",
      bind,
      allow_set,
      &allow,
      read_token.is_some(),
      &mut warnings,
    )?;
    let metrics_token = settings.metrics_token.filter(|t| !t.is_empty());
    let metrics_allow = match settings.metrics_bind {
      Some(metrics_bind) => listener_allow(
        "api_metrics_bind",
        metrics_bind,
        allow_set,
        &allow_ranges,
        read_token.is_some() || metrics_token.is_some(),
        &mut warnings,
      )?,
      None => None,
    };

    Ok((
      ApiConfig {
        bind,
        features,
        allow,
        read_token,
        write_token,
        metrics_bind: settings.metrics_bind,
        metrics_allow,
        metrics_token,
        cors_origins: settings.cors_origins,
        closed_connection_history: settings.closed_connection_history,
        qlog_folder: settings.qlog_folder,
        upstream_timeout: settings.upstream_timeout,
        latency_detail,
        latency_sample: settings.latency_sample.max(1),
        slowlog: SlowlogConfig {
          threshold: settings.slowlog_threshold,
          first_object_threshold: settings.slowlog_first_object_threshold,
          size: settings.slowlog_size,
        },
      },
      warnings,
    ))
  }

  pub fn has(&self, feature: Feature) -> bool {
    self.features.contains(&feature)
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn settings(bind: &str, allow: Option<&[&str]>, token: Option<&str>) -> ApiSettings {
    ApiSettings {
      bind: Some(bind.parse().unwrap()),
      allow: allow.map(|a| a.iter().map(|s| s.to_string()).collect()),
      read_token: token.map(String::from),
      ..Default::default()
    }
  }

  #[test]
  fn cidr_matching() {
    let net = Cidr::parse("10.1.0.0/16").unwrap();
    assert!(net.contains("10.1.2.3".parse().unwrap()));
    assert!(!net.contains("10.2.0.1".parse().unwrap()));
    assert!(net.contains("::ffff:10.1.9.9".parse().unwrap()));
    let odd = Cidr::parse("192.168.1.128/25").unwrap();
    assert!(odd.contains("192.168.1.200".parse().unwrap()));
    assert!(!odd.contains("192.168.1.100".parse().unwrap()));
    assert!(Cidr::parse("::1").unwrap().contains("::1".parse().unwrap()));
    assert!(Cidr::parse("10.0.0.0/33").is_err());
    assert!(Cidr::parse("not-an-ip").is_err());
  }

  #[test]
  fn loopback_bind_skips_the_allowlist() {
    let (config, warnings) =
      ApiConfig::from_settings(settings("127.0.0.1:4480", Some(&[]), None)).unwrap();
    assert!(config.allow.is_none());
    assert_eq!(
      warnings.len(),
      1,
      "a changed api_allow is reported as ignored"
    );

    let (_, warnings) = ApiConfig::from_settings(settings("127.0.0.1:4480", None, None)).unwrap();
    assert!(warnings.is_empty());
  }

  #[test]
  fn network_bind_rules() {
    assert!(ApiConfig::from_settings(settings("0.0.0.0:4480", Some(&[]), None)).is_err());
    assert!(
      ApiConfig::from_settings(settings("0.0.0.0:4480", Some(&["0.0.0.0/0"]), None)).is_err()
    );
    assert!(
      ApiConfig::from_settings(settings("0.0.0.0:4480", Some(&["0.0.0.0/0"]), Some("t"))).is_ok()
    );

    let (_, warnings) =
      ApiConfig::from_settings(settings("0.0.0.0:4480", Some(&["10.0.0.0/8"]), None)).unwrap();
    assert!(warnings[0].contains("without a token"));

    let (_, warnings) = ApiConfig::from_settings(settings("0.0.0.0:4480", None, None)).unwrap();
    assert!(warnings[0].contains("will get 403"));
  }

  #[test]
  fn features() {
    let (config, _) = ApiConfig::from_settings(ApiSettings::default()).unwrap();
    assert!(config.has(Feature::State) && !config.has(Feature::Events));

    let unknown = ApiSettings {
      features: Some(vec!["health".into(), "nope".into()]),
      ..Default::default()
    };
    assert!(ApiConfig::from_settings(unknown).is_err());

    let actions_without_token = ApiSettings {
      features: Some(vec!["actions".into()]),
      ..Default::default()
    };
    assert!(ApiConfig::from_settings(actions_without_token).is_err());
  }
}
