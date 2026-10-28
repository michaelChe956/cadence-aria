//! F-19 provider 零活动看门狗：超时常量与 legacy 流 provider start 诊断登记。
//! 自 `provider_drive.rs` 拆出（1200 行守护，Wave 3.1）：纯移动零语义变化。

use super::*;

/// F-19 看门狗：provider 会话零活动上限（拉起后无事件/命令持续至此即判楔死）。
///
/// 依据：v27 story 会话 codex app-server 楔死实测——09:16 拉起，前几分钟 9 条
/// skill 探索命令后**完全静默 27min**（CPU 时间采样零增长、出站 TCP 0 条、
/// durable streaming 恒 137 字；cadence/notes 2026-09-19 阶段4监控 F-19）。
/// `DEFAULT_PROVIDER_TIMEOUT_SECS=3h` 是整 run 总上限，对「子进程活着但不
/// 干活」无效（27min 楔死全程在总上限内）。
///
/// r38 校准：600s→1800s。原 600s 依据「流式 provider 正常推理逐 token 产出
/// 事件，合法静默间隙远小于 10min」——r32(work item plan 生成 648s)与
/// r38(design fresh 实体阶段)两轮实测推翻该假设：大文档生成+API 慢周期
/// 下实体阶段合法静默同样可超 10min，600s 把合法慢生成当楔死误杀。1800s
/// 与 work item plan 循环(fd056da5)同值：真挂死 30min 收口仍可诊断
/// (harness 阶段预算 5400s 前有界)；权限/选择挂起期间看门狗不计时不变
/// (人工等待由 ApprovalBridge PERMISSION_TIMEOUT 与 choice_wait 界收口)。
#[cfg(not(test))]
pub(crate) const PROVIDER_IDLE_WATCHDOG_TIMEOUT: std::time::Duration =
    std::time::Duration::from_secs(1800);
#[cfg(test)]
pub(crate) const PROVIDER_IDLE_WATCHDOG_TIMEOUT: std::time::Duration =
    std::time::Duration::from_millis(150);

/// **生产取值**常量（不受 `cfg(test)` 影响）：provider 零活动看门狗阈值 = 1800s。
///
/// 与 `PROVIDER_IDLE_WATCHDOG_TIMEOUT` 的生产取值同值，但不受测试期 150ms 覆盖影响，
/// 供跨模块断言（如 scratch 孤儿 TTL 与看门狗阈值的大小关系）链接**生产值**，
/// 避免用字面量断言导致生产阈值上调后断言失效。
/// 非测试构建下无消费方（断言在测试期），故按构建期豁免 dead_code。
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) const PROVIDER_IDLE_WATCHDOG_TIMEOUT_PRODUCTION: std::time::Duration =
    std::time::Duration::from_secs(1800);

/// provider **启动握手**停滞界：plan author 启动口
/// (`start_work_item_plan_author`) 从调用到返回 `ProviderSession` 的上限。
///
/// 缺口依据（owner r20/r21 现场）：进程在同一处静默 37–40 分钟——零 provider
/// 事件、零 WS 帧，只能人工 kill。F-19 系列看门狗全部在**拿到 session 之后**的
/// 驱动循环内生效（`work_item_plan.rs` 的 `PROVIDER_WORK_ITEM_PLAN_IDLE_WATCHDOG_TIMEOUT`、
/// `provider_drive.rs` 的 `PROVIDER_IDLE_WATCHDOG_TIMEOUT`），而 SC 单候选确认 run
/// 与 SC 返修接力 run（`HumanGateScManualRevision`）的启动握手在其**之前**：
/// 握手期间既无 provider 事件可看、也无任何看门狗计时，只能等 adapter 内部
/// 的整 run 总预算 `DEFAULT_PROVIDER_TIMEOUT_SECS`（3h）收口。
///
/// 取值理由（300s）：
/// - 握手是纯机器交互（spawn 子进程 → initialize → session/new，必要时叠加
///   trust 登记），三个 adapter 的**单次** RPC 请求界都是 60s
///   （`PI_RPC_REQUEST_TIMEOUT` / `KIMI_RPC_REQUEST_TIMEOUT` /
///   `CODEX_RPC_REQUEST_TIMEOUT`）。300s = 5×单次 RPC 界，足以覆盖若干串行
///   RPC + 进程冷启动 + trust 登记在慢机器上的抖动。
/// - 严格小于 `PROVIDER_IDLE_WATCHDOG_TIMEOUT`（1800s）：握手期**不存在**模型
///   生成，因此合法的「慢」远小于生成期合法静默；用 1800s 会把真楔死当正常。
/// - 远小于整 run 总预算 `DEFAULT_PROVIDER_TIMEOUT_SECS`（3h）：后者是兜底，
///   不能让一处握手卡死吃掉整个 run 预算（r19 现场桥线程空等 57min 的同族教训）。
///
/// 该界只约束「尚未产生任何 provider 动作」的握手窗口，**不改变**任何既有看门狗
/// 的覆盖：拿到 session 后仍由各驱动循环的零活动/choice 界收口。
pub(crate) const PROVIDER_LAUNCH_STALL_TIMEOUT_PRODUCTION: std::time::Duration =
    std::time::Duration::from_secs(300);

