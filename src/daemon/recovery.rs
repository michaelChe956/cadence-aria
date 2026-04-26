use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct EventLogIndex {
    pub daemon_session_id: String,
    pub latest_event_id: u64,
    pub first_retained_event_id: u64,
    pub first_retained_event_id_by_task: BTreeMap<String, u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplayWindow {
    pub latest_event_id: u64,
    pub first_retained_event_id: u64,
}

impl ReplayWindow {
    pub fn from_index(index: EventLogIndex) -> Self {
        Self {
            latest_event_id: index.latest_event_id,
            first_retained_event_id: index.first_retained_event_id,
        }
    }

    pub fn decide(&self, last_seen_event_id: Option<u64>) -> ReplayDecision {
        let last_seen_event_id = last_seen_event_id.unwrap_or(0);
        if last_seen_event_id + 1 < self.first_retained_event_id {
            ReplayDecision::WindowLost
        } else {
            ReplayDecision::ReplayFrom(last_seen_event_id + 1)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplayDecision {
    ReplayFrom(u64),
    WindowLost,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TaskEventRetention {
    pub global_recent_events: usize,
    pub per_task_recent_events: usize,
}

impl TaskEventRetention {
    pub fn phase1_default() -> Self {
        Self {
            global_recent_events: 10_000,
            per_task_recent_events: 1_000,
        }
    }

    pub fn effective_minimum_for_tasks(&self, task_count: usize) -> usize {
        self.global_recent_events
            .max(task_count.saturating_mul(self.per_task_recent_events))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventLogEntry {
    pub event_id: u64,
    pub task_id: Option<String>,
}

pub fn compute_retention_index(
    events: &[EventLogEntry],
    retention: TaskEventRetention,
) -> EventLogIndex {
    let latest_event_id = events.last().map_or(0, |entry| entry.event_id);
    let global_floor = latest_event_id
        .saturating_sub(retention.global_recent_events as u64)
        .saturating_add(1)
        .max(1);

    let mut by_task: BTreeMap<String, Vec<u64>> = BTreeMap::new();
    for event in events {
        if let Some(task_id) = &event.task_id {
            by_task
                .entry(task_id.clone())
                .or_default()
                .push(event.event_id);
        }
    }

    let mut protected_event_ids = BTreeSet::new();
    let mut first_retained_event_id_by_task = BTreeMap::new();
    for (task_id, event_ids) in by_task {
        let start_index = event_ids
            .len()
            .saturating_sub(retention.per_task_recent_events);
        if let Some(first_task_event_id) = event_ids.get(start_index).copied() {
            first_retained_event_id_by_task.insert(task_id, first_task_event_id);
            for event_id in &event_ids[start_index..] {
                protected_event_ids.insert(*event_id);
            }
        }
    }

    let mut retained_event_ids = BTreeSet::new();
    for event in events {
        if event.event_id >= global_floor || protected_event_ids.contains(&event.event_id) {
            retained_event_ids.insert(event.event_id);
        }
    }

    EventLogIndex {
        daemon_session_id: String::new(),
        latest_event_id,
        first_retained_event_id: retained_event_ids.first().copied().unwrap_or(1),
        first_retained_event_id_by_task,
    }
}
