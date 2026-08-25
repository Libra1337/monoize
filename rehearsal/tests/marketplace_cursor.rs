use monoize_lynshen_rehearsal::marketplace::{
    CursorError, EndpointKind, ListCursor, canonical_filter_digest,
};

const KEY: [u8; 32] = [7; 32];

fn digest(query: &str, group: &str) -> [u8; 32] {
    canonical_filter_digest(EndpointKind::List, &[(1, query), (2, group)])
}

#[test]
fn cursor_round_trip_preserves_public_sort_key() {
    let cursor =
        ListCursor::new(42, 24, digest("gpt", "public"), 3, "GPT-4o-模型").expect("valid cursor");
    let encoded = cursor.encode(&KEY).expect("encode cursor");
    let decoded =
        ListCursor::decode(&encoded, &KEY, 42, 24, digest("gpt", "public")).expect("decode cursor");

    assert_eq!(decoded, cursor);
    assert!(encoded.len() <= 512);
    assert!(!encoded.contains("provider-internal-id"));
}

#[test]
fn cursor_rejects_tampering_filter_and_limit_changes() {
    let cursor = ListCursor::new(42, 24, digest("gpt", "public"), 3, "gpt-4o")
        .unwrap()
        .encode(&KEY)
        .unwrap();
    let mut tampered = cursor.clone();
    let replacement = if tampered.ends_with('A') { 'B' } else { 'A' };
    tampered.pop();
    tampered.push(replacement);

    assert_eq!(
        ListCursor::decode(&tampered, &KEY, 42, 24, digest("gpt", "public")).unwrap_err(),
        CursorError::Invalid
    );
    assert_eq!(
        ListCursor::decode(&cursor, &KEY, 42, 24, digest("other", "public")).unwrap_err(),
        CursorError::Invalid
    );
    assert_eq!(
        ListCursor::decode(&cursor, &KEY, 42, 50, digest("gpt", "public")).unwrap_err(),
        CursorError::Invalid
    );
}

#[test]
fn valid_signature_with_changed_revision_is_stale() {
    let cursor = ListCursor::new(42, 24, digest("gpt", "public"), 3, "gpt-4o")
        .unwrap()
        .encode(&KEY)
        .unwrap();
    assert_eq!(
        ListCursor::decode(&cursor, &KEY, 43, 24, digest("gpt", "public")).unwrap_err(),
        CursorError::Stale
    );
}

#[test]
fn rejects_invalid_model_and_oversized_ascii_cursor() {
    assert_eq!(
        ListCursor::new(1, 24, digest("", ""), 0, "\n").unwrap_err(),
        CursorError::Invalid
    );
    assert_eq!(
        ListCursor::new(1, 24, digest("", ""), 0, &"a".repeat(400)).unwrap_err(),
        CursorError::Invalid
    );
}
