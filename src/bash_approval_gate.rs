use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::{
    bash_approval_policy::{
        BashApprovalPolicy, BashApprovalPolicyMatcher, BashApprovalPolicyStore,
    },
    bash_model_classifier::ModelRiskClassifier,
    bash_risk::{BashRiskAssessor, BashRiskDecision, BashRiskLevel, BashRiskRequest},
    config::{BashApprovalMode, BashConfig},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BashApprovalSummary {
    pub mode: String,
    pub risk: String,
    pub approved_by: String,
    pub policy_id: Option<String>,
    pub reason: String,
}

#[derive(Debug, Clone)]
pub enum BashApprovalAction {
    Approved,
    RequiresConfirmation { can_remember_policy: bool },
    Blocked { reason: String },
}

#[derive(Debug, Clone)]
pub struct BashGateDecision {
    pub risk: BashRiskLevel,
    pub action: BashApprovalAction,
    pub summary: Option<BashApprovalSummary>,
    pub risk_decision: BashRiskDecision,
}

pub struct BashApprovalGate {
    config: BashConfig,
    assessor: BashRiskAssessor,
    store: BashApprovalPolicyStore,
    model: Option<ModelRiskClassifier>,
}

impl BashApprovalGate {
    pub fn new(config: BashConfig, api_key: Option<String>) -> Self {
        let store = BashApprovalPolicyStore::load_or_create(
            config.approval_policy_path.clone(),
            config.approval_policy_ttl_days,
        )
        .unwrap_or_else(|error| {
            eprintln!("bash approval> failed to load policy store: {error}");
            let fallback = std::env::temp_dir().join("sparrow_bash_approval_policies.json");
            BashApprovalPolicyStore::load_or_create(fallback, config.approval_policy_ttl_days)
                .expect("fallback policy store should be writable")
        });
        let model = api_key
            .filter(|key| !key.trim().is_empty())
            .map(|key| ModelRiskClassifier::new(&key, config.model_low_risk_threshold));
        Self {
            config,
            assessor: BashRiskAssessor::new(),
            store,
            model,
        }
    }

    pub async fn decide(
        &mut self,
        command: &str,
        cwd: &Path,
        timeout_ms: u64,
    ) -> Result<BashGateDecision> {
        let request = BashRiskRequest {
            command: command.into(),
            cwd: cwd.to_path_buf(),
            allowed_roots: self.config.roots.clone(),
            timeout_ms,
        };
        let risk_decision = self.assessor.classify(request.clone());

        if risk_decision.risk == BashRiskLevel::Blocked {
            return Ok(self.blocked(risk_decision));
        }

        if self.config.approval_mode == BashApprovalMode::NeverPrompt {
            return Ok(self.approved(
                risk_decision.risk,
                "never_prompt",
                None,
                risk_decision.reason.clone(),
                risk_decision,
            ));
        }

        if self.config.approval_mode == BashApprovalMode::AlwaysPrompt {
            return Ok(self.requires_confirmation(risk_decision));
        }

        if risk_decision.risk != BashRiskLevel::High {
            if let Some(policy) = self
                .store
                .find_matching_low_risk_policy(command, cwd)
                .unwrap_or_else(|error| {
                    eprintln!("bash approval> failed to read policy store: {error}");
                    None
                })
            {
                let hard_check = self.assessor.classify(BashRiskRequest {
                    command: command.into(),
                    cwd: cwd.to_path_buf(),
                    allowed_roots: self.config.roots.clone(),
                    timeout_ms,
                });
                if matches!(
                    hard_check.risk,
                    BashRiskLevel::High | BashRiskLevel::Blocked
                ) {
                    return Ok(self.requires_confirmation(hard_check));
                }
                return Ok(self.approved(
                    BashRiskLevel::Low,
                    "policy_cache",
                    Some(policy.id),
                    policy.reason,
                    risk_decision,
                ));
            }
        }

        match risk_decision.risk {
            BashRiskLevel::Low => Ok(self.approved(
                BashRiskLevel::Low,
                "local_rule",
                None,
                risk_decision.reason.clone(),
                risk_decision,
            )),
            BashRiskLevel::Medium => {
                if let Some(model) = &self.model {
                    match model.classify(&request).await {
                        Ok(Some(model_response)) if model_response.risk == BashRiskLevel::Low => {
                            let policy_id = if let Some(candidate) =
                                &model_response.policy_candidate
                            {
                                let matcher = BashApprovalPolicyMatcher::from_candidate(candidate);
                                let policy = BashApprovalPolicy::new(
                                    matcher,
                                    cwd.to_path_buf(),
                                    BashRiskLevel::Low,
                                    "model".into(),
                                    model_response.confidence,
                                    model_response.reason.clone(),
                                    self.store.default_expiry(),
                                );
                                self.store.add_policy(policy).ok()
                            } else {
                                None
                            };
                            return Ok(self.approved(
                                BashRiskLevel::Low,
                                "model",
                                policy_id,
                                model_response.reason,
                                risk_decision,
                            ));
                        }
                        Ok(_) => {}
                        Err(error) => {
                            eprintln!("bash approval> model classification failed: {error}")
                        }
                    }
                }
                Ok(self.requires_confirmation(risk_decision))
            }
            BashRiskLevel::High => Ok(self.requires_confirmation(risk_decision)),
            BashRiskLevel::Blocked => unreachable!(),
        }
    }

    pub fn remember_policy(&mut self, decision: &BashGateDecision, cwd: PathBuf) -> Result<String> {
        let Some(candidate) = &decision.risk_decision.policy_candidate else {
            anyhow::bail!("approval decision has no policy candidate");
        };
        let policy = BashApprovalPolicy::new(
            BashApprovalPolicyMatcher::from_candidate(candidate),
            cwd,
            BashRiskLevel::Low,
            "user_policy".into(),
            decision.risk_decision.confidence,
            decision.risk_decision.reason.clone(),
            self.store.default_expiry(),
        );
        self.store.add_policy(policy)
    }

    fn blocked(&self, risk_decision: BashRiskDecision) -> BashGateDecision {
        BashGateDecision {
            risk: BashRiskLevel::Blocked,
            action: BashApprovalAction::Blocked {
                reason: risk_decision.reason.clone(),
            },
            summary: Some(summary(
                self.config.approval_mode,
                BashRiskLevel::Blocked,
                "blocked",
                None,
                risk_decision.reason.clone(),
            )),
            risk_decision,
        }
    }

    fn requires_confirmation(&self, risk_decision: BashRiskDecision) -> BashGateDecision {
        let can_remember_policy =
            risk_decision.risk == BashRiskLevel::Medium && risk_decision.policy_candidate.is_some();
        BashGateDecision {
            risk: risk_decision.risk,
            action: BashApprovalAction::RequiresConfirmation {
                can_remember_policy,
            },
            summary: Some(summary(
                self.config.approval_mode,
                risk_decision.risk,
                "requires_confirmation",
                None,
                risk_decision.reason.clone(),
            )),
            risk_decision,
        }
    }

    fn approved(
        &self,
        risk: BashRiskLevel,
        approved_by: &str,
        policy_id: Option<String>,
        reason: String,
        risk_decision: BashRiskDecision,
    ) -> BashGateDecision {
        BashGateDecision {
            risk,
            action: BashApprovalAction::Approved,
            summary: Some(summary(
                self.config.approval_mode,
                risk,
                approved_by,
                policy_id,
                reason,
            )),
            risk_decision,
        }
    }
}

fn summary(
    mode: BashApprovalMode,
    risk: BashRiskLevel,
    approved_by: &str,
    policy_id: Option<String>,
    reason: String,
) -> BashApprovalSummary {
    BashApprovalSummary {
        mode: mode.as_str().into(),
        risk: risk_label(risk).into(),
        approved_by: approved_by.into(),
        policy_id,
        reason,
    }
}

fn risk_label(risk: BashRiskLevel) -> &'static str {
    match risk {
        BashRiskLevel::Low => "low",
        BashRiskLevel::Medium => "medium",
        BashRiskLevel::High => "high",
        BashRiskLevel::Blocked => "blocked",
    }
}
