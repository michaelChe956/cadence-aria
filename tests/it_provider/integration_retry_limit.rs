use cadence_aria::cross_cutting::integration_queue::{
    IntegrationFailureTracker, IntegrationRetryDecision,
};
use cadence_aria::protocol::loop_counters::{LoopCounterName, LoopCounterRegistry};

#[test]
fn integration_failure_counter_allows_one_retry_then_enters_manual_hold() {
    assert_eq!(
        LoopCounterRegistry::phase1().threshold(LoopCounterName::IntegrationFailure),
        2
    );
    let mut tracker = IntegrationFailureTracker::new("worktask_001");

    let first = tracker.record_failure("N23");
    assert_eq!(
        first,
        IntegrationRetryDecision::Retry {
            worktask_id: "worktask_001".to_string(),
            retry_count: 1,
            trigger_node: "N23".to_string(),
        }
    );

    let second = tracker.record_failure("N23");
    assert_eq!(
        second,
        IntegrationRetryDecision::ManualIntervention {
            worktask_id: "worktask_001".to_string(),
            retry_count: 2,
            trigger_node: "N23".to_string(),
            reason: "integration_retry_limit_exceeded".to_string(),
        }
    );
    assert_eq!(
        tracker.loop_counters()[&LoopCounterName::IntegrationFailure],
        2
    );
}
