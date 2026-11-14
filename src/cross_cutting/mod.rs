pub mod adapter_compatibility;
pub mod approval_bridge;
pub mod approval_gate;
pub mod aria_state_paths;
pub mod artifact_projection;
pub mod artifact_validate;
pub mod ast_grep_tool;
pub mod bounded_command_runner;
pub mod checkpoint_and_recovery;
pub mod choice_delivery;
pub mod claude_code_provider;
pub mod cli_adapter;
pub mod codex_provider;
pub mod diagnostic_redaction;
pub mod document_ops;
pub mod git_command;
// Task 1b 段①:LC 专用 validated sync bridge(streaming→sync,专用 OS 线程
// + 自有 Tokio runtime,不嵌套 current-thread block_on)。
pub mod gateway_sync_provider;
pub mod image_client;
pub mod image_reference_validation;
pub mod integration_queue;
pub mod json_rpc_peer;
pub mod kimi_code_provider;
pub mod local_usage;
// 生产 tracing subscriber 初始化（仅 `aria web` 服务形态调用）。
pub mod logging_setup;
pub mod manual_intervention;
pub mod openspec_constraints;
pub mod path_overlay;
pub mod pi_provider;
pub mod policy_mode;
pub mod process_manager;
pub mod provider_adapter;
pub mod provider_availability_gate;
pub mod provider_boundary;
pub mod provider_capabilities;
pub mod provider_context_builder;
// A1-b2(Bug45 批2 环境回退收尾):白名单环境构造器模块已删除——provider
// 子进程全路径(process_manager/cli_adapter/kimi terminal 三底座)宿主
// 继承+显式覆盖层;owner home 解析在 trust 侧
// (provider_trust_adapters::resolve_owner_home_from_env)。
pub mod provider_execution;
pub mod provider_health;
pub mod provider_registry;
// 批2 Task 5:先登记后启动、真实静止与资源回收合同。
pub mod provider_resources;
pub mod provider_router;
pub mod provider_run;
#[cfg(test)]
pub(crate) mod real_provider_e2e;
pub mod runtime_event_log;
pub mod semver;
pub mod session_launch;
pub mod streaming_provider;
pub mod structured_output;
pub mod tool_policy_audit;
pub mod traceability;
#[cfg(test)]
pub(crate) mod tracing_capture;
pub mod worktree;
