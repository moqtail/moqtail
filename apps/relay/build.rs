use std::process::Command;

fn main() {
  let commit = Command::new("git")
    .args(["rev-parse", "--short", "HEAD"])
    .output()
    .ok()
    .and_then(|output| String::from_utf8(output.stdout).ok())
    .map(|s| s.trim().to_string())
    .unwrap_or_else(|| "unknown".to_string());

  let version = env!("CARGO_PKG_VERSION");
  let moqtail_version = format!("moqtail/{version}+{commit}");

  println!("cargo:rustc-env=MOQTAIL_VERSION={}", moqtail_version);
}
