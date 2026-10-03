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
MOQT Streaming Format Draft, Section 4
https://datatracker.ietf.org/doc/draft-ietf-moq-msf/01/
*/

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Catalog {
  pub version: String,
  #[serde(default)]
  pub supports_delta_updates: bool,
  pub tracks: Vec<Track>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
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
  pub framerate: Option<u32>,

  #[serde(skip_serializing_if = "Option::is_none")]
  pub alt_group: Option<u8>,

  #[serde(skip_serializing_if = "Option::is_none")]
  pub samplerate: Option<u32>,

  #[serde(skip_serializing_if = "Option::is_none")]
  pub channel_config: Option<String>,
}

#[cfg(test)]
mod tests {
  use super::*;
  #[test]
  fn test_empty_catalog_serialization() {
    let catalog = Catalog {
      version: "draft-01".to_string(),
      supports_delta_updates: false,
      tracks: vec![],
    };

    let serialized = serde_json::to_string(&catalog).unwrap();

    assert!(serialized.contains("\"version\":\"draft-01\""));
    assert!(serialized.contains("\"supports_delta_updates\":false"));
    assert!(serialized.contains("\"tracks\":[]"));
  }

  #[test]
  fn test_track_with_minimal_fields() {
    let track = Track {
      name: "Minimal Track".to_string(),
      render_group: 1,
      packaging: "mp4".to_string(),
      codec: "h264".to_string(),
      width: None,
      height: None,
      bitrate: None,
      framerate: None,
      alt_group: None,
      samplerate: None,
      channel_config: None,
    };

    let serialized = serde_json::to_string(&track).unwrap();

    assert!(serialized.contains("\"name\":\"Minimal Track\""));
    assert!(serialized.contains("\"render_group\":1"));
    assert!(serialized.contains("\"packaging\":\"mp4\""));
    assert!(serialized.contains("\"codec\":\"h264\""));
    assert!(!serialized.contains("\"width\""));
    assert!(!serialized.contains("\"height\""));
    assert!(!serialized.contains("\"bitrate\""));
    assert!(!serialized.contains("\"framerate\""));
    assert!(!serialized.contains("\"alt_group\""));
    assert!(!serialized.contains("\"samplerate\""));
    assert!(!serialized.contains("\"channel_config\""));
  }

  #[test]
  fn test_catalog_with_mixed_tracks() {
    let catalog = Catalog {
      version: "draft-01".to_string(),
      supports_delta_updates: true,
      tracks: vec![
        Track {
          name: "Video Track".to_string(),
          render_group: 1,
          packaging: "mp4".to_string(),
          codec: "h265".to_string(),
          width: Some(1280),
          height: Some(720),
          bitrate: Some(3000),
          framerate: Some(60),
          alt_group: None,
          samplerate: None,
          channel_config: None,
        },
        Track {
          name: "Audio Track".to_string(),
          render_group: 2,
          packaging: "aac".to_string(),
          codec: "aac".to_string(),
          width: None,
          height: None,
          bitrate: Some(128),
          framerate: None,
          alt_group: None,
          samplerate: Some(48000),
          channel_config: Some("5.1".to_string()),
        },
      ],
    };

    let serialized = serde_json::to_string(&catalog).unwrap();

    assert!(serialized.contains("\"version\":\"draft-01\""));
    assert!(serialized.contains("\"supports_delta_updates\":true"));
    assert!(serialized.contains("\"name\":\"Video Track\""));
    assert!(serialized.contains("\"codec\":\"h265\""));
    assert!(serialized.contains("\"width\":1280"));
    assert!(serialized.contains("\"height\":720"));
    assert!(serialized.contains("\"bitrate\":3000"));
    assert!(serialized.contains("\"framerate\":60"));
    assert!(serialized.contains("\"name\":\"Audio Track\""));
    assert!(serialized.contains("\"codec\":\"aac\""));
    assert!(serialized.contains("\"samplerate\":48000"));
    assert!(serialized.contains("\"channel_config\":\"5.1\""));
  }

