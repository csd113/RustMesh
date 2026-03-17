//! Integration tests for the `RustWave` API layer.

use crate::api::state::{AppState, QueuedFile};
use bytes::Bytes;
use uuid::Uuid;

#[tokio::test]
async fn queue_enqueue_dequeue_roundtrip() {
    let state = AppState::new(false);
    assert_eq!(state.queue_depth().await, 0);

    state
        .enqueue(QueuedFile {
            queued_id: Uuid::new_v4(),
            bytes: Bytes::from_static(b"api-test"),
        })
        .await;

    assert_eq!(state.queue_depth().await, 1);
    assert!(state.dequeue().await.is_some());
    assert!(state.dequeue().await.is_none());
}

#[tokio::test]
async fn queue_preserves_fifo_order() {
    let state = AppState::new(true);

    for i in 0u8..3 {
        state
            .enqueue(QueuedFile {
                queued_id: Uuid::new_v4(),
                bytes: Bytes::from(vec![i]),
            })
            .await;
    }

    assert_eq!(state.queue_depth().await, 3);
    for expected in 0u8..3 {
        let dequeued = state.dequeue().await;
        assert!(dequeued.is_some(), "queue should have an item");
        if let Some(file) = dequeued {
            assert_eq!(
                file.bytes.first().copied(),
                Some(expected),
                "bytes non-empty"
            );
        }
    }
    assert_eq!(state.queue_depth().await, 0);
}
