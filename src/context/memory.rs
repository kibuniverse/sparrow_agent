use chrono::{DateTime, Utc};

use crate::{
    api::ChatMessage,
    context::{artifact_ref::ContextArtifactRef, budget::TokenEstimator},
};

#[derive(Debug, Clone)]
pub struct WorkingMemory {
    pub task_goal: Option<String>,
    pub durable_facts: Vec<MemoryItem>,
    pub decisions: Vec<MemoryItem>,
    pub open_questions: Vec<MemoryItem>,
    pub artifact_refs: Vec<ContextArtifactRef>,
    pub updated_at: DateTime<Utc>,
}

impl Default for WorkingMemory {
    fn default() -> Self {
        Self {
            task_goal: None,
            durable_facts: Vec::new(),
            decisions: Vec::new(),
            open_questions: Vec::new(),
            artifact_refs: Vec::new(),
            updated_at: Utc::now(),
        }
    }
}

impl WorkingMemory {
    pub fn is_empty(&self) -> bool {
        self.task_goal
            .as_deref()
            .is_none_or(|goal| goal.trim().is_empty())
            && self.durable_facts.is_empty()
            && self.decisions.is_empty()
            && self.open_questions.is_empty()
            && self.artifact_refs.is_empty()
    }

    pub fn to_context_message(
        &self,
        estimator: &TokenEstimator,
        max_tokens: usize,
    ) -> Option<ChatMessage> {
        if self.is_empty() {
            return None;
        }

        let mut sections = Vec::new();
        if let Some(goal) = self
            .task_goal
            .as_deref()
            .filter(|goal| !goal.trim().is_empty())
        {
            sections.push(format!("Task goal:\n{goal}"));
        }
        push_items(&mut sections, "Durable facts", &self.durable_facts);
        push_items(&mut sections, "Decisions", &self.decisions);
        push_items(&mut sections, "Open questions", &self.open_questions);
        if !self.artifact_refs.is_empty() {
            let artifacts = self
                .artifact_refs
                .iter()
                .map(ContextArtifactRef::render)
                .collect::<Vec<_>>()
                .join("\n");
            sections.push(format!("Artifacts:\n{artifacts}"));
        }

        let content = format!(
            "Working memory for the ongoing task. Treat this as compressed context, not as a new user request.\n\n{}",
            sections.join("\n\n")
        );
        Some(ChatMessage::user(
            estimator.truncate_text_to_tokens(&content, max_tokens),
        ))
    }

    pub fn apply_delta(&mut self, delta: &MemoryDelta) -> DeltaOutcome {
        let outcome = match delta {
            MemoryDelta::SetGoal { goal } => {
                let goal = goal.trim();
                self.task_goal = if goal.is_empty() {
                    None
                } else {
                    Some(goal.to_string())
                };
                DeltaOutcome::Applied
            }
            MemoryDelta::AddFact { content } => push_non_empty(&mut self.durable_facts, content),
            MemoryDelta::AddDecision { content } => push_non_empty(&mut self.decisions, content),
            MemoryDelta::AddQuestion { content } => push_non_empty(&mut self.open_questions, content),
            MemoryDelta::ResolveQuestion { needle } => {
                remove_first_match(&mut self.open_questions, needle)
            }
            MemoryDelta::RemoveFact { needle } => remove_first_match(&mut self.durable_facts, needle),
            MemoryDelta::Clear { scope } => DeltaOutcome::Cleared(self.clear_scope(scope)),
        };

        if outcome != DeltaOutcome::NotFound {
            self.updated_at = Utc::now();
        }
        outcome
    }

    fn clear_scope(&mut self, scope: &MemoryScope) -> usize {
        match scope {
            MemoryScope::All => {
                let n = self.durable_facts.len() + self.decisions.len() + self.open_questions.len();
                self.durable_facts.clear();
                self.decisions.clear();
                self.open_questions.clear();
                self.task_goal = None;
                n
            }
            MemoryScope::Facts => clear_vec(&mut self.durable_facts),
            MemoryScope::Decisions => clear_vec(&mut self.decisions),
            MemoryScope::OpenQuestions => clear_vec(&mut self.open_questions),
        }
    }
}

#[derive(Debug, Clone)]
pub struct MemoryItem {
    pub content: String,
    pub created_at: DateTime<Utc>,
}

impl MemoryItem {
    pub fn new(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            created_at: Utc::now(),
        }
    }
}

