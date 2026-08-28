use super::*;
use crate::product::models::{
    RepositoryProfile, RepositoryProfileConfidence, TrustedDraftVerificationCommand,
};
use crate::product::work_item_plan_compiler::{
    PlanCandidateValidationContext, validate_plan_candidate_ir,
};

fn rep4_trusted_command_catalog() -> Vec<TrustedDraftVerificationCommand> {
    vec![
        TrustedDraftVerificationCommand {
            command: "cargo test --locked --lib levels_api".to_string(),
            cwd: "backend".to_string(),
            purpose: "backend checks".to_string(),
            source_ref: "levels-api-unit".to_string(),
        },
        TrustedDraftVerificationCommand {
            command: "pnpm test level-select".to_string(),
            cwd: "frontend".to_string(),
            purpose: "frontend checks".to_string(),
            source_ref: "level-select-ui".to_string(),
        },
        TrustedDraftVerificationCommand {
            command: "cargo test --locked --test levels_integration".to_string(),
            cwd: "integration".to_string(),
            purpose: "integration checks".to_string(),
            source_ref: "levels-integration".to_string(),
        },
    ]
}

fn rep4_repository_profile() -> RepositoryProfile {
    RepositoryProfile {
        id: "repository_profile_levels_0001".to_string(),
        project_id: "project_levels_0001".to_string(),
        issue_id: "issue_levels_0001".to_string(),
        repository_id: "repo-levels".to_string(),
        logical_repository_id: None,
        membership_revision: 0,
        provider_run_ref: Some("provider_run_levels_0001".to_string()),
        languages: vec!["rust".to_string(), "typescript".to_string()],
        frameworks: vec!["axum".to_string(), "react".to_string()],
        package_managers: vec!["cargo".to_string(), "pnpm".to_string()],
        test_frameworks: vec!["cargo-test".to_string(), "vitest".to_string()],
        build_systems: vec!["cargo".to_string(), "vite".to_string()],
        verification_capabilities: vec!["unit".to_string(), "integration".to_string()],
        detected_layers: vec!["backend".to_string(), "frontend".to_string()],
        split_recommendation: "frontend_backend".to_string(),
        confidence: RepositoryProfileConfidence::High,
        uncertainties: Vec::new(),
        created_at: "2026-08-27T00:00:00Z".to_string(),
        updated_at: "2026-08-27T00:00:00Z".to_string(),
    }
}

fn rep4_validation_context<'a>(
    profile: Option<&'a RepositoryProfile>,
    story_ids: &'a [String],
    design_ids: &'a [String],
) -> PlanCandidateValidationContext<'a> {
    PlanCandidateValidationContext {
        project_id: "project_levels_0001",
        issue_id: "issue_levels_0001",
        plan_id: "plan_levels_0001",
        source_story_spec_ids: story_ids,
        source_design_spec_ids: design_ids,
        repository_profile: profile,
        now: "2026-08-27T00:00:00Z",
    }
}
#[test]
fn full_lowering_validator_projects_rep4_through_all_existing_validator_layers() {
    let catalog = rep4_trusted_command_catalog();
    let ir = compile_work_item_plan(
        REP4_FIXTURE,
        &WorkItemPlanSourceContext {
            target_repository_id: "repo-levels".to_string(),
            trusted_command_catalog: catalog,
        },
    )
    .expect("rep4 必须先 lower 为 typed IR");
    let profile = rep4_repository_profile();
    let story_ids = vec!["story_spec_levels_0001".to_string()];
    let design_ids = vec!["design_spec_levels_0001".to_string()];

    let report = validate_plan_candidate_ir(
        &ir,
        &rep4_validation_context(Some(&profile), &story_ids, &design_ids),
    )
    .expect("rep4 IR 必须通过既有 validator 并返回 mechanical report");

    assert_eq!(report.source_revision_hash, ir.source_revision_hash);
    assert_eq!(report.compiler_version, ir.compiler_version);
    assert!(
        report.findings.iter().all(|finding| finding.severity
            != crate::product::models::WorkItemSplitFindingSeverity::Error),
        "rep4 的既有 validator findings 不得含 Error：{:#?}",
        report.findings
    );
    assert!(
        report
            .findings
            .iter()
            .all(|finding| finding.code != "integration_work_item_required"),
        "rep4 必须满足 integration work item 前提：{:#?}",
        report.findings
    );
}

#[test]
fn full_lowering_validator_preserves_existing_unknown_provider_code() {
    let catalog = rep4_trusted_command_catalog();
    let mut ir = compile_work_item_plan(
        REP4_FIXTURE,
        &WorkItemPlanSourceContext {
            target_repository_id: "repo-levels".to_string(),
            trusted_command_catalog: catalog,
        },
    )
    .expect("rep4 必须先 lower 为 typed IR");
    ir.items[2].contract.input_contracts[1].provider_logical_work_item_id =
        "WI-not-in-outline".to_string();
    let profile = rep4_repository_profile();
    let story_ids = vec!["story_spec_levels_0001".to_string()];
    let design_ids = vec!["design_spec_levels_0001".to_string()];

    let diagnostics = validate_plan_candidate_ir(
        &ir,
        &rep4_validation_context(Some(&profile), &story_ids, &design_ids),
    )
    .expect_err("不存在的 integration input provider 必须由既有 validator 拒绝");

    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "unknown_provider_logical_work_item"),
        "adapter 必须原样透传既有 unknown_provider_logical_work_item，实际为 {diagnostics:#?}"
    );
    assert!(
        diagnostics.iter().all(|diagnostic| diagnostic.line == 0),
        "validator finding 没有 source 行号时 adapter 必须显式保留为 0：{diagnostics:#?}"
    );
    assert!(
        diagnostics
            .iter()
            .all(|diagnostic| !diagnostic.message.is_empty()
                && !diagnostic.repair_example.is_empty()),
        "validator finding 的信息与回写修复提示不得在 adapter 中丢失：{diagnostics:#?}"
    );
}

#[test]
fn full_lowering_validator_is_byte_stable_for_same_ir_and_context() {
    let catalog = rep4_trusted_command_catalog();
    let source_context = WorkItemPlanSourceContext {
        target_repository_id: "repo-levels".to_string(),
        trusted_command_catalog: catalog,
    };
    let first_ir = compile_work_item_plan(REP4_FIXTURE, &source_context)
        .expect("rep4 必须先 lower 为 typed IR");
    let second_ir = compile_work_item_plan(REP4_FIXTURE, &source_context)
        .expect("相同输入第二次 lower 必须成功");
    let profile = rep4_repository_profile();
    let story_ids = vec!["story_spec_levels_0001".to_string()];
    let design_ids = vec!["design_spec_levels_0001".to_string()];
    let context = rep4_validation_context(Some(&profile), &story_ids, &design_ids);

    let first_report = validate_plan_candidate_ir(&first_ir, &context)
        .expect("首次 lower 的 IR 必须通过 validator");
    let second_report = validate_plan_candidate_ir(&second_ir, &context)
        .expect("第二次 lower 的 IR 必须通过 validator");

    assert_eq!(
        serde_json::to_vec(&first_ir).expect("IR 必须序列化"),
        serde_json::to_vec(&second_ir).expect("IR 必须序列化"),
        "同输入两次 lower 的 IR 必须逐字节一致"
    );
    assert_eq!(
        serde_json::to_vec(&first_report).expect("report 必须序列化"),
        serde_json::to_vec(&second_report).expect("report 必须序列化"),
        "同输入两次 lower 的 mechanical report 必须逐字节一致"
    );
}
