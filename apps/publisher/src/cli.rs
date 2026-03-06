use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "publisher", author, version, about = "MOQtail publisher")]
pub struct Cli {
  /// Relay endpoint URL
  #[arg(default_value = "https://127.0.0.1:4433")]
  pub endpoint: String,

  /// Validate TLS certificate
  #[arg(long, default_value_t = false)]
  pub validate_cert: bool,

  /// Track namespace
  #[arg(long, short, default_value = "moqtail")]
  pub namespace: String,

  /// Path to video file
  #[arg(long, default_value = "data/video/Smoking Test.mp4")]
  pub video_path: String,
}
