use anyhow::{Context, Result};
use std::path::Path;

pub struct VideoInfo {
  pub width: u16,
  pub height: u16,
  pub framerate: f64,
}

pub async fn get_video_info(video_path: &str) -> Result<VideoInfo> {
  let path = video_path.to_owned();
  tokio::task::spawn_blocking(move || read_video_info(&path))
    .await
    .context("video info task panicked")?
}

fn read_video_info(video_path: &str) -> Result<VideoInfo> {
  let path = Path::new(video_path);
  let f = std::fs::File::open(path).with_context(|| format!("failed to open {video_path}"))?;
  let size = f.metadata()?.len();
  let reader = std::io::BufReader::new(f);
  let mp4 = mp4::Mp4Reader::read_header(reader, size)
    .with_context(|| format!("failed to read MP4 header from {video_path}"))?;

  for track in mp4.tracks().values() {
    if track.track_type().ok() == Some(mp4::TrackType::Video) {
      return Ok(VideoInfo {
        width: track.width(),
        height: track.height(),
        framerate: track.frame_rate(),
      });
    }
  }

  anyhow::bail!("no video track found in {video_path}")
}
