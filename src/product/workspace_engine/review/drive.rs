use super::*;
use crate::cross_cutting::provider_adapter::{
    PROVIDER_ERROR_STDERR_TAIL_BYTES, ProviderAdapterError,
};
use crate::cross_cutting::session_launch::ValidatedStreamingProviderInput;
use crate::product::logical_codebase::provider_gateway::{
    GatewaySessionDisposition, ResumeSessionLaunchRequest,
};
use crate::product::logical_codebase::{
    LogicalCodebaseProviderGateway, PolicyTarget, ProviderGatewayError, ProviderRef,
    SessionLaunchRequest, SessionPolicyAction, ValidatedSessionLaunchPolicy,
};
use crate::product::models::{LcRecoveryBusinessContext, LcRecoveryRunContext};
use crate::product::workspace_engine::provider_drive::{
    PROVIDER_CHOICE_WAIT_TIMEOUT, PROVIDER_IDLE_WATCHDOG_TIMEOUT, PendingChoiceRequests,
};
use crate::product::workspace_engine::types::ReviewProviderRunFailure;
use std::pin::Pin;

/// 重子调用装箱助手（治本 Task 14b，栈根修）：被包的 future 在**本助手
/// 自己的栈帧**内构造，随即装箱到堆——调用方（评审链各层）的 poll 帧里
/// 只剩胖指针，不再摊住子 future 的构造临时区。
///
/// 修前 `drive_review_session_via_gateway_with_window` 的 poll 帧预留
/// 768,440 B（重子调用的构造临时区之和），是 provider run 爆栈链上的最大
/// 单帧；逐个装箱后降到目标量级。语义零变化：只是把「构造」挪进助手的
/// 栈帧、把 future 状态搬到堆。
#[inline(never)]
fn boxed_child<'a, F, Fut>(make: F) -> Pin<Box<dyn Future<Output = Fut::Output> + Send + 'a>>
where
    F: FnOnce() -> Fut + Send + 'a,
    Fut: Future + Send + 'a,
    Fut::Output: Send + 'a,
{
    Box::pin(make())
}

include!("drive_parts/review_session_direct.inc.rs");

include!("drive_parts/review_session_gateway.inc.rs");

include!("drive_parts/revision_launch.inc.rs");

/// Task 2.3：LC revision launch 决策结果——validated launch（cwd=root、
/// target=E `native_launch_anchor` 聚合根锚）+ resume 是否放行（supersede/
/// fresh 时 false）。
/// Task 9c:`superseded` 携带 StartNew 的旧会话 id 与稳定等待投影(仅显式
/// `StartGeneration` fresh)。
#[derive(Debug)]
pub(crate) struct RevisionLaunchDecision {
    launch: ValidatedSessionLaunchPolicy,
    resume_allowed: bool,
    superseded: Option<SupersededRevisionWait>,
}

/// Task 9c:StartNew(supersede)的等待上下文——被取代旧会话 id + gateway
/// 稳定等待投影(reason_code + 唯一 allowed action StartGeneration)。
#[derive(Debug)]
struct SupersededRevisionWait {
    superseded_session_id: String,
    waiting: crate::product::logical_codebase::provider_gateway::LcSupersededWaiting,
}

include!("drive_parts/reviewer_provider_session.inc.rs");

fn reviewer_failure_diagnostic(
    code: Option<crate::protocol::provider_errors::ProviderErrorCode>,
    message: &str,
) -> String {
    if code == Some(crate::protocol::provider_errors::ProviderErrorCode::ProviderUnavailable)
        && message.contains("503")
        && message.contains("No available accounts")
    {
        "provider_gateway_503_no_accounts: 推理网关账号池不可用；检查服务后手动重驱".to_string()
    } else {
        "provider_reviewer_failed: 评审运行失败；核对 provider 后手动重驱".to_string()
    }
}

fn provider_allows_review_repair(provider: &ProviderName) -> bool {
    !matches!(provider, ProviderName::Pi)
}

