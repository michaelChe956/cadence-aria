use std::sync::{Mutex, OnceLock};

use crate::product::coding_workspace_engine::group_review_budget::{
    BudgetConfigurationError, BudgetDecision, CapacityDecision, FindingsDecision,
    GROUP_REVIEW_CAPACITY_LIMIT, GROUP_REVIEW_DEFAULT_CONCURRENCY, GROUP_REVIEW_HARD_CAP_BYTES,
    GROUP_REVIEW_QUALITY_TARGET_BYTES, GROUP_REVIEW_REDUCTION_MAX_FINDINGS,
    GROUP_REVIEW_SHARD_MAX_FINDINGS, REPAIR_PROMPT_BYTE_CAP, check_capacity,
    check_reduction_findings, check_shard_findings, decide_budget, group_review_shard_concurrency,
    validate_budget_configuration,
};
use crate::product::coding_workspace_engine::group_review_types::PromptSegments;

fn segments() -> PromptSegments {
    PromptSegments {
        fixed_protocol: "协议：审查\n".to_string(),
        identity: "身份：组审查员\n".to_string(),
        unit_records: "单元：α\n".to_string(),
        evidence_digest: "证据：✓\n".to_string(),
        graph: "图：→\n".to_string(),
        diff: "差异：你好\n".to_string(),
        retry_diagnostic_reserve: "重试：é\n".to_string(),
    }
}

#[test]
fn prompt_segments_measure_matches_joined_utf8_bytes() {
    let segments = segments();

    let joined = segments.join();
    #[allow(clippy::needless_as_bytes)]
    let joined_bytes = joined.as_bytes().len();
    assert_eq!(segments.measure().total, joined_bytes);
    assert_eq!(GROUP_REVIEW_QUALITY_TARGET_BYTES, 28 * 1024);
    assert_eq!(GROUP_REVIEW_HARD_CAP_BYTES, 30 * 1024);
    assert_eq!(REPAIR_PROMPT_BYTE_CAP, 16 * 1024);
}

#[test]
fn decide_budget_has_send_warning_and_overflow_tiers() {
    assert_eq!(
        decide_budget(
            GROUP_REVIEW_QUALITY_TARGET_BYTES,
            GROUP_REVIEW_QUALITY_TARGET_BYTES,
            GROUP_REVIEW_HARD_CAP_BYTES,
        ),
        BudgetDecision::Send
    );
    assert_eq!(
        decide_budget(
            GROUP_REVIEW_QUALITY_TARGET_BYTES + 1,
            GROUP_REVIEW_QUALITY_TARGET_BYTES,
            GROUP_REVIEW_HARD_CAP_BYTES,
        ),
        BudgetDecision::SendWithWarning
    );
    assert_eq!(
        decide_budget(
            GROUP_REVIEW_HARD_CAP_BYTES + 1,
            GROUP_REVIEW_QUALITY_TARGET_BYTES,
            GROUP_REVIEW_HARD_CAP_BYTES,
        ),
        BudgetDecision::Overflow
    );
}

#[test]
fn budget_configuration_requires_quality_target_below_hard_cap() {
    assert_eq!(
        validate_budget_configuration(30_000, 30_000),
        Err(BudgetConfigurationError::QualityTargetNotBelowHardCap)
    );
    assert_eq!(
        validate_budget_configuration(
            GROUP_REVIEW_QUALITY_TARGET_BYTES,
            GROUP_REVIEW_HARD_CAP_BYTES + 1,
        ),
        Err(BudgetConfigurationError::HardCapExceedsMaximum)
    );
    assert_eq!(validate_budget_configuration(20 * 1024, 24 * 1024), Ok(()));
}

#[test]
fn capacity_exceeding_twenty_units_returns_pure_exceeded_decision() {
    assert_eq!(
        check_capacity(GROUP_REVIEW_CAPACITY_LIMIT),
        CapacityDecision::WithinCapacity
    );
    assert_eq!(check_capacity(21), CapacityDecision::CapacityExceeded);
}

#[test]
fn findings_over_limit_are_exceeded() {
    assert_eq!(
        check_shard_findings(GROUP_REVIEW_SHARD_MAX_FINDINGS),
        FindingsDecision::WithinLimit
    );
    assert_eq!(check_shard_findings(9), FindingsDecision::FindingsExceeded);
    assert_eq!(
        check_reduction_findings(GROUP_REVIEW_REDUCTION_MAX_FINDINGS),
        FindingsDecision::WithinLimit
    );
    assert_eq!(
        check_reduction_findings(17),
        FindingsDecision::FindingsExceeded
    );
}

#[test]
fn shard_concurrency_defaults_to_two_and_accepts_only_positive_configured_values() {
    let _lock = concurrency_environment_lock()
        .lock()
        .expect("concurrency environment lock");
    let _environment = ShardConcurrencyEnvironment::clear();

    assert_eq!(GROUP_REVIEW_DEFAULT_CONCURRENCY, 2);
    assert_eq!(group_review_shard_concurrency(), 2);

    ShardConcurrencyEnvironment::set("5");
    assert_eq!(group_review_shard_concurrency(), 5);
    ShardConcurrencyEnvironment::set("0");
    assert_eq!(group_review_shard_concurrency(), 2);
    ShardConcurrencyEnvironment::set("not-a-number");
    assert_eq!(group_review_shard_concurrency(), 2);
}

fn concurrency_environment_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

struct ShardConcurrencyEnvironment {
    original: Option<std::ffi::OsString>,
}

impl ShardConcurrencyEnvironment {
    fn clear() -> Self {
        // SAFETY: this test serializes access to this environment variable with its mutex.
        let original = std::env::var_os("GROUP_REVIEW_SHARD_CONCURRENCY");
        // SAFETY: this test serializes access to this environment variable with its mutex.
        unsafe { std::env::remove_var("GROUP_REVIEW_SHARD_CONCURRENCY") };
        Self { original }
    }

    fn set(value: &str) {
        // SAFETY: this test serializes access to this environment variable with its mutex.
        unsafe { std::env::set_var("GROUP_REVIEW_SHARD_CONCURRENCY", value) };
    }
}

impl Drop for ShardConcurrencyEnvironment {
    fn drop(&mut self) {
        // SAFETY: this test serializes access to this environment variable with its mutex.
        unsafe { std::env::remove_var("GROUP_REVIEW_SHARD_CONCURRENCY") };
        if let Some(value) = &self.original {
            // SAFETY: this test serializes access to this environment variable with its mutex.
            unsafe { std::env::set_var("GROUP_REVIEW_SHARD_CONCURRENCY", value) };
        }
    }
}
