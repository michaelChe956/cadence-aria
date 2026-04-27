use crate::runtime_units::rework::LoopCounterRegistry;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntegrationQueueRecord {
    pub integration_record_id: String,
    pub worktask_id: String,
    pub candidate_commit_sha: String,
    pub queue_position: usize,
    pub status: IntegrationQueueStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IntegrationQueueStatus {
    Queued,
    Running,
    Completed,
}

#[derive(Debug, Clone, Default)]
pub struct IntegrationQueue {
    records: Vec<IntegrationQueueRecord>,
}

impl IntegrationQueue {
    pub fn enqueue(
        &mut self,
        worktask_id: impl Into<String>,
        candidate_commit_sha: impl Into<String>,
    ) -> IntegrationQueueRecord {
        let worktask_id = worktask_id.into();
        let queue_position = self.records.len() + 1;
        let record = IntegrationQueueRecord {
            integration_record_id: format!("integration_{worktask_id}_{queue_position:04}"),
            worktask_id,
            candidate_commit_sha: candidate_commit_sha.into(),
            queue_position,
            status: IntegrationQueueStatus::Queued,
        };
        self.records.push(record.clone());
        record
    }

    pub fn records(&self) -> &[IntegrationQueueRecord] {
        &self.records
    }
}

pub fn candidate_commit_is_not_integrated(record: &IntegrationQueueRecord) -> bool {
    record.status != IntegrationQueueStatus::Completed
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IntegrationRetryDecision {
    Retry {
        worktask_id: String,
        retry_count: u32,
        trigger_node: String,
    },
    ManualIntervention {
        worktask_id: String,
        retry_count: u32,
        trigger_node: String,
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntegrationFailureTracker {
    worktask_id: String,
    loop_counters: BTreeMap<String, u32>,
}

impl IntegrationFailureTracker {
    pub fn new(worktask_id: impl Into<String>) -> Self {
        Self {
            worktask_id: worktask_id.into(),
            loop_counters: BTreeMap::new(),
        }
    }

    pub fn record_failure(&mut self, trigger_node: impl Into<String>) -> IntegrationRetryDecision {
        let trigger_node = trigger_node.into();
        let count = self
            .loop_counters
            .entry("integration_failure_counter".to_string())
            .and_modify(|value| *value += 1)
            .or_insert(1);
        let retry_count = *count;
        let threshold = LoopCounterRegistry::phase1()
            .threshold("integration_failure_counter")
            .unwrap_or(2);
        if retry_count >= threshold {
            IntegrationRetryDecision::ManualIntervention {
                worktask_id: self.worktask_id.clone(),
                retry_count,
                trigger_node,
                reason: "integration_retry_limit_exceeded".to_string(),
            }
        } else {
            IntegrationRetryDecision::Retry {
                worktask_id: self.worktask_id.clone(),
                retry_count,
                trigger_node,
            }
        }
    }

    pub fn loop_counters(&self) -> &BTreeMap<String, u32> {
        &self.loop_counters
    }
}
