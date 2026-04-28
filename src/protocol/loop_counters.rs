use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopCounterName {
    PatchRound,
    Rework,
    DesignRevision,
    Clarification,
    IntegrationFailure,
}

impl LoopCounterName {
    pub fn default_threshold(&self) -> u32 {
        match self {
            LoopCounterName::PatchRound => 2,
            LoopCounterName::Rework => 3,
            LoopCounterName::DesignRevision => 3,
            LoopCounterName::Clarification => 3,
            LoopCounterName::IntegrationFailure => 2,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            LoopCounterName::PatchRound => "patch_round",
            LoopCounterName::Rework => "rework",
            LoopCounterName::DesignRevision => "design_revision",
            LoopCounterName::Clarification => "clarification",
            LoopCounterName::IntegrationFailure => "integration_failure",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoopCounterRegistry {
    thresholds: BTreeMap<LoopCounterName, u32>,
}

impl LoopCounterRegistry {
    pub fn phase1() -> Self {
        let mut thresholds = BTreeMap::new();
        for name in [
            LoopCounterName::PatchRound,
            LoopCounterName::Rework,
            LoopCounterName::DesignRevision,
            LoopCounterName::Clarification,
            LoopCounterName::IntegrationFailure,
        ] {
            thresholds.insert(name, name.default_threshold());
        }
        Self { thresholds }
    }

    pub fn threshold(&self, name: LoopCounterName) -> u32 {
        self.thresholds.get(&name).copied().unwrap_or_else(|| name.default_threshold())
    }

    pub fn all_thresholds(&self) -> &BTreeMap<LoopCounterName, u32> {
        &self.thresholds
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoopCounterError {
    ThresholdExceeded {
        counter: LoopCounterName,
        current: u32,
        threshold: u32,
    },
}

impl std::fmt::Display for LoopCounterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoopCounterError::ThresholdExceeded { counter, current, threshold } => {
                write!(
                    f,
                    "loop_counter_threshold_exceeded: {} current={} threshold={}",
                    counter.as_str(),
                    current,
                    threshold
                )
            }
        }
    }
}

impl std::error::Error for LoopCounterError {}

/// Validate that a counter value does not exceed its registered threshold.
pub fn check_counter(
    counters: &BTreeMap<LoopCounterName, u32>,
    name: LoopCounterName,
    registry: &LoopCounterRegistry,
) -> Result<(), LoopCounterError> {
    let current = counters.get(&name).copied().unwrap_or(0);
    let threshold = registry.threshold(name);
    if current > threshold {
        Err(LoopCounterError::ThresholdExceeded {
            counter: name,
            current,
            threshold,
        })
    } else {
        Ok(())
    }
}