fn push_items(sections: &mut Vec<String>, title: &str, items: &[MemoryItem]) {
    if items.is_empty() {
        return;
    }

    let rendered = items
        .iter()
        .map(|item| format!("- {}", item.content))
        .collect::<Vec<_>>()
        .join("\n");
    sections.push(format!("{title}:\n{rendered}"));
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemoryScope {
    All,
    Facts,
    Decisions,
    OpenQuestions,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemoryDelta {
    SetGoal { goal: String },
    AddFact { content: String },
    AddDecision { content: String },
    AddQuestion { content: String },
    ResolveQuestion { needle: String },
    RemoveFact { needle: String },
    Clear { scope: MemoryScope },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeltaOutcome {
    Applied,
    NotFound,
    Cleared(usize),
}

fn push_non_empty(items: &mut Vec<MemoryItem>, content: &str) -> DeltaOutcome {
    let content = content.trim();
    if content.is_empty() {
        DeltaOutcome::NotFound
    } else {
        items.push(MemoryItem::new(content));
        DeltaOutcome::Applied
    }
}

fn remove_first_match(items: &mut Vec<MemoryItem>, needle: &str) -> DeltaOutcome {
    let needle = needle.trim();
    if needle.is_empty() {
        return DeltaOutcome::NotFound;
    }
    let lower = needle.to_lowercase();
    match items
        .iter()
        .position(|item| item.content.to_lowercase().contains(&lower))
    {
        Some(index) => {
            items.remove(index);
            DeltaOutcome::Applied
        }
        None => DeltaOutcome::NotFound,
    }
}

fn clear_vec(items: &mut Vec<MemoryItem>) -> usize {
    let n = items.len();
    items.clear();
    n
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::budget::TokenEstimator;

    fn item(text: &str) -> MemoryItem {
        MemoryItem::new(text)
    }

    #[test]
    fn set_goal_overwrites_and_empty_clears() {
        let mut mem = WorkingMemory::default();
        assert_eq!(
            mem.apply_delta(&MemoryDelta::SetGoal {
                goal: "ship v1".into()
            }),
            DeltaOutcome::Applied
        );
        assert_eq!(mem.task_goal.as_deref(), Some("ship v1"));

        assert_eq!(
            mem.apply_delta(&MemoryDelta::SetGoal {
                goal: "   ".into()
            }),
            DeltaOutcome::Applied
        );
        assert_eq!(mem.task_goal, None);
    }

    #[test]
    fn add_fact_appends_and_ignores_empty() {
        let mut mem = WorkingMemory::default();
        assert_eq!(
            mem.apply_delta(&MemoryDelta::AddFact {
                content: "rust edition 2024".into()
            }),
            DeltaOutcome::Applied
        );
        assert_eq!(
            mem.apply_delta(&MemoryDelta::AddFact {
                content: "  ".into()
            }),
            DeltaOutcome::NotFound
        );
        assert_eq!(mem.durable_facts.len(), 1);
        assert_eq!(mem.durable_facts[0].content, "rust edition 2024");
    }

    #[test]
    fn remove_fact_removes_first_case_insensitive_match() {
        let mut mem = WorkingMemory::default();
        mem.durable_facts.push(item("API base is /v1"));
        mem.durable_facts.push(item("API base is /v2"));

        assert_eq!(
            mem.apply_delta(&MemoryDelta::RemoveFact {
                needle: "api base".into()
            }),
            DeltaOutcome::Applied
        );
        assert_eq!(mem.durable_facts.len(), 1);
        assert_eq!(mem.durable_facts[0].content, "API base is /v2");
    }

    #[test]
    fn remove_fact_no_match_is_not_found() {
        let mut mem = WorkingMemory::default();
        mem.durable_facts.push(item("hello"));

        assert_eq!(
            mem.apply_delta(&MemoryDelta::RemoveFact {
                needle: "missing".into()
            }),
            DeltaOutcome::NotFound
        );
        assert_eq!(mem.durable_facts.len(), 1);
    }

    #[test]
    fn resolve_question_removes_first_match() {
        let mut mem = WorkingMemory::default();
        mem.open_questions.push(item("Which DB?"));
        mem.open_questions.push(item("Which port?"));

        assert_eq!(
            mem.apply_delta(&MemoryDelta::ResolveQuestion {
                needle: "port".into()
            }),
            DeltaOutcome::Applied
        );
        assert_eq!(mem.open_questions.len(), 1);
        assert_eq!(mem.open_questions[0].content, "Which DB?");
    }

    #[test]
    fn clear_scopes_correctly() {
        let mut mem = WorkingMemory::default();
        mem.durable_facts.push(item("a"));
        mem.decisions.push(item("d"));
        mem.open_questions.push(item("q"));
        mem.task_goal = Some("g".into());

        assert_eq!(
            mem.apply_delta(&MemoryDelta::Clear {
                scope: MemoryScope::Facts
            }),
            DeltaOutcome::Cleared(1)
        );
        assert!(mem.durable_facts.is_empty());
        assert_eq!(mem.decisions.len(), 1);

        assert_eq!(
            mem.apply_delta(&MemoryDelta::Clear {
                scope: MemoryScope::All
            }),
            DeltaOutcome::Cleared(2)
        );
        assert!(mem.decisions.is_empty() && mem.open_questions.is_empty());
        assert_eq!(mem.task_goal, None);
    }

    #[test]
    fn apply_then_render_reflects_state() {
        let mut mem = WorkingMemory::default();
        mem.apply_delta(&MemoryDelta::SetGoal {
            goal: "migrate config".into(),
        });
        mem.apply_delta(&MemoryDelta::AddFact {
            content: "config is toml".into(),
        });

        let message = mem.to_context_message(&TokenEstimator, 4_096).unwrap();
        let content = message.content.unwrap();
        assert!(content.contains("Task goal"));
        assert!(content.contains("migrate config"));
        assert!(content.contains("config is toml"));
    }
}