/// 生产取值护栏（两构建下都求值）：启动停滞界必须严格小于零活动看门狗阈值，
/// 且小于整 run 总预算——避免后续校准把启动界抬到与生成期看门狗同界。
const _: () = assert!(
    PROVIDER_LAUNCH_STALL_TIMEOUT_PRODUCTION.as_secs()
        < PROVIDER_IDLE_WATCHDOG_TIMEOUT_PRODUCTION.as_secs()
);
const _: () = assert!(
    PROVIDER_LAUNCH_STALL_TIMEOUT_PRODUCTION.as_secs()
        < crate::cross_cutting::provider_adapter::DEFAULT_PROVIDER_TIMEOUT_SECS
);

/// 启动停滞界的**生效值**：生产恒为
/// [`PROVIDER_LAUNCH_STALL_TIMEOUT_PRODUCTION`]；测试可经
/// [`set_provider_launch_stall_timeout_for_test`] 注入短阈值（默认不注入，
/// 保持生产值——既有「启动悬挂」类用例依赖真实悬挂语义，不被测试期常量
/// 意外缩短）。
pub(crate) fn provider_launch_stall_timeout() -> std::time::Duration {
    #[cfg(test)]
    {
        let injected_ms =
            LAUNCH_STALL_TIMEOUT_OVERRIDE_MS.load(std::sync::atomic::Ordering::SeqCst);
        if injected_ms > 0 {
            return std::time::Duration::from_millis(injected_ms);
        }
    }
    PROVIDER_LAUNCH_STALL_TIMEOUT_PRODUCTION
}

/// 测试期注入槽（毫秒；0 = 未注入）。
#[cfg(test)]
static LAUNCH_STALL_TIMEOUT_OVERRIDE_MS: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);

/// 测试期注入停滞界；Drop 复位。**可控时间**：用例无需真等 300s。
#[cfg(test)]
pub(crate) struct ProviderLaunchStallTimeoutGuard;

#[cfg(test)]
impl Drop for ProviderLaunchStallTimeoutGuard {
    fn drop(&mut self) {
        LAUNCH_STALL_TIMEOUT_OVERRIDE_MS.store(0, std::sync::atomic::Ordering::SeqCst);
    }
}

#[cfg(test)]
pub(crate) fn set_provider_launch_stall_timeout_for_test(
    timeout: std::time::Duration,
) -> ProviderLaunchStallTimeoutGuard {
    LAUNCH_STALL_TIMEOUT_OVERRIDE_MS.store(
        timeout.as_millis() as u64,
        std::sync::atomic::Ordering::SeqCst,
    );
    ProviderLaunchStallTimeoutGuard
}

/// r32 校准:work item plan 驱动循环(SC 单候选 plan 文档生成)的零活动
/// 看门狗界——1800s(30min)。r32 现场 plan fresh 被 600s 界误杀:Claude
/// 生成完整 work item plan 文档的合法静默可超 10min(r28 实测生成段
/// 648s>600s;600s 界把合法慢生成当楔死)。30min 真挂死收口仍可接受
//(harness 阶段预算 5400s 前有界);若现场再误杀,再议事件类型分级
///(stream_chunk 重置/纯静默才计时)方案。
#[cfg(not(test))]
pub(crate) const PROVIDER_WORK_ITEM_PLAN_IDLE_WATCHDOG_TIMEOUT: std::time::Duration =
    std::time::Duration::from_secs(1800);
#[cfg(test)]
pub(crate) const PROVIDER_WORK_ITEM_PLAN_IDLE_WATCHDOG_TIMEOUT: std::time::Duration =
    std::time::Duration::from_millis(150);

