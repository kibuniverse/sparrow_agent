use std::{collections::HashMap, sync::Arc};

use anyhow::Result;
use tokio::sync::Mutex;

use crate::{agent::Agent, config::AppConfig};

#[derive(Default)]
pub struct ConversationStore {
    agents: Mutex<HashMap<String, Arc<Mutex<Agent>>>>,
    running_tasks: Mutex<HashMap<String, String>>,
}

impl ConversationStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn agent_for_conversation(
        &self,
        conversation_id: &str,
        config: &AppConfig,
    ) -> Result<Arc<Mutex<Agent>>> {
        let mut agents = self.agents.lock().await;

        if let Some(agent) = agents.get(conversation_id) {
            return Ok(Arc::clone(agent));
        }

        let agent = Arc::new(Mutex::new(Agent::new(config.clone()).await?));
        agents.insert(conversation_id.to_string(), Arc::clone(&agent));
        Ok(agent)
    }

    pub async fn try_start_task(
        &self,
        conversation_id: &str,
        task_id: &str,
    ) -> std::result::Result<(), String> {
        let mut running_tasks = self.running_tasks.lock().await;

        if let Some(existing_task_id) = running_tasks.get(conversation_id) {
            return Err(existing_task_id.clone());
        }

        running_tasks.insert(conversation_id.to_string(), task_id.to_string());
        Ok(())
    }

    pub async fn is_busy(&self, conversation_id: &str) -> bool {
        self.running_tasks
            .lock()
            .await
            .contains_key(conversation_id)
    }

    pub async fn finish_task(&self, conversation_id: &str, task_id: &str) {
        let mut running_tasks = self.running_tasks.lock().await;

        if running_tasks
            .get(conversation_id)
            .is_some_and(|running_task_id| running_task_id == task_id)
        {
            running_tasks.remove(conversation_id);
        }
    }
}
