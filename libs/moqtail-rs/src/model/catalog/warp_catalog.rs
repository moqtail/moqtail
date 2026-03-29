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

/*
MOQT Streaming Format (MSF) Catalog (draft-ietf-moq-msf-00)
https://www.ietf.org/archive/id/draft-ietf-moq-msf-00.html
*/

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct MsfCatalog {
  pub version: u8,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub generated_at: Option<u64>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub delta_update: Option<bool>,
  pub tracks: Vec<Track>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Track {
  pub name: String,
  pub render_group: u8,
  pub packaging: String,
  pub is_live: bool,
  pub codec: String,

  #[serde(skip_serializing_if = "Option::is_none")]
  pub role: Option<String>,

  #[serde(skip_serializing_if = "Option::is_none")]
  pub target_latency: Option<u32>,

  #[serde(skip_serializing_if = "Option::is_none")]
  pub width: Option<u32>,

  #[serde(skip_serializing_if = "Option::is_none")]
  pub height: Option<u32>,

  #[serde(skip_serializing_if = "Option::is_none")]
  pub bitrate: Option<u32>,

  #[serde(skip_serializing_if = "Option::is_none")]
  pub framerate: Option<f64>,

  #[serde(skip_serializing_if = "Option::is_none")]
  pub timescale: Option<u32>,

  #[serde(skip_serializing_if = "Option::is_none")]
  pub alt_group: Option<u8>,

  #[serde(skip_serializing_if = "Option::is_none")]
  pub init_data: Option<String>,

  #[serde(rename = "samplerate", skip_serializing_if = "Option::is_none")]
  pub sample_rate: Option<u32>,

  #[serde(skip_serializing_if = "Option::is_none")]
  pub channel_config: Option<String>,
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_empty_catalog_serialization() {
    let catalog = MsfCatalog {
      version: 1,
      generated_at: None,
      delta_update: None,
      tracks: vec![],
    };

    let serialized = serde_json::to_string(&catalog).unwrap();
    assert!(serialized.contains("\"version\":1"));
    assert!(serialized.contains("\"tracks\":[]"));
    assert!(!serialized.contains("\"generatedAt\""));
  }

  #[test]
  fn test_track_with_minimal_fields() {
    let track = Track {
      name: "Minimal Track".to_string(),
      render_group: 1,
      packaging: "loc".to_string(),
      is_live: true,
      codec: "hvc1".to_string(),
      role: Some("video".to_string()),
      target_latency: None,
      width: None,
      height: None,
      bitrate: None,
      framerate: None,
      timescale: None,
      alt_group: None,
      init_data: None,
      sample_rate: None,
      channel_config: None,
    };

    let serialized = serde_json::to_string(&track).unwrap();
    assert!(serialized.contains("\"name\":\"Minimal Track\""));
    assert!(serialized.contains("\"renderGroup\":1"));
    assert!(serialized.contains("\"packaging\":\"loc\""));
    assert!(serialized.contains("\"isLive\":true"));
    assert!(serialized.contains("\"role\":\"video\""));
  }

  #[test]
  fn test_catalog_with_video_and_audio_tracks() {
    let catalog = MsfCatalog {
      version: 1,
      generated_at: Some(1_746_104_606_044),
      delta_update: Some(true),
      tracks: vec![
        Track {
          name: "video".to_string(),
          render_group: 1,
          packaging: "loc".to_string(),
          is_live: true,
          codec: "hvc1.1.6.L93.B0".to_string(),
          role: Some("video".to_string()),
          target_latency: Some(1500),
          width: Some(1280),
          height: Some(720),
          bitrate: Some(3_000_000),
          framerate: Some(30.0),
          timescale: Some(90_000),
          alt_group: Some(1),
          init_data: Some("AQID".to_string()),
          sample_rate: None,
          channel_config: None,
        },
        Track {
          name: "audio".to_string(),
          render_group: 1,
          packaging: "loc".to_string(),
          is_live: true,
          codec: "opus".to_string(),
          role: Some("audio".to_string()),
          target_latency: Some(1500),
          width: None,
          height: None,
          bitrate: Some(128_000),
          framerate: None,
          timescale: None,
          alt_group: None,
          init_data: None,
          sample_rate: Some(48_000),
          channel_config: Some("2".to_string()),
        },
      ],
    };

    let serialized = serde_json::to_string(&catalog).unwrap();
    assert!(serialized.contains("\"generatedAt\":1746104606044"));
    assert!(serialized.contains("\"targetLatency\":1500"));
    assert!(serialized.contains("\"timescale\":90000"));
    assert!(serialized.contains("\"initData\":\"AQID\""));
    assert!(serialized.contains("\"samplerate\":48000"));
  }

  #[test]
  fn test_invalid_catalog_deserialization() {
    let invalid_json = r#"
      {
        "version": "invalid",
        "tracks": []
      }
    "#;
    let result: Result<MsfCatalog, _> = serde_json::from_str(invalid_json);
    assert!(result.is_err());
  }

  #[test]
  fn test_is_live_required_on_track() {
    let missing_is_live = r#"
      {
        "name": "video",
        "renderGroup": 1,
        "packaging": "loc",
        "codec": "hvc1"
      }
    "#;

    let result: Result<Track, _> = serde_json::from_str(missing_is_live);
    assert!(result.is_err());
  }

  #[test]
  fn test_track_deserialization() {
    let json_data = r#"
      {
        "name": "video",
        "renderGroup": 1,
        "packaging": "loc",
        "isLive": true,
        "codec": "hvc1",
        "role": "video",
        "targetLatency": 1500,
        "timescale": 90000,
        "initData": "AQID"
      }
    "#;

    let track: Track = serde_json::from_str(json_data).unwrap();
    assert_eq!(track.name, "video");
    assert!(track.is_live);
    assert_eq!(track.role.as_deref(), Some("video"));
    assert_eq!(track.target_latency, Some(1500));
    assert_eq!(track.timescale, Some(90_000));
    assert_eq!(track.init_data.as_deref(), Some("AQID"));
  }
}
