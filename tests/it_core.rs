//! 集成测试入口：核心域（artifact/spec/plan/design/openspec/cli/runtime 等）。
#[path = "it_core/approval_gate_handlers.rs"]
mod approval_gate_handlers;
#[path = "it_core/artifact_schema_min_fields.rs"]
mod artifact_schema_min_fields;
#[path = "it_core/artifact_validate.rs"]
mod artifact_validate;
#[path = "it_core/clarification_record.rs"]
mod clarification_record;
#[path = "it_core/cli_adapter_baseline.rs"]
mod cli_adapter_baseline;
#[path = "it_core/cli_entry.rs"]
mod cli_entry;
#[path = "it_core/context_builder.rs"]
mod context_builder;
#[path = "it_core/design_projection.rs"]
mod design_projection;
#[path = "it_core/document_ops.rs"]
mod document_ops;
#[path = "it_core/issue_rollback.rs"]
mod issue_rollback;
#[path = "it_core/large_file_guard.rs"]
mod large_file_guard;
#[path = "it_core/openspec_bundle.rs"]
mod openspec_bundle;
#[path = "it_core/openspec_bundle_schema.rs"]
mod openspec_bundle_schema;
#[path = "it_core/phase1_end_to_end_smoke.rs"]
mod phase1_end_to_end_smoke;
#[path = "it_core/phase1_profile.rs"]
mod phase1_profile;
#[path = "it_core/plan_projection.rs"]
mod plan_projection;
#[path = "it_core/policy_override_degrade.rs"]
mod policy_override_degrade;
#[path = "it_core/risk_registry_minimal.rs"]
mod risk_registry_minimal;
#[path = "it_core/runtime_snapshot_schema.rs"]
mod runtime_snapshot_schema;
#[path = "it_core/runtime_units.rs"]
mod runtime_units;
#[path = "it_core/spec_projection.rs"]
mod spec_projection;
#[path = "it_core/superseded_artifact_refs.rs"]
mod superseded_artifact_refs;
mod support;
#[path = "it_core/traceability_binding.rs"]
mod traceability_binding;
#[path = "it_core/work_item_scheduler.rs"]
mod work_item_scheduler;
#[path = "it_core/workspace_ws_integration.rs"]
mod workspace_ws_integration;
#[path = "it_core/worktree_locking.rs"]
mod worktree_locking;
