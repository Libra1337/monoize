#[cfg(test)]
mod quota_error_tests {
    use super::*;
    use serde_json::json;
    use tokio::sync::mpsc;

    // SAN-11a: quota-classified Responses streams collapse to the fixed
    // generic text in `response.failed` and drop the replayed upstream error
    // object, for every model and regardless of the masking switch.
    #[tokio::test]
    async fn responses_stream_quota_error_uses_generic_text() {
        let (event_tx, event_rx) = mpsc::channel(8);
        let (sse_tx, mut sse_rx) = mpsc::channel(8);

        event_tx
            .send(UrpStreamEvent::Error {
                code: Some("rate_limit_error".to_string()),
                message: "upstream status 429: 5-hour quota exceeded".to_string(),
                extra_body: HashMap::from([(
                    "error".to_string(),
                    json!({
                        "code": "quota_exceeded",
                        "message": "You have exceeded your 5 hour quota; resets 2026-09-15T21:00:00Z"
                    }),
                )]),
            })
            .await
            .expect("error event");
        drop(event_tx);

        encode_urp_stream_as_responses(
            event_rx,
            sse_tx,
            "glm-5.3",
            std::time::Instant::now(),
            None,
            false,
        )
        .await
        .expect("encode responses stream");

        let mut text = String::new();
        while let Some(event) = sse_rx.recv().await {
            text.push_str(&format!("{event:?}"));
        }
        assert!(
            text.contains(crate::error_sanitize::GENERIC_QUOTA_TEXT),
            "{text}"
        );
        assert!(!text.contains("5 hour"), "{text}");
        assert!(!text.contains("5-hour"), "{text}");
        assert!(!text.contains("resets 2026"), "{text}");
        assert!(!text.contains("quota_exceeded"), "{text}");
    }
}
