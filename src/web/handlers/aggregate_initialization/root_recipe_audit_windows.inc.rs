// Root recipe 命令审计窗口（per-command before/after 快照 receipt）——本片段
// 通过 `include!` 进入 `aggregate_initialization` 模块（与
// `production_dependencies.inc.rs` 同模块命名空间，符号路径与可见性不变）。
//
// 窗口纪律线：审计只读（绝不覆盖/回滚用户与成员字节）；IO 不可观测、
// 命令身份漂移、预算超限一律 fail-closed 报错；被拒命令的证据照旧 durable
// 保留（拒绝事实不报错，由调用方决定失败面）。
//
// REQ-BOOT-17：零动作的 provider turn（窗口内零落盘差异）⇒ 该 step 判失败
// （失败事实含 provider / step / 零变更），后续 step 不再推进；零动作判据
// 复用既有 receipt auditor 的结论（`observed_changes` 是否为空），不新增
// 第二套「有无变更」判定。

use crate::product::logical_codebase::aggregate_initialization_coordinator::TurnDiagnostic;

/// Task 1.8（D2）：本 step 在固定 root recipe 命令索引中的命令
/// （全局 index + 命令文本）。确定性 step 无命令（空表）。
fn root_recipe_step_commands(step: AggregateInitializationStepKind) -> Vec<(usize, &'static str)> {
    crate::product::logical_codebase::aggregate_initialization_store::root_recipe_command_index()
        .into_iter()
        .filter(|(command_step, _, _)| *command_step == step)
        .map(|(_, command_index, command)| (command_index, command))
        .collect()
}

/// 末命令判定：本 step 持有全局最大 command index（固定顺序下即
/// OpenspecAndExamples 的命令 4）——最终 receipt 的 finalize 挂点。
fn step_owns_final_recipe_command(step: AggregateInitializationStepKind) -> bool {
    let step_max = root_recipe_step_commands(step)
        .iter()
        .map(|(command_index, _)| *command_index)
        .max();
    let global_max = crate::product::logical_codebase::aggregate_initialization_store::root_recipe_command_index()
        .iter()
        .map(|(_, command_index, _)| *command_index)
        .max();
    step_max.is_some_and(|step_max| Some(step_max) == global_max)
}

/// 命令审计窗口开启：canonical root 冻结 + before 全量快照。IO/命令身份
/// 漂移 fail-closed 为可重试 ProviderTurn 失败（不可观测绝不静默）。
fn begin_root_recipe_command_watches(
    operation_id: &str,
    step: AggregateInitializationStepKind,
    preflight: &AggregatePreflightSnapshot,
) -> Result<
    Vec<crate::product::logical_codebase::root_recipe_receipt::RootRecipeCommandWatch>,
    crate::product::json_store::ProductStoreError,
> {
    let auditor =
        crate::product::logical_codebase::root_recipe_receipt::RootRecipeFilesystemAuditor::new();
    let canonical_root = std::path::PathBuf::from(&preflight.aggregate_root);
    root_recipe_step_commands(step)
        .into_iter()
        .map(|(command_index, command)| {
            auditor.before_command(operation_id, &canonical_root, step, command_index, command)
        })
        .collect()
}

/// 命令成功后逐条落 per-command receipt（拒绝也 durable 保留证据）。
/// 同 index 同内容重放幂等；同 index 不同内容冲突不覆盖（fail-closed）。
///
/// REQ-BOOT-17：本 turn 的审计事实若判为**零动作**（窗口内零落盘差异）⇒
/// 该 step 判失败（失败事实含 provider / step / 零变更），后续 step 不再
/// 推进；零动作判据复用既有 receipt auditor 的结论，不新增第二套判定。
///
/// **幂等重跑例外**：Cadence-skills 四条命令的契约就是幂等（脚本按契约报告
/// `all-skipped` / 0 created / 0 updated、产物无变化）。因此零变更**且**
/// 脚本报告成功 ⇒ 命令确实执行且按契约无变更 ⇒ 判**成功**（本仓 2026-11-12
/// 真机：已初始化的聚合根上重跑 recipe ⇒ 旧判据把幂等成功判成失败）。
/// 脚本报告失败 / 没有报告（命令可能没执行）⇒ 仍判失败。
///
/// 失败诊断同时携带 **provider 的真实输出尾部**（`turn_output`）：零变更
/// 本身不说明原因——命令没被执行、命令执行了但失败、命令按契约幂等无变更，
/// 三种情形的 provider 输出完全不同（本仓 2026-11-12 真机：pre-check 脚本
/// 的 JSON 报告被整段丢弃，用户只看到 `zero_change=true`）。
fn complete_root_recipe_command_receipts(
    paths: &ProductAppPaths,
    lc_id: &str,
    project_id: &str,
    watches: Vec<crate::product::logical_codebase::root_recipe_receipt::RootRecipeCommandWatch>,
    step: AggregateInitializationStepKind,
    provider: &crate::product::models::ProviderName,
    outcome: &crate::product::logical_codebase::aggregate_initialization_coordinator::AggregateProviderTurnOutcome,
) -> Result<(), AggregateInitializationError> {
    let auditor =
        crate::product::logical_codebase::root_recipe_receipt::RootRecipeFilesystemAuditor::new();
    let store = crate::product::logical_codebase::RootRecipeReceiptStore::for_lc(
        paths.clone(),
        lc_id.to_string(),
    );
    let receipts = watches
        .into_iter()
        .map(|watch| auditor.after_command(watch, chrono::Utc::now().to_rfc3339()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| root_recipe_audit_failure(step, error))?;
    let zero_change = store
        .append_turn_receipts(project_id, step, provider, receipts)
        .map_err(|error| root_recipe_audit_failure(step, error))?;
    match zero_change {
        None => Ok(()),
        Some(reason) => {
            // 幂等重跑：脚本报告成功 ⇒ 命令确实执行且按契约无变更 ⇒ 成功。
            if outcome.report
                == crate::product::logical_codebase::aggregate_initialization_coordinator::RecipeCommandReport::Success
            {
                return Ok(());
            }
            Err(AggregateInitializationError::ProviderTurnDiagnosed {
                step,
                reason: reason.clone(),
                retryable: true,
                provider: Some(
                    crate::product::logical_codebase::provider_trust::provider_wire_label(provider)
                        .to_string(),
                ),
                diagnostic: TurnDiagnostic {
                    cause: Some("root_recipe_turn_zero_change".to_string()),
                    output: Some(
                        crate::product::logical_codebase::aggregate_initialization_coordinator::provider_turn_failure_output(
                            &reason,
                            &outcome.summary,
                        ),
                    ),
                },
            })
        }
    }
}