/// F-22/F-19b：choice（结构化提问）悬置等待上限——provider 发出
/// ChoiceRequest（pi `ask_user`/claude AskUserQuestion/codex requestUserInput）
/// 后等待人工应答的界。
///
/// 依据：v28 实测 0482（codex 首跑 + abort 后 pi 重跑）与 0483（全新会话，无
/// abort）三连楔死——provider 经结构化提问等待用户，看门狗按设计挂起
/// （`pending_choice_requests` 非空不计时），而 choice 卡经 broadcast 的
/// try_send 送达面无重发/无恢复（丢失即无人应答），run 无人收口永久楔死
/// （12–15+ 分钟零增长零出站，看门狗零触发）。取 900s 与 ApprovalBridge
/// `PERMISSION_TIMEOUT` 的人工等待界对齐：权限悬置由 adapter 侧超时收口，
/// choice 悬置此前无任何界——本常量补齐对称语义。触发后转可诊断失败态
/// （原因码 `provider_choice_wait_timeout`），story/design 面回 prepare_context
/// 可重跑。
#[cfg(not(test))]
pub(crate) const PROVIDER_CHOICE_WAIT_TIMEOUT: std::time::Duration =
    std::time::Duration::from_secs(900);
#[cfg(test)]
pub(crate) const PROVIDER_CHOICE_WAIT_TIMEOUT: std::time::Duration =
    std::time::Duration::from_millis(150);

impl WorkspaceEngine {
    /// F-19：legacy 流（story/design/workitem）provider start 的诊断登记。
    ///
    /// 拉起 provider 子进程前在 durable `provider_start_ledger` 落一条带
    /// provider 名与时间戳的条目（key 形 `workspace_{role}:{session_id}:{n}`，
    /// 对照 SC 面 `single_candidate_author:{session}:{n}` 先例）。F-19 实测中
    /// ledger 恒空导致「start 未发生」与「start 后楔死」无法区分——本登记补齐
    /// 该诊断面（SC/门预留路径各自已有登记，不经此处）。
    ///
    /// 失败语义：诊断面 best-effort——登记失败仅告警不阻断 run（fail-closed 会
    /// 因 store 抖动杀死正常生成，与登记目的不成比例；对照 tool-policy audit
    /// 的 fail-closed 语义：那是承重契约，这是观测登记）。
    pub(crate) fn register_provider_start_in_ledger(
        &mut self,
        role: ProviderConversationRole,
        provider: ProviderName,
    ) {
        // 范围闸：WorkItemPlan 面（Legacy+SingleCandidate）的 ledger 键索引由
        // 既有预留路径按 len 派生（routing_scope/SC reserve），追加诊断条目会
        // 漂移其键算术；该面已各自登记，本登记仅覆盖 story/design/workitem
        // 聊天面（F-19 实测楔死面）。
        if matches!(self.session.workspace_type, WorkspaceType::WorkItemPlan) {
            return;
        }
        let Some(store) = self.lifecycle_store.as_ref() else {
            return;
        };
        let session_id = self.session.session_id.clone();
        let attempt = self.session.provider_start_ledger.len();
        // ProviderConversationRole/ProviderName 均为 snake_case serde 枚举：
        // key 与 provider 文本统一取 serde 投影（author/reviewer、codex/pi…）。
        let role_text = serde_json::to_value(&role)
            .ok()
            .and_then(|value| value.as_str().map(ToString::to_string))
            .unwrap_or_else(|| "provider".to_string());
        let key = format!("workspace_{role_text}:{session_id}:{attempt}");
        let provider_text = serde_json::to_value(&provider)
            .ok()
            .and_then(|value| value.as_str().map(ToString::to_string))
            .unwrap_or_else(|| "unknown".to_string());
        let started_at = chrono::Utc::now().to_rfc3339();
        match store.claim_provider_start_with_details(
            &session_id,
            &key,
            Some(provider_text.as_str()),
            Some(started_at.clone()),
        ) {
            Ok(true) => {
                self.session.provider_start_ledger.push(
                    crate::product::work_item_plan_policy::ProviderStartLedgerEntry {
                        provider_start_idempotency_key: key,
                        started: true,
                        provider: Some(provider_text),
                        started_at: Some(started_at),
                    },
                );
            }
            Ok(false) => {
                tracing::warn!(
                    session_id = %session_id,
                    key = %key,
                    "provider start already registered; skipping duplicate ledger entry"
                );
            }
            Err(error) => {
                tracing::warn!(
                    session_id = %session_id,
                    key = %key,
                    error = %error,
                    "provider start ledger registration failed (best-effort diagnostics)"
                );
            }
        }
    }
}
