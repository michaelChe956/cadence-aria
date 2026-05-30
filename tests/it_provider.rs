//! 集成测试入口：provider / recovery / execution / planning / final 域。
#[path = "it_provider/execution_chain_fake_provider.rs"]
mod execution_chain_fake_provider;
#[path = "it_provider/final_closure_fake_provider.rs"]
mod final_closure_fake_provider;
#[path = "it_provider/final_followup_routes.rs"]
mod final_followup_routes;
#[path = "it_provider/integration_retry_limit.rs"]
mod integration_retry_limit;
#[path = "it_provider/planning_chain_fake_provider.rs"]
mod planning_chain_fake_provider;
#[path = "it_provider/provider_adapter_baseline.rs"]
mod provider_adapter_baseline;
#[path = "it_provider/provider_error_routes.rs"]
mod provider_error_routes;
#[path = "it_provider/provider_workspace_runner.rs"]
mod provider_workspace_runner;
#[path = "it_provider/reconnect_replay.rs"]
mod reconnect_replay;
#[path = "it_provider/recovery_provider_worktree.rs"]
mod recovery_provider_worktree;
#[path = "it_provider/recovery_smoke.rs"]
mod recovery_smoke;
mod support;
