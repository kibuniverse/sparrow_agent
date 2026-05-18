use std::path::PathBuf;

use sparrow_agent::{
    bash_approval_gate::{BashApprovalAction, BashApprovalGate},
    bash_approval_policy::{
        BashApprovalPolicy, BashApprovalPolicyMatcher, BashApprovalPolicyStore,
    },
    bash_risk::BashRiskLevel,
    config::{BashApprovalMode, BashConfig},
};

fn config(root: PathBuf, policy_path: PathBuf, mode: BashApprovalMode) -> BashConfig {
    BashConfig {
        enabled: true,
        roots: vec![root],
        approval_mode: mode,
        approval_policy_path: policy_path,
        approval_policy_ttl_days: 90,
        model_low_risk_threshold: 0.85,
        timeout_ms: 30_000,
        max_timeout_ms: 120_000,
        max_command_chars: 8_192,
        stream_max_bytes: 8 * 1024,
        env_allowlist: vec!["PATH".into()],
    }
}

#[tokio::test]
async fn smart_mode_auto_approves_low_risk_local_rule() {
    let root = tempfile::tempdir().unwrap();
    let mut gate = BashApprovalGate::new(
        config(
            root.path().to_path_buf(),
            root.path().join("policies.json"),
            BashApprovalMode::Smart,
        ),
        None,
    );

    let decision = gate
        .decide("git status --short", root.path(), 30_000)
        .await
        .unwrap();

    assert_eq!(decision.risk, BashRiskLevel::Low);
    assert!(matches!(decision.action, BashApprovalAction::Approved));
    assert_eq!(decision.summary.unwrap().approved_by, "local_rule");
}

#[tokio::test]
async fn smart_mode_denies_blocked_even_when_never_prompt_would_run_other_commands() {
    let root = tempfile::tempdir().unwrap();
    let mut gate = BashApprovalGate::new(
        config(
            root.path().to_path_buf(),
            root.path().join("policies.json"),
            BashApprovalMode::NeverPrompt,
        ),
        None,
    );

    let decision = gate.decide("rm -rf /", root.path(), 30_000).await.unwrap();

    assert_eq!(decision.risk, BashRiskLevel::Blocked);
    assert!(matches!(
        decision.action,
        BashApprovalAction::Blocked { .. }
    ));
}

#[tokio::test]
async fn policy_cache_can_auto_approve_medium_command_shape() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("policies.json");
    let mut store = BashApprovalPolicyStore::load_or_create(path.clone(), 90).unwrap();
    store
        .add_policy(BashApprovalPolicy::new(
            BashApprovalPolicyMatcher::ExactNormalizedCommand {
                command: "printf ok > output.txt".into(),
            },
            root.path().to_path_buf(),
            BashRiskLevel::Low,
            "user_policy".into(),
            1.0,
            "remembered exact command".into(),
            chrono::Utc::now() + chrono::Duration::days(1),
        ))
        .unwrap();
    let mut gate = BashApprovalGate::new(
        config(root.path().to_path_buf(), path, BashApprovalMode::Smart),
        None,
    );

    let decision = gate
        .decide("printf ok > output.txt", root.path(), 30_000)
        .await
        .unwrap();

    assert_eq!(decision.risk, BashRiskLevel::Low);
    assert!(matches!(decision.action, BashApprovalAction::Approved));
    assert_eq!(decision.summary.unwrap().approved_by, "policy_cache");
}
