use crate::video::VideoInfo;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Quality {
  Q360p,
  Q480p,
  Q720p,
  Q1080p,
}

#[derive(Debug, Clone)]
pub struct QualityVariant {
  pub quality: Quality,
  pub width: u16,
  pub height: u16,
  pub bitrate_kbps: u32,
}

/// CBR bitrate ladder (highest tier = 2 Mbps).
const VARIANTS: &[(Quality, u16, u16, u32)] = &[
  (Quality::Q1080p, 1920, 1080, 2000),
  (Quality::Q720p, 1280, 720, 1200),
  (Quality::Q480p, 854, 480, 700),
  (Quality::Q360p, 640, 360, 400),
];

/// Returns the quality variants available for the given source video.
/// Only includes tiers at or below the source resolution.
/// Returns an error if no variants match (source below 360p).
pub fn quality_variants(info: &VideoInfo) -> anyhow::Result<Vec<QualityVariant>> {
  let variants: Vec<QualityVariant> = VARIANTS
    .iter()
    .filter(|(_, _, h, _)| *h <= info.height)
    .map(|&(quality, width, height, bitrate_kbps)| QualityVariant {
      quality,
      width,
      height,
      bitrate_kbps,
    })
    .collect();

  if variants.is_empty() {
    anyhow::bail!(
      "source resolution {}x{} is below the minimum supported tier (360p)",
      info.width,
      info.height
    );
  }

  Ok(variants)
}

impl std::fmt::Display for Quality {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match self {
      Quality::Q360p => write!(f, "360p"),
      Quality::Q480p => write!(f, "480p"),
      Quality::Q720p => write!(f, "720p"),
      Quality::Q1080p => write!(f, "1080p"),
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn video(width: u16, height: u16) -> VideoInfo {
    VideoInfo {
      width,
      height,
      framerate: 30.0,
    }
  }

  #[test]
  fn test_1080p_source_returns_all_variants() {
    let variants = quality_variants(&video(1920, 1080)).unwrap();
    assert_eq!(variants.len(), 4);
    assert_eq!(variants[0].quality, Quality::Q1080p);
    assert_eq!(variants[1].quality, Quality::Q720p);
    assert_eq!(variants[2].quality, Quality::Q480p);
    assert_eq!(variants[3].quality, Quality::Q360p);
  }

  #[test]
  fn test_720p_source_excludes_1080p() {
    let variants = quality_variants(&video(1280, 720)).unwrap();
    assert_eq!(variants.len(), 3);
    assert_eq!(variants[0].quality, Quality::Q720p);
    assert_eq!(variants[1].quality, Quality::Q480p);
    assert_eq!(variants[2].quality, Quality::Q360p);
  }

  #[test]
  fn test_480p_source_excludes_720p_and_above() {
    let variants = quality_variants(&video(854, 480)).unwrap();
    assert_eq!(variants.len(), 2);
    assert_eq!(variants[0].quality, Quality::Q480p);
    assert_eq!(variants[1].quality, Quality::Q360p);
  }

  #[test]
  fn test_360p_source_returns_only_360p() {
    let variants = quality_variants(&video(640, 360)).unwrap();
    assert_eq!(variants.len(), 1);
    assert_eq!(variants[0].quality, Quality::Q360p);
  }

  #[test]
  fn test_below_360p_returns_error() {
    let result = quality_variants(&video(320, 240));
    assert!(result.is_err());
  }

  #[test]
  fn test_bitrate_ladder_values() {
    let variants = quality_variants(&video(1920, 1080)).unwrap();
    assert_eq!(variants[0].bitrate_kbps, 2000);
    assert_eq!(variants[1].bitrate_kbps, 1200);
    assert_eq!(variants[2].bitrate_kbps, 700);
    assert_eq!(variants[3].bitrate_kbps, 400);
  }

  #[test]
  fn test_resolution_values() {
    let variants = quality_variants(&video(1920, 1080)).unwrap();
    assert_eq!((variants[0].width, variants[0].height), (1920, 1080));
    assert_eq!((variants[1].width, variants[1].height), (1280, 720));
    assert_eq!((variants[2].width, variants[2].height), (854, 480));
    assert_eq!((variants[3].width, variants[3].height), (640, 360));
  }

  #[test]
  fn test_non_standard_source_includes_lower_tiers() {
    // e.g. 900p source should include 720p, 480p, 360p but not 1080p
    let variants = quality_variants(&video(1600, 900)).unwrap();
    assert_eq!(variants.len(), 3);
    assert_eq!(variants[0].quality, Quality::Q720p);
  }

  #[test]
  fn test_quality_display() {
    assert_eq!(format!("{}", Quality::Q1080p), "1080p");
    assert_eq!(format!("{}", Quality::Q720p), "720p");
    assert_eq!(format!("{}", Quality::Q480p), "480p");
    assert_eq!(format!("{}", Quality::Q360p), "360p");
  }
}
