//! Shared state for the `RustWave` API server.

use bytes::Bytes;
use std::{collections::VecDeque, sync::Arc};
use tokio::sync::Mutex;
use uuid::Uuid;

#[derive(Debug)]
pub struct QueuedFile {
    pub queued_id: Uuid,
    pub bytes: Bytes,
}

pub type IncomingQueue = Arc<Mutex<VecDeque<QueuedFile>>>;

#[derive(Clone)]
pub struct AppState {
    pub broadcaster_url: String,
    pub channet_url: String,
    #[allow(dead_code)]
    pub wave_routes_enabled: bool,
    pub incoming_queue: IncomingQueue,
}

impl AppState {
    pub fn new(wave_routes_enabled: bool) -> Self {
        let broadcaster_url = std::env::var("RUSTWAVE_BROADCASTER_URL")
            .unwrap_or_else(|_| "http://localhost:9090".to_string());

        let channet_url = std::env::var("RUSTWAVE_CHANNET_URL")
            .unwrap_or_else(|_| "http://localhost:7070".to_string());

        Self {
            broadcaster_url,
            channet_url,
            wave_routes_enabled,
            incoming_queue: Arc::new(Mutex::new(VecDeque::new())),
        }
    }

    pub async fn queue_depth(&self) -> usize {
        self.incoming_queue.lock().await.len()
    }

    pub async fn enqueue(&self, file: QueuedFile) {
        self.incoming_queue.lock().await.push_back(file);
    }

    pub async fn dequeue(&self) -> Option<QueuedFile> {
        self.incoming_queue.lock().await.pop_front()
    }
}
