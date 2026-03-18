#[cfg(test)]
mod tests {
    use crate::api::state::AppState;

    #[tokio::test]
    async fn queue_enqueue_dequeue() {
        use crate::api::state::QueuedFile;
        use bytes::Bytes;
        use uuid::Uuid;

        let state = AppState::new(false);
        assert_eq!(state.queue_depth().await, 0);

        state.enqueue(QueuedFile {
            queued_id: Uuid::new_v4(),
            bytes: Bytes::from_static(b"hello"),
        }).await;

        assert_eq!(state.queue_depth().await, 1);
        let file = state.dequeue().await.unwrap();
        assert_eq!(file.bytes.as_ref(), b"hello");
        assert!(state.dequeue().await.is_none());
    }
}