  #[test]
  fn test_invalid_catalog_deserialization() {
    let invalid_json_data = r#"
      {
        "version": 1,
        "supports_delta_updates": true,
        "tracks": []
      }
    "#;

    let result: Result<Catalog, _> = serde_json::from_str(invalid_json_data);
    assert!(result.is_err());
  }

  #[test]
  fn test_track_with_partial_fields_deserialization() {
    let json_data = r#"
      {
        "name": "Partial Track",
        "render_group": 1,
        "packaging": "mp4",
        "codec": "vp9",
        "width": 640,
        "height": 360
      }
    "#;

    let track: Track = serde_json::from_str(json_data).unwrap();

    assert_eq!(track.name, "Partial Track");
    assert_eq!(track.render_group, 1);
    assert_eq!(track.packaging, "mp4");
    assert_eq!(track.codec, "vp9");
    assert_eq!(track.width, Some(640));
    assert_eq!(track.height, Some(360));
    assert_eq!(track.bitrate, None);
    assert_eq!(track.framerate, None);
    assert_eq!(track.alt_group, None);
    assert_eq!(track.samplerate, None);
    assert_eq!(track.channel_config, None);
  }
  #[test]
  fn test_catalog_serialization() {
    let catalog = Catalog {
      version: "draft-01".to_string(),
      supports_delta_updates: true,
      tracks: vec![
        Track {
          name: "Track 1".to_string(),
          render_group: 1,
          packaging: "mp4".to_string(),
          codec: "h264".to_string(),
          width: Some(1920),
          height: Some(1080),
          bitrate: Some(5000),
          framerate: Some(30),
          alt_group: Some(1),
          samplerate: None,
          channel_config: None,
        },
        Track {
          name: "Audio Track".to_string(),
          render_group: 2,
          packaging: "aac".to_string(),
          codec: "aac".to_string(),
          width: None,
          height: None,
          bitrate: Some(128),
          framerate: None,
          alt_group: None,
          samplerate: Some(44100),
          channel_config: Some("stereo".to_string()),
        },
      ],
    };

    let serialized = serde_json::to_string(&catalog).unwrap();

    assert!(serialized.contains("\"version\":\"draft-01\""));
    assert!(serialized.contains("\"supports_delta_updates\":true"));
    assert!(serialized.contains("\"name\":\"Track 1\""));
    assert!(serialized.contains("\"codec\":\"h264\""));
  }

  #[test]
  fn test_catalog_deserialization() {
    let json_data = r#"
      {
        "version": "draft-01",
        "supports_delta_updates": true,
        "tracks": [
          {
            "name": "Track 1",
            "render_group": 1,
            "packaging": "mp4",
            "codec": "h264",
            "width": 1920,
            "height": 1080,
            "bitrate": 5000,
            "framerate": 30,
            "alt_group": 1
          },
          {
            "name": "Audio Track",
            "render_group": 2,
            "packaging": "aac",
            "codec": "aac",
            "bitrate": 128,
            "samplerate": 44100,
            "channel_config": "stereo"
          }
        ]
      }
      "#;

    let catalog: Catalog = serde_json::from_str(json_data).unwrap();

    assert_eq!(catalog.version, "draft-01");
    assert!(catalog.supports_delta_updates);
    assert_eq!(catalog.tracks.len(), 2);

    let track1 = &catalog.tracks[0];
    assert_eq!(track1.name, "Track 1");
    assert_eq!(track1.codec, "h264");
    assert_eq!(track1.width, Some(1920));
    assert_eq!(track1.height, Some(1080));

    let track2 = &catalog.tracks[1];
    assert_eq!(track2.name, "Audio Track");
    assert_eq!(track2.codec, "aac");
    assert_eq!(track2.samplerate, Some(44100));
    assert_eq!(track2.channel_config.as_deref(), Some("stereo"));
  }
}
