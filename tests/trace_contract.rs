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
fn json_snapshot_wraps_non_json_text() {
    let snapshot = JsonSnapshot::from_text("plain tool output", 64);

    assert!(!snapshot.truncated);
    assert_eq!(snapshot.value["raw"], "plain tool output");
    assert_eq!(snapshot.text, "plain tool output");
}