/// 逻辑 review 会话经 gateway 启动:组装 `ReviewReadOnly` launch → `validate` →
/// `start_streaming`。launch 的 provider ref 由调用方传入的 reviewer
/// (`session.reviewer_provider`)经集中映射 `ProviderRef::from_provider_name` 派生
/// (C-2:不再硬编码 ClaudeCode;不支持的 provider 显式失败)。gateway 错误映射为
/// `ProviderAdapterError`(与 `drive_reviewer_provider_session_once` 的 `Start`
/// 失败路径对齐)。
///
/// Task 2.3（REQ-ENV-10）：cwd 与 target 分离——cwd 取 gateway 冻结的 canonical
/// authority root（manifest `provider_context_root`，双工厂 canonical 一致性由
/// Task 2.8 断言），不再与 target 同源；target 保持独立 `aggregate_root` 锚
/// （成员 worktree，即 review 对象），readable roots 随 cwd root 化（成员位于
/// root 子树内的聚合布局）。validated 后 input 显式回填独立 cwd——spawn 前
/// effective cwd 复验以 envelope 冻结值为准。
///
/// 指引卡简化补缺：返回值携带分类桥同源等待投影（`trust_waiting`，仅
/// trust 分型命中且启动失败时 Some）——reviewer 首发臂消费它紧随 error
/// 事件发出 `EngineEvent::TrustMissingGuidance`；启动成功恒 None。
/// 批2 Task 20b(决策项⑦):规划面活动审查窗——句柄+同源复验的期望
/// 事实(worktree/provider/run_id 在开窗时冻结;assert 不再回读会话
/// 字段,repair/静默重试与测试换装会话字段零漂移)。
pub(crate) struct ActivePlanReviewWindow {
    pub(crate) handle: crate::product::reviewer_window::ReviewerWindowHandle,
    pub(crate) worktree: std::path::PathBuf,
    pub(crate) provider: String,
    pub(crate) run_id: String,
    /// B1-1:开窗时打开的 plan-review 主 scope(`plan-review-{window}` ×
    /// run × boot)——规划 reviewer 每次 launch 从此真实 scope 铸造受信
    /// 归属(此前开窗后即丢弃);窗口生命周期内持有,收口由既有窗口流
    /// 与 scope 级 quiesce 承担。
    pub(crate) launch_scope:
        std::sync::Arc<crate::cross_cutting::provider_resources::ProviderRunResourceScope>,
}

async fn start_review_session_via_gateway_owned(
    gateway: &Arc<LogicalCodebaseProviderGateway>,
    reviewer: &ProviderName,
    input: StreamingProviderInput,
    project_id: String,
    // B1-1:规划 reviewer 的受信资源归属(活动窗主 scope 铸造;direct/
    // gateway 两形态同一来源)。绑定进 validated input 由 adapter
    // `start_validated` 的 spawn 点消费。
    launch_ownership: Option<crate::cross_cutting::provider_resources::TrustedLaunchOwnership>,
    cancel: CancellationToken,
) -> (
    Result<ProviderSession, ProviderAdapterError>,
    Option<crate::product::logical_codebase::provider_trust::ProviderTrustWaiting>,
) {
    let root = gateway.authority_root().to_path_buf();
    let provider_ref = match ProviderRef::from_provider_name(reviewer, "cap_managed_snapshot") {
        Ok(provider_ref) => provider_ref,
        Err(error) => return (Err(map_gateway_error_to_adapter(&error)), None),
    };
    let request = SessionLaunchRequest {
        project_id,
        provider: provider_ref,
        action: SessionPolicyAction::ReviewReadOnly,
        target: PolicyTarget::aggregate_root(input.working_dir.clone()),
        // Task 2.3：cwd=canonical root（独立于 target）；target 是
        // aggregate_root 锚（worktree 取成员路径，即 review 对象）。
        working_directory: root.clone(),
        readable_roots: vec![root],
        writable_roots: Vec::new(),
        config_artifact_ref: "sha256:managed-config-artifact".to_string(),
    };
    let validated = match gateway.validate(request) {
        Ok(validated) => validated,
        Err(error) => return (Err(map_gateway_error_to_adapter(&error)), None),
    };
    let mut input = input;
    input.working_directory = Some(validated.envelope().working_directory.clone());
    // A1-b1(Bug45 批2 环境回退):reviewer 受信环境注入停止——子进程经底座
    // (process_manager)继承宿主环境,caller input.env_vars 原样透传(仅作
    // 显式覆盖层);窗口纪律仍由 assert_plan_review_window 与 start 漏斗
    // 窗口守卫把关,与启动环境解耦。
    let validated_input = ValidatedStreamingProviderInput::new(input, validated);
    // B1-1:受信 launch 归属绑定进 validated input(crate 私有 setter),
    // adapter `start_validated` 的 spawn 点消费同一登记生命周期。
    let validated_input = match launch_ownership {
        Some(ownership) => validated_input.with_launch_ownership(ownership),
        None => validated_input,
    };
    match boxed_child(|| gateway.start_streaming(validated_input, cancel)).await {
        Ok(session) => (Ok(session), None),
        Err(error) => {
            let (mapped, trust_waiting) =
                map_gateway_error_to_adapter_with_trust(gateway, reviewer, &error);
            (Err(mapped), trust_waiting)
        }
    }
}

