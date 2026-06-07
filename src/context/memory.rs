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
