use std::{fs, path::PathBuf};

use chrono::{Duration, Utc};
use sparrow_agent::{
    bash_approval_policy::{
        BashApprovalPolicy, BashApprovalPolicyMatcher, BashApprovalPolicyStore,
    },
    bash_risk::BashRiskLevel,
};

fn policy(
    matcher: BashApprovalPolicyMatcher,
    cwd_scope: PathBuf,
    expires_at: chrono::DateTime<Utc>,
) -> BashApprovalPolicy {
    BashApprovalPolicy::new(
        matcher,
        cwd_scope,
        BashRiskLevel::Low,
        "test".into(),
        1.0,
        "test policy".into(),
        expires_at,
    )
}

#[test]
fn store_creates_empty_policy_file_with_private_permissions() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("policies.json");

    let store = BashApprovalPolicyStore::load_or_create(path.clone(), 90).unwrap();

    assert_eq!(store.policies().len(), 0);
    assert!(path.exists());

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}

#[test]
fn argv_prefix_policy_matches_only_inside_cwd_scope_and_before_expiry() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("policies.json");
    let workspace = root.path().join("workspace");
    fs::create_dir(&workspace).unwrap();
    let mut store = BashApprovalPolicyStore::load_or_create(path, 90).unwrap();
    let policy = policy(
        BashApprovalPolicyMatcher::ArgvPrefix {
            program: "git".into(),
            args: vec!["status".into()],
        },
        workspace.clone(),
        Utc::now() + Duration::days(1),
    );
    store.add_policy(policy).unwrap();

    assert!(
        store
            .find_matching_low_risk_policy("git status --short", &workspace)
            .unwrap()
            .is_some()
    );
    assert!(
        store
            .find_matching_low_risk_policy("git reset --hard", &workspace)
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .find_matching_low_risk_policy("git status --short > status.txt", &workspace)
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .find_matching_low_risk_policy("git status", root.path())
            .unwrap()
            .is_none()
    );
}

#[test]
fn expired_policy_does_not_match() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("policies.json");
    let mut store = BashApprovalPolicyStore::load_or_create(path, 90).unwrap();
    store
        .add_policy(policy(
            BashApprovalPolicyMatcher::ArgvExact {
                program: "cargo".into(),
                args: vec!["test".into()],
            },
            root.path().to_path_buf(),
            Utc::now() - Duration::days(1),
        ))
        .unwrap();

    assert!(
        store
            .find_matching_low_risk_policy("cargo test", root.path())
            .unwrap()
            .is_none()
    );
}
