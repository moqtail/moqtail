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
  pub delta_update: Option<bool>,
  pub tracks: Vec<Track>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Track {
  pub name: String,
  pub render_group: u8,
  pub packaging: String,
  pub codec: String,

  #[serde(skip_serializing_if = "Option::is_none")]
  pub width: Option<u32>,

  #[serde(skip_serializing_if = "Option::is_none")]
  pub height: Option<u32>,

  #[serde(skip_serializing_if = "Option::is_none")]
  pub bitrate: Option<u32>,

  #[serde(skip_serializing_if = "Option::is_none")]
  pub framerate: Option<f64>,

  #[serde(skip_serializing_if = "Option::is_none")]
  pub alt_group: Option<u8>,

  #[serde(skip_serializing_if = "Option::is_none")]
  pub is_live: Option<bool>,

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
      delta_update: None,
      tracks: vec![],
    };

    let serialized = serde_json::to_string(&catalog).unwrap();

    assert!(serialized.contains("\"version\":1"));
    assert!(serialized.contains("\"tracks\":[]"));
    assert!(!serialized.contains("\"deltaUpdate\""));
  }

  #[test]
  fn test_track_with_minimal_fields() {
    let track = Track {
      name: "Minimal Track".to_string(),
      render_group: 1,
      packaging: "loc".to_string(),
      codec: "hvc1".to_string(),
      width: None,
      height: None,
      bitrate: None,
      framerate: None,
      alt_group: None,
      is_live: None,
      init_data: None,
      sample_rate: None,
      channel_config: None,
    };

    let serialized = serde_json::to_string(&track).unwrap();

    assert!(serialized.contains("\"name\":\"Minimal Track\""));
    assert!(serialized.contains("\"renderGroup\":1"));
    assert!(serialized.contains("\"packaging\":\"loc\""));
    assert!(serialized.contains("\"codec\":\"hvc1\""));
    assert!(!serialized.contains("\"width\""));
    assert!(!serialized.contains("\"height\""));
    assert!(!serialized.contains("\"bitrate\""));
    assert!(!serialized.contains("\"framerate\""));
    assert!(!serialized.contains("\"altGroup\""));
    assert!(!serialized.contains("\"isLive\""));
    assert!(!serialized.contains("\"initData\""));
    assert!(!serialized.contains("\"samplerate\""));
    assert!(!serialized.contains("\"channelConfig\""));
  }

  #[test]
  fn test_catalog_with_mixed_tracks() {
    let catalog = MsfCatalog {
      version: 1,
      delta_update: Some(true),
      tracks: vec![
        Track {
          name: "Video Track".to_string(),
          render_group: 1,
          packaging: "loc".to_string(),
          codec: "hvc1.1.6.L93.B0".to_string(),
          width: Some(1280),
          height: Some(720),
          bitrate: Some(3_000_000),
          framerate: Some(60.0),
          alt_group: Some(1),
          is_live: Some(true),
          init_data: None,
          sample_rate: None,
          channel_config: None,
        },
        Track {
          name: "Audio Track".to_string(),
          render_group: 1,
          packaging: "loc".to_string(),
          codec: "opus".to_string(),
          width: None,
          height: None,
          bitrate: Some(128_000),
          framerate: None,
          alt_group: None,
          is_live: Some(true),
          init_data: None,
          sample_rate: Some(48_000),
          channel_config: Some("2".to_string()),
        },
      ],
    };

    let serialized = serde_json::to_string(&catalog).unwrap();

    assert!(serialized.contains("\"version\":1"));
    assert!(serialized.contains("\"deltaUpdate\":true"));
    assert!(serialized.contains("\"name\":\"Video Track\""));
    assert!(serialized.contains("\"codec\":\"hvc1.1.6.L93.B0\""));
    assert!(serialized.contains("\"width\":1280"));
    assert!(serialized.contains("\"height\":720"));
    assert!(serialized.contains("\"bitrate\":3000000"));
    assert!(serialized.contains("\"framerate\":60.0"));
    assert!(serialized.contains("\"name\":\"Audio Track\""));
    assert!(serialized.contains("\"codec\":\"opus\""));
    assert!(serialized.contains("\"samplerate\":48000"));
    assert!(serialized.contains("\"channelConfig\":\"2\""));
  }

  #[test]
  fn test_invalid_catalog_deserialization() {
    let invalid_json_data = r#"
      {
        "version": "invalid",
        "tracks": []
      }
    "#;

    let result: Result<MsfCatalog, _> = serde_json::from_str(invalid_json_data);
    assert!(result.is_err());
  }

  #[test]
  fn test_track_with_partial_fields_deserialization() {
    let json_data = r#"
      {
        "name": "Partial Track",
        "renderGroup": 1,
        "packaging": "loc",
        "codec": "vp9",
        "width": 640,
        "height": 360
      }
    "#;

    let track: Track = serde_json::from_str(json_data).unwrap();

    assert_eq!(track.name, "Partial Track");
    assert_eq!(track.render_group, 1);
    assert_eq!(track.packaging, "loc");
    assert_eq!(track.codec, "vp9");
    assert_eq!(track.width, Some(640));
    assert_eq!(track.height, Some(360));
    assert_eq!(track.bitrate, None);
    assert_eq!(track.framerate, None);
    assert_eq!(track.alt_group, None);
    assert_eq!(track.sample_rate, None);
    assert_eq!(track.channel_config, None);
  }

  #[test]
  fn test_catalog_serialization() {
    let catalog = MsfCatalog {
      version: 1,
      delta_update: Some(true),
      tracks: vec![
        Track {
          name: "Track 1".to_string(),
          render_group: 1,
          packaging: "loc".to_string(),
          codec: "h264".to_string(),
          width: Some(1920),
          height: Some(1080),
          bitrate: Some(5_000_000),
          framerate: Some(30.0),
          alt_group: Some(1),
          is_live: Some(true),
          init_data: None,
          sample_rate: None,
          channel_config: None,
        },
        Track {
          name: "Audio Track".to_string(),
          render_group: 1,
          packaging: "loc".to_string(),
          codec: "aac".to_string(),
          width: None,
          height: None,
          bitrate: Some(128_000),
          framerate: None,
          alt_group: None,
          is_live: Some(true),
          init_data: Some("AQID".to_string()),
          sample_rate: Some(44_100),
          channel_config: Some("stereo".to_string()),
        },
      ],
    };

    let serialized = serde_json::to_string(&catalog).unwrap();

    assert!(serialized.contains("\"version\":1"));
    assert!(serialized.contains("\"deltaUpdate\":true"));
    assert!(serialized.contains("\"initData\":\"AQID\""));
    assert!(serialized.contains("\"name\":\"Track 1\""));
    assert!(serialized.contains("\"codec\":\"h264\""));
  }

  #[test]
  fn test_catalog_deserialization() {
    let json_data = r#"
      {
        "version": 1,
        "deltaUpdate": true,
        "tracks": [
          {
            "name": "Track 1",
            "renderGroup": 1,
            "packaging": "loc",
            "codec": "h264",
            "width": 1920,
            "height": 1080,
            "bitrate": 5000000,
            "framerate": 30.0,
            "altGroup": 1,
            "isLive": true
          },
          {
            "name": "Audio Track",
            "renderGroup": 1,
            "packaging": "loc",
            "codec": "aac",
            "bitrate": 128000,
            "samplerate": 44100,
            "channelConfig": "stereo",
            "initData": "AQID"
          }
        ]
      }
    "#;

    let catalog: MsfCatalog = serde_json::from_str(json_data).unwrap();

    assert_eq!(catalog.version, 1);
    assert_eq!(catalog.delta_update, Some(true));
    assert_eq!(catalog.tracks.len(), 2);

    let track1 = &catalog.tracks[0];
    assert_eq!(track1.name, "Track 1");
    assert_eq!(track1.codec, "h264");
    assert_eq!(track1.width, Some(1920));
    assert_eq!(track1.height, Some(1080));

    let track2 = &catalog.tracks[1];
    assert_eq!(track2.name, "Audio Track");
    assert_eq!(track2.codec, "aac");
    assert_eq!(track2.sample_rate, Some(44_100));
    assert_eq!(track2.channel_config.as_deref(), Some("stereo"));
    assert_eq!(track2.init_data.as_deref(), Some("AQID"));
  }
}