/// 既有 5 参签名(测试/trust 分类面消费):无归属委托 owned 变体。
/// 生产规划 reviewer launch 一律经 `_owned` 传活动窗归属。
#[cfg(test)]
async fn start_review_session_via_gateway(
    gateway: &Arc<LogicalCodebaseProviderGateway>,
    reviewer: &ProviderName,
    input: StreamingProviderInput,
    project_id: String,
    cancel: CancellationToken,
) -> (
    Result<ProviderSession, ProviderAdapterError>,
    Option<crate::product::logical_codebase::provider_trust::ProviderTrustWaiting>,
) {
    start_review_session_via_gateway_owned(gateway, reviewer, input, project_id, None, cancel).await
}

/// T6 fix-loop r2：review/drive.rs 两个 gateway 启动口（author revision、
/// reviewer 首发）共用的压平+分类——共享分类桥消费 typed 变体判据
/// （压平前），Generation 场景；`provider` 是本次启动的真实角色
/// provider（revision 传 author、review 传 reviewer），不得把参数名
/// 机械照搬成错误角色。
/// 指引卡简化补缺：结构化变体一次产出平铺 details 与同源等待投影
/// （`trust_waiting` 供调用方紧随 error 事件发出
/// `EngineEvent::TrustMissingGuidance`）；判据/文案与平铺面完全一致。
fn map_gateway_error_to_adapter_with_trust(
    gateway: &crate::product::logical_codebase::LogicalCodebaseProviderGateway,
    provider: &ProviderName,
    error: &ProviderGatewayError,
) -> (
    ProviderAdapterError,
    Option<crate::product::logical_codebase::provider_trust::ProviderTrustWaiting>,
) {
    let mut mapped = map_gateway_error_to_adapter(error);
    let classification = gateway.classify_launch_trust_failure(
        error,
        provider,
        &mapped.details,
        crate::product::logical_codebase::provider_trust::ProviderTrustScenario::Generation,
    );
    mapped.details = classification.details;
    (mapped, classification.trust_waiting)
}

/// gateway 错误到 adapter 错误的桥接:使调用点后续对 `Err` 的处理与直接
/// `provider.start` 完全一致(`ReviewProviderRunFailure::Start`)。
/// 诊断直通（claude×轻 握手谜团 第 2 轮）:Adapter 变体不再丢弃 stderr
/// 字段——stderr 尾部（有界）并入 details（随 Start 失败 →
/// EngineEvent::Error → WS error 消息上浮），stderr 字段同源保留；其余
/// gateway 校验错误维持 Display 文案不变（零变化）。
fn map_gateway_error_to_adapter(error: &ProviderGatewayError) -> ProviderAdapterError {
    match error {
        ProviderGatewayError::Adapter(inner) => {
            let mut mapped = ProviderAdapterError::provider_unavailable(error.to_string());
            ProviderAdapterError::append_bounded_stderr_tail(
                &mut mapped.details,
                &inner.stderr,
                PROVIDER_ERROR_STDERR_TAIL_BYTES,
            );
            mapped.stderr = inner.stderr.clone();
            mapped
        }
        _ => ProviderAdapterError::provider_unavailable(error.to_string()),
    }
}

// ---- 批2 Task 20b(决策项⑦):规划面 reviewer 审查窗口 ----
// 约定(F13,波3):窗口 store 根目录名统一引用
// `reviewer_window_store::REVIEWER_WINDOW_STORE_DIR` 单一常量(与
// `coding_workspace_engine::code_review`、`coding_attempt_store/
// git_operation.rs` 同源;快照/资源 store 根仍为字面量锚定约定,
// 与 git_operation.rs 同款——本项只上收 reviewer-windows 一处)。
const PLAN_REVIEWER_SNAPSHOT_STORE_DIR: &str = "reviewer-snapshots";
const PLAN_PROVIDER_RESOURCE_STORE_DIR: &str = "provider-resources";
/// 修复轮 1(F1):聚合根规划评审窗口目标待 owner 裁定的专用错误码
/// (真·无唯一成员面不静默杀、不以 IdentityDrift 误报)。
pub(crate) const PLAN_REVIEW_WINDOW_TARGET_UNRESOLVED: &str =
    "plan_review_window_target_unresolved";

include!("drive_parts/window_scope.inc.rs");

#[cfg(test)]
#[path = "drive_tests.rs"]
mod tests;

include!("drive_parts/review_tests.inc.rs");
