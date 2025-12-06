use moqtail::model::data::full_track_name::FullTrackName;
use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[repr(u8)]
pub enum SwitchStatus {
  None,
  Current,
  Next,
}

pub type SwitchSetItems = Arc<RwLock<HashMap<FullTrackName, SwitchStatus>>>;

#[derive(Debug, Clone)]
pub struct SwitchSet {
  pub items: SwitchSetItems,
}

impl SwitchSet {
  pub fn new() -> Self {
    Self {
      items: Arc::new(RwLock::new(HashMap::new())),
    }
  }

  pub async fn add_or_update_switch_item(&self, track_name: FullTrackName, status: SwitchStatus) {
    let mut items = self.items.write().await;
    items.insert(track_name, status);
  }

  pub async fn get_switch_status(&self, track_name: &FullTrackName) -> Option<SwitchStatus> {
    let items = self.items.read().await;
    items.get(track_name).copied()
  }
}

impl fmt::Display for SwitchStatus {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    let status_str = match self {
      SwitchStatus::None => "None",
      SwitchStatus::Current => "Current",
      SwitchStatus::Next => "Next",
    };
    write!(f, "{}", status_str)
  }
}

impl fmt::Display for SwitchSet {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    let items = self.items.blocking_read();
    for (track_name, status) in items.iter() {
      writeln!(f, "{:?}: {}", track_name, status)?;
    }
    Ok(())
  }
}
