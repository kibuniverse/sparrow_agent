use sparrow_agent::trace::JsonSnapshot;

#[test]
fn json_snapshot_parses_redacts_and_truncates_text() {
    let snapshot = JsonSnapshot::from_text(
        r#"{"query":"repo","api_key":"secret","nested":{"token":"abc"}}"#,
        48,
    );

    assert!(snapshot.truncated);
    assert!(snapshot.text.len() <= 48);
    assert_eq!(snapshot.value["api_key"], "[REDACTED]");
    assert_eq!(snapshot.value["nested"]["token"], "[REDACTED]");
}

#[test]
fn json_snapshot_preserves_token_count_metrics() {
    let snapshot = JsonSnapshot::from_text(
        r#"{"reasoning_tokens":3,"total_tokens":15,"token":"secret"}"#,
        128,
    );

    assert_eq!(snapshot.value["reasoning_tokens"], 3);
    assert_eq!(snapshot.value["total_tokens"], 15);
    assert_eq!(snapshot.value["token"], "[REDACTED]");
}

#[test]
fn json_snapshot_wraps_non_json_text() {
    let snapshot = JsonSnapshot::from_text("plain tool output", 64);

    assert!(!snapshot.truncated);
    assert_eq!(snapshot.value["raw"], "plain tool output");
    assert_eq!(snapshot.text, "plain tool output");
}
