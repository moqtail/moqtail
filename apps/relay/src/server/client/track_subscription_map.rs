use crate::server::subscription::Subscription;
use moqtail::model::data::full_track_name::FullTrackName;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Weak;
use tokio::sync::RwLock;

#[derive(Clone, Debug)]
pub struct TrackSubscriptionMap {
  pub map: Arc<RwLock<HashMap<FullTrackName, Weak<Subscription>>>>,
}

impl TrackSubscriptionMap {
  pub fn new() -> Self {
    Self {
      map: Arc::new(RwLock::new(HashMap::new())),
    }
  }

  pub async fn add_subscription(
    &self,
    track_name: FullTrackName,
    subscription: Weak<Subscription>,
  ) {
    let mut map = self.map.write().await;
    map.insert(track_name, subscription);
  }

  pub async fn get_subscription(&self, track_name: &FullTrackName) -> Option<Weak<Subscription>> {
    let map = self.map.read().await;
    map.get(track_name).cloned()
  }

  pub async fn remove_subscription(&self, track_name: &FullTrackName) -> bool {
    let mut map = self.map.write().await;
    map.remove(track_name).is_some()
  }
}
