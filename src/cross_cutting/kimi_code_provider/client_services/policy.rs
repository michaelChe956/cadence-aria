//! Duty/action/target/authority/approval policy for kimi client services
//! (批2 Task 8)。
//!
//! Declaring `clientCapabilities` never authorizes execution: every
//! terminal/fs request must pass this policy first. 判定维度:
//! - 受信岗位(duty):批2 Task 8 起从「wire 角色整段拒绝」改为职责矩阵
//!   (REQ-ENV-09 岗位矩阵);受信授予(T2 冻结执行合同)优先,直连会话
//!   按承载角色映射**同一**矩阵——不建第二套角色规则。
//! - 目标(target):fs 读/写的授权根锚定与 openat/O_NOFOLLOW 原子路径
//!   合同由 `fs_service` 独立承担,不投影成 terminal target-only——
//!   「fs API 拒绝越界」与「Host terminal 物理可达」两种观察都正确,
//!   互不否定、互不放松。
//! - authority/审批:permission mode(Auto 直决/Supervised 走
//!   ApprovalBridge)。
use crate::cross_cutting::provider_execution::{FrozenProviderExecution, TrustedProviderRole};
use crate::cross_cutting::streaming_provider::ProviderPermissionMode;
use crate::protocol::contracts::AdapterRole;

/// 受信岗位授予:kimi client services 对 T2 冻结执行合同
/// ([`FrozenProviderExecution`])的消费面。字段 private、唯一构造口
/// [`Self::from_frozen_execution`]——职责与 review 窗口引用不经 wire
/// role/prompt/ACP 参数铸造(职责×profile×窗口×author 岗位策略已在
/// `freeze_provider_execution` 装配面 fail-closed)。生产接线归
/// T10/T20(前瞻载体,测试注入可信 fixture)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TrustedDutyGrant {
    duty: TrustedProviderRole,
    carrier: AdapterRole,
    review_window_id: Option<String>,
}

impl TrustedDutyGrant {
    /// 从 T2 冻结执行合同提取消费面事实。
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn from_frozen_execution(execution: &FrozenProviderExecution) -> Self {
        Self {
            duty: execution.duty(),
            carrier: execution.carrier().clone(),
            review_window_id: execution.review_window_id().map(ToString::to_string),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientAction {
    Terminal,
    FsRead,
    FsWrite,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyDecision {
    Allow,
    /// The caller must route through the ApprovalBridge before executing.
    RequireApproval,
    Deny(&'static str),
}

pub struct ClientServicePolicy {
    pub role: AdapterRole,
    pub permission_mode: ProviderPermissionMode,
    /// 受信岗位授予(T2 冻结执行合同消费面):`None` = 直连会话(承载
    /// 角色映射职责,行为见 `duty_of_wire_role`)。
    trusted: Option<TrustedDutyGrant>,
}

impl ClientServicePolicy {
    pub fn new(role: AdapterRole, permission_mode: ProviderPermissionMode) -> Self {
        Self {
            role,
            permission_mode,
            trusted: None,
        }
    }

    /// 注入受信岗位授予(唯一受信来源 = T2 冻结执行合同;测试注入可信
    /// fixture,生产接线归 T10/T20)。
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn with_trusted_duty(mut self, grant: TrustedDutyGrant) -> Self {
        self.trusted = Some(grant);
        self
    }

    /// 当前职责(广告面/审计消费):受信授予优先,直连会话按承载角色
    /// 映射。
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn duty(&self) -> TrustedProviderRole {
        self.trusted
            .as_ref()
            .map(|grant| grant.duty)
            .unwrap_or_else(|| duty_of_wire_role(&self.role))
    }

    /// BOOT-04/D1（Task 1.2）：kimi 侧 bootstrap marker 消费判定。LC 根
    /// recipe 的自举执行器固定由 Claude Code 承担（REQ-BOOT-03 recipe
    /// provider 固定；REQ-ENV-06 kimi 不读 tool_policy 字段），kimi 不是
    /// bootstrap provider——marker 通道到达 kimi 会话装配时必须在真实
    /// spawn 前拒绝（fail-closed），使其无法成为第二条「带写权限
    /// Executor」物理路径。无 marker 的普通会话（None/deny 意图）零变化
    ///（deny 意图归三 adapter 的双向守卫裁决，本判定不重复）。
    // 仅本文件 cfg(test) 消费（生产无调用点）。
    #[cfg(test)]
    pub fn evaluate_bootstrap_executor_marker(
        tool_policy: Option<&crate::cross_cutting::streaming_provider::ProviderToolPolicy>,
    ) -> PolicyDecision {
        let carries_marker = tool_policy.is_some_and(|policy| {
            matches!(
                policy.intent,
                crate::cross_cutting::streaming_provider::ToolPolicyIntent::BootstrapExecutorMarker(
                    _
                )
            )
        });
        if carries_marker {
            PolicyDecision::Deny(
                "kimi does not host the lc bootstrap executor; the root recipe provider is fixed to Claude Code",
            )
        } else {
            PolicyDecision::Allow
        }
    }

    pub fn evaluate(&self, action: ClientAction) -> PolicyDecision {
        match self.trusted.as_ref() {
            // 授予的承载角色必须等于会话 wire 角色(T2/gateway 装配面已
            // 校验 identity carrier == input role;此处纵深防御,错位
            // fail-closed)。
            Some(grant) if grant.carrier != self.role => {
                PolicyDecision::Deny("trusted duty grant carrier disagrees with the session role")
            }
            Some(grant) => {
                self.evaluate_duty(grant.duty, grant.review_window_id.as_deref(), action)
            }
            None => self.evaluate_duty(duty_of_wire_role(&self.role), None, action),
        }
    }

    /// 工具广告面(与 [`Self::evaluate`] 同一岗位合同):Author 职责不
    /// 广告代码编辑 fs 写(`writeTextFile:false`,AuthorToolPolicy v1
    /// `builtin_write_code_file` 语义;不按扩展名猜「代码文件」另建
    /// 过滤)——广告与实际拒绝同源;其余职责保持既有布尔方言全集
    /// (广告≠授权,请求仍逐条过 evaluate)。会话 initialize 接线归
    /// T10/T20(前瞻载体,测试钉合同)。
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn advertised_capabilities(&self) -> serde_json::Value {
        let fs_write_advertised = self.duty() != TrustedProviderRole::Author;
        serde_json::json!({
            "fs": {"readTextFile": true, "writeTextFile": fs_write_advertised},
            "terminal": true
        })
    }

    /// 职责矩阵判定(批2 Task 8):受信授予与承载角色映射共用同一矩阵。
    /// `review_window` 仅 Reviewer 的 FsWrite 消费(有效窗口引用经 T2
    /// 冻结合同到达;无窗 fail-closed——无窗启动已在装配面拒绝,此处
    /// 纵深防御)。
    fn evaluate_duty(
        &self,
        duty: TrustedProviderRole,
        review_window: Option<&str>,
        action: ClientAction,
    ) -> PolicyDecision {
        match (duty, action) {
            // Author 岗位纪律(REQ-ENV-09,AuthorToolPolicy v1 语义
            // builtin_write_code_file):kimi 侧「编辑代码文件」内建工具
            // 映射到代码编辑 fs 写服务,请求拒绝;作者文档走平台通道;
            // read/terminal/report/document 通道保持可用。
            (TrustedProviderRole::Author, ClientAction::FsWrite) => PolicyDecision::Deny(
                "author duty denies code-editing fs writes (builtin_write_code_file); author documents go through the platform channel",
            ),
            // Reviewer 正向窗口合同:有效窗口引用 → 授权根内 FsWrite 按
            // authority/审批判定;无窗恒拒(审批不是窗口的替代)。
            (TrustedProviderRole::Reviewer, ClientAction::FsWrite) => {
                if review_window.is_some_and(|window| !window.trim().is_empty()) {
                    self.evaluate_permission_mode()
                } else {
                    PolicyDecision::Deny(
                        "reviewer duty requires a valid review window for fs writes",
                    )
                }
            }
            // 规划/拆分/交接职责:fs 写恒拒(职责级可区分文案);读与
            // 受约束 terminal 合法,不整段拒绝。
            (TrustedProviderRole::Orchestrator, ClientAction::FsWrite) => {
                PolicyDecision::Deny("orchestrator duty is not permitted to write files")
            }
            (TrustedProviderRole::Splitter, ClientAction::FsWrite) => {
                PolicyDecision::Deny("splitter duty is not permitted to write files")
            }
            (TrustedProviderRole::Handoff, ClientAction::FsWrite) => {
                PolicyDecision::Deny("handoff duty is not permitted to write files")
            }
            // 其余格子(全职责 FsRead/Terminal、Coder 全动作):随
            // authority/审批档。
            (_, _) => self.evaluate_permission_mode(),
        }
    }

    fn evaluate_permission_mode(&self) -> PolicyDecision {
        match self.permission_mode {
            ProviderPermissionMode::Auto => PolicyDecision::Allow,
            ProviderPermissionMode::Supervised => PolicyDecision::RequireApproval,
        }
    }
}

/// 直连会话(无受信授予)的承载角色→职责映射:Author 不在 `AdapterRole`
/// 中,只能经受信授予到达;其余承载角色映射到同名/对应职责,消费同一
/// 职责矩阵。
fn duty_of_wire_role(role: &AdapterRole) -> TrustedProviderRole {
    match role {
        AdapterRole::Orchestrator => TrustedProviderRole::Orchestrator,
        AdapterRole::Executor => TrustedProviderRole::Coder,
        AdapterRole::Reviewer => TrustedProviderRole::Reviewer,
        AdapterRole::WorkItemSplitter => TrustedProviderRole::Splitter,
        AdapterRole::Handoff => TrustedProviderRole::Handoff,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Task 1.2（BOOT-04/D1）：kimi 不承载 LC 根 recipe 自举执行器。
    #[test]
    fn bootstrap_executor_marker_is_denied_for_kimi_client_services() {
        use crate::cross_cutting::streaming_provider::{ProviderToolPolicy, ToolPolicyIntent};

        // marker 通道（任意角色装配面）→ Deny：kimi 不是 bootstrap provider。
        let marker_intent = ToolPolicyIntent::BootstrapExecutorMarker(test_marker());
        let marker_policy = ProviderToolPolicy {
            intent: marker_intent,
        };
        // 判定与 role 无关：通道级拒绝（任意角色装配面一律 Deny）。
        assert!(matches!(
            ClientServicePolicy::evaluate_bootstrap_executor_marker(Some(&marker_policy)),
            PolicyDecision::Deny(_)
        ));

        // 普通会话零变化：None → Allow；deny 意图归三 adapter 守卫，不在此裁决。
        assert_eq!(
            ClientServicePolicy::evaluate_bootstrap_executor_marker(None),
            PolicyDecision::Allow
        );
        let deny_policy = ProviderToolPolicy::deny_file_write_builtins();
        assert_eq!(
            ClientServicePolicy::evaluate_bootstrap_executor_marker(Some(&deny_policy)),
            PolicyDecision::Allow
        );
    }

    /// 构造测试用 marker：kimi 判定只看通道形态，不依赖凭据有效性
    ///（凭据/相位核验在 admission 层；此处仅证明通道被拒绝）。
    fn test_marker()
    -> crate::product::logical_codebase::provider_admission_preflight::BootstrapExecutorMarker {
        use crate::product::logical_codebase::aggregate_initialization::AggregateInitializationStepKind;
        use crate::product::logical_codebase::provider_admission_preflight::{
            BootstrapExecutorMarker, BootstrapPhaseCredential,
        };

        // 测试内构造：凭据字段在同模块树外不可见，此处经结构体字面量仅用于
        // kimi 通道判定（不进入 admission/spawn 判定路径）。
        let credential = BootstrapPhaseCredential::for_test(
            "project-kimi-test",
            "lc-kimi-test",
            "op-kimi-test",
            AggregateInitializationStepKind::PreCheck,
            "sha256:kimi-test-input",
            std::path::PathBuf::from("/tmp/kimi-test-root"),
        );
        BootstrapExecutorMarker::new(
            credential,
            crate::product::logical_codebase::policy::SessionPolicyAction::CodingTargetWrite,
            std::path::PathBuf::from("/tmp/kimi-test-root"),
            "kimi-channel-test",
        )
        .expect("test marker must be constructible")
    }

    /// 批2 Task 8 反转:Reviewer terminal 正向随权限档(有授权根与 Host
    /// 单路径的执行面);FsWrite 无有效窗口引用 fail-closed(有窗判定见
    /// 受信授予测试);FsRead 随权限档。
    #[test]
    fn reviewer_terminal_positively_allowed_but_fs_write_requires_window() {
        let auto = ClientServicePolicy::new(AdapterRole::Reviewer, ProviderPermissionMode::Auto);
        assert_eq!(auto.evaluate(ClientAction::Terminal), PolicyDecision::Allow);
        assert_eq!(auto.evaluate(ClientAction::FsRead), PolicyDecision::Allow);
        assert_eq!(
            auto.evaluate(ClientAction::FsWrite),
            PolicyDecision::Deny("reviewer duty requires a valid review window for fs writes")
        );

        let supervised =
            ClientServicePolicy::new(AdapterRole::Reviewer, ProviderPermissionMode::Supervised);
        assert_eq!(
            supervised.evaluate(ClientAction::Terminal),
            PolicyDecision::RequireApproval
        );
        assert_eq!(
            supervised.evaluate(ClientAction::FsWrite),
            PolicyDecision::Deny("reviewer duty requires a valid review window for fs writes")
        );
    }

    /// 无窗(直连 wire 会话)FsWrite 在 Supervised 下同样拒绝——审批不是
    /// 窗口的替代。
    #[test]
    fn reviewer_fs_write_still_denied_without_window_in_supervised() {
        let policy =
            ClientServicePolicy::new(AdapterRole::Reviewer, ProviderPermissionMode::Supervised);
        assert_eq!(
            policy.evaluate(ClientAction::FsWrite),
            PolicyDecision::Deny("reviewer duty requires a valid review window for fs writes")
        );
    }

    #[test]
    fn coding_requires_approval_in_supervised_mode() {
        let policy =
            ClientServicePolicy::new(AdapterRole::Executor, ProviderPermissionMode::Supervised);
        assert_eq!(
            policy.evaluate(ClientAction::Terminal),
            PolicyDecision::RequireApproval
        );
        assert_eq!(
            policy.evaluate(ClientAction::FsWrite),
            PolicyDecision::RequireApproval
        );
        assert_eq!(
            policy.evaluate(ClientAction::FsRead),
            PolicyDecision::RequireApproval
        );
    }

    #[test]
    fn coding_allows_in_auto_mode() {
        let policy = ClientServicePolicy::new(AdapterRole::Executor, ProviderPermissionMode::Auto);
        assert_eq!(
            policy.evaluate(ClientAction::Terminal),
            PolicyDecision::Allow
        );
        assert_eq!(
            policy.evaluate(ClientAction::FsWrite),
            PolicyDecision::Allow
        );
        assert_eq!(policy.evaluate(ClientAction::FsRead), PolicyDecision::Allow);
    }

    #[test]
    fn orchestrator_fs_read_allowed_in_auto() {
        let policy =
            ClientServicePolicy::new(AdapterRole::Orchestrator, ProviderPermissionMode::Auto);
        assert_eq!(policy.evaluate(ClientAction::FsRead), PolicyDecision::Allow);
    }

    #[test]
    fn orchestrator_terminal_routes_through_permission_mode() {
        let policy = ClientServicePolicy::new(
            AdapterRole::Orchestrator,
            ProviderPermissionMode::Supervised,
        );
        assert_eq!(
            policy.evaluate(ClientAction::Terminal),
            PolicyDecision::RequireApproval
        );
        assert_eq!(
            policy.evaluate(ClientAction::Terminal),
            policy.evaluate(ClientAction::FsRead)
        );
    }

    #[test]
    fn orchestrator_fs_write_denied() {
        let policy =
            ClientServicePolicy::new(AdapterRole::Orchestrator, ProviderPermissionMode::Auto);
        assert!(matches!(
            policy.evaluate(ClientAction::FsWrite),
            PolicyDecision::Deny(message) if message.contains("orchestrator duty")
        ));
    }

    #[test]
    fn reviewer_terminal_follows_permission_mode_and_executor_unchanged() {
        let reviewer =
            ClientServicePolicy::new(AdapterRole::Reviewer, ProviderPermissionMode::Auto);
        assert_eq!(
            reviewer.evaluate(ClientAction::Terminal),
            PolicyDecision::Allow
        );

        let executor =
            ClientServicePolicy::new(AdapterRole::Executor, ProviderPermissionMode::Supervised);
        assert_eq!(
            executor.evaluate(ClientAction::Terminal),
            PolicyDecision::RequireApproval
        );
        assert_eq!(
            executor.evaluate(ClientAction::FsWrite),
            PolicyDecision::RequireApproval
        );
    }

    /// 批2 Task 8 反转:Splitter/Handoff 不再整段拒绝——合法操作(读/
    /// 受约束 terminal)随权限档;fs 写拒绝(职责级,可区分文案)。
    #[test]
    fn splitter_and_handoff_legal_operations_not_denied_wholesale() {
        for (role, duty_label) in [
            (AdapterRole::WorkItemSplitter, "splitter duty"),
            (AdapterRole::Handoff, "handoff duty"),
        ] {
            let auto = ClientServicePolicy::new(role.clone(), ProviderPermissionMode::Auto);
            assert_eq!(
                auto.evaluate(ClientAction::FsRead),
                PolicyDecision::Allow,
                "{role:?}"
            );
            assert_eq!(
                auto.evaluate(ClientAction::Terminal),
                PolicyDecision::Allow,
                "{role:?}"
            );
            assert!(
                matches!(
                    auto.evaluate(ClientAction::FsWrite),
                    PolicyDecision::Deny(message) if message.contains(duty_label)
                ),
                "{role:?}"
            );
            let supervised = ClientServicePolicy::new(role, ProviderPermissionMode::Supervised);
            assert_eq!(
                supervised.evaluate(ClientAction::FsRead),
                PolicyDecision::RequireApproval
            );
            assert_eq!(
                supervised.evaluate(ClientAction::Terminal),
                PolicyDecision::RequireApproval
            );
        }
    }

    /// 批2 Task 8:F3 Task 4.1 的 24 格「角色整段拒绝」矩阵按岗位语义
    /// 重铸(旧角色断言反转)——4 承载角色(Orchestrator/WorkItemSplitter/
    /// Reviewer/Executor)× 2 权限档(Auto/Supervised)× 3 动作(FsRead/
    /// Terminal/FsWrite)共 24 格逐一冻结新决策(含冻结 Deny 文案);
    /// Handoff 作为表外边界按两种权限档全动作覆盖(读/terminal 随档,
    /// 写 Deny)。任何一格收紧/放宽都必须能被对应格断言捕获。
    #[test]
    fn kimi_duty_client_service_decision_matrix() {
        use ProviderPermissionMode::{Auto, Supervised};

        fn deny_orchestrator_write() -> PolicyDecision {
            PolicyDecision::Deny("orchestrator duty is not permitted to write files")
        }
        fn deny_splitter_write() -> PolicyDecision {
            PolicyDecision::Deny("splitter duty is not permitted to write files")
        }
        fn deny_handoff_write() -> PolicyDecision {
            PolicyDecision::Deny("handoff duty is not permitted to write files")
        }
        fn deny_reviewer_no_window() -> PolicyDecision {
            PolicyDecision::Deny("reviewer duty requires a valid review window for fs writes")
        }

        for (role, mode, action, expected) in [
            // Orchestrator:fs读/terminal 随权限档,fs写两种档恒 Deny。
            (
                AdapterRole::Orchestrator,
                Auto,
                ClientAction::FsRead,
                PolicyDecision::Allow,
            ),
            (
                AdapterRole::Orchestrator,
                Auto,
                ClientAction::Terminal,
                PolicyDecision::Allow,
            ),
            (
                AdapterRole::Orchestrator,
                Auto,
                ClientAction::FsWrite,
                deny_orchestrator_write(),
            ),
            (
                AdapterRole::Orchestrator,
                Supervised,
                ClientAction::FsRead,
                PolicyDecision::RequireApproval,
            ),
            (
                AdapterRole::Orchestrator,
                Supervised,
                ClientAction::Terminal,
                PolicyDecision::RequireApproval,
            ),
            (
                AdapterRole::Orchestrator,
                Supervised,
                ClientAction::FsWrite,
                deny_orchestrator_write(),
            ),
            // WorkItemSplitter:不再整段 Deny——读/terminal 随权限档,
            // fs写两种档恒 Deny(职责级文案)。
            (
                AdapterRole::WorkItemSplitter,
                Auto,
                ClientAction::FsRead,
                PolicyDecision::Allow,
            ),
            (
                AdapterRole::WorkItemSplitter,
                Auto,
                ClientAction::Terminal,
                PolicyDecision::Allow,
            ),
            (
                AdapterRole::WorkItemSplitter,
                Auto,
                ClientAction::FsWrite,
                deny_splitter_write(),
            ),
            (
                AdapterRole::WorkItemSplitter,
                Supervised,
                ClientAction::FsRead,
                PolicyDecision::RequireApproval,
            ),
            (
                AdapterRole::WorkItemSplitter,
                Supervised,
                ClientAction::Terminal,
                PolicyDecision::RequireApproval,
            ),
            (
                AdapterRole::WorkItemSplitter,
                Supervised,
                ClientAction::FsWrite,
                deny_splitter_write(),
            ),
            // Reviewer(无受信窗口引用的直连形态):terminal 正向随权限
            // 档,fs读随权限档,fs写两种档恒 Deny(无窗)。
            (
                AdapterRole::Reviewer,
                Auto,
                ClientAction::FsRead,
                PolicyDecision::Allow,
            ),
            (
                AdapterRole::Reviewer,
                Auto,
                ClientAction::Terminal,
                PolicyDecision::Allow,
            ),
            (
                AdapterRole::Reviewer,
                Auto,
                ClientAction::FsWrite,
                deny_reviewer_no_window(),
            ),
            (
                AdapterRole::Reviewer,
                Supervised,
                ClientAction::FsRead,
                PolicyDecision::RequireApproval,
            ),
            (
                AdapterRole::Reviewer,
                Supervised,
                ClientAction::Terminal,
                PolicyDecision::RequireApproval,
            ),
            (
                AdapterRole::Reviewer,
                Supervised,
                ClientAction::FsWrite,
                deny_reviewer_no_window(),
            ),
            // Executor(Coder 档):三动作均随权限档(Auto=Allow/
            // Supervised=审批)。
            (
                AdapterRole::Executor,
                Auto,
                ClientAction::FsRead,
                PolicyDecision::Allow,
            ),
            (
                AdapterRole::Executor,
                Auto,
                ClientAction::Terminal,
                PolicyDecision::Allow,
            ),
            (
                AdapterRole::Executor,
                Auto,
                ClientAction::FsWrite,
                PolicyDecision::Allow,
            ),
            (
                AdapterRole::Executor,
                Supervised,
                ClientAction::FsRead,
                PolicyDecision::RequireApproval,
            ),
            (
                AdapterRole::Executor,
                Supervised,
                ClientAction::Terminal,
                PolicyDecision::RequireApproval,
            ),
            (
                AdapterRole::Executor,
                Supervised,
                ClientAction::FsWrite,
                PolicyDecision::RequireApproval,
            ),
            // Handoff(表外边界):读/terminal 随权限档,fs写两种档 Deny。
            (
                AdapterRole::Handoff,
                Auto,
                ClientAction::FsRead,
                PolicyDecision::Allow,
            ),
            (
                AdapterRole::Handoff,
                Auto,
                ClientAction::Terminal,
                PolicyDecision::Allow,
            ),
            (
                AdapterRole::Handoff,
                Auto,
                ClientAction::FsWrite,
                deny_handoff_write(),
            ),
            (
                AdapterRole::Handoff,
                Supervised,
                ClientAction::FsRead,
                PolicyDecision::RequireApproval,
            ),
            (
                AdapterRole::Handoff,
                Supervised,
                ClientAction::Terminal,
                PolicyDecision::RequireApproval,
            ),
            (
                AdapterRole::Handoff,
                Supervised,
                ClientAction::FsWrite,
                deny_handoff_write(),
            ),
        ] {
            let policy = ClientServicePolicy::new(role.clone(), mode.clone());
            assert_eq!(
                policy.evaluate(action),
                expected,
                "{role:?} x {mode:?} x {action:?}"
            );
        }
    }

    // ==== 批2 Task 8:受信岗位授予(T2 冻结执行合同消费面)====

    use super::test_fixtures::frozen_duty_grant;
    use serde_json::json;

    /// Author 岗位(经 Orchestrator 承载):代码编辑 fs 写请求拒绝
    /// (AuthorToolPolicy v1 `builtin_write_code_file` 语义,kimi 侧映射到
    /// fs 写服务,不按扩展名另建过滤);read/terminal 随权限档可用
    /// (report/document 走平台通道,不经 client services);岗位拒绝不随
    /// 权限档放宽。
    #[test]
    fn author_grant_denies_code_edit_fs_write_but_allows_read_and_terminal() {
        let auto =
            ClientServicePolicy::new(AdapterRole::Orchestrator, ProviderPermissionMode::Auto)
                .with_trusted_duty(frozen_duty_grant(
                    TrustedProviderRole::Author,
                    AdapterRole::Orchestrator,
                    None,
                ));
        assert_eq!(auto.evaluate(ClientAction::FsRead), PolicyDecision::Allow);
        assert_eq!(auto.evaluate(ClientAction::Terminal), PolicyDecision::Allow);
        assert_eq!(
            auto.evaluate(ClientAction::FsWrite),
            PolicyDecision::Deny(
                "author duty denies code-editing fs writes (builtin_write_code_file); author documents go through the platform channel"
            )
        );

        let supervised = ClientServicePolicy::new(
            AdapterRole::Orchestrator,
            ProviderPermissionMode::Supervised,
        )
        .with_trusted_duty(frozen_duty_grant(
            TrustedProviderRole::Author,
            AdapterRole::Orchestrator,
            None,
        ));
        assert_eq!(
            supervised.evaluate(ClientAction::FsRead),
            PolicyDecision::RequireApproval
        );
        assert_eq!(
            supervised.evaluate(ClientAction::Terminal),
            PolicyDecision::RequireApproval
        );
        assert_eq!(
            supervised.evaluate(ClientAction::FsWrite),
            PolicyDecision::Deny(
                "author duty denies code-editing fs writes (builtin_write_code_file); author documents go through the platform channel"
            )
        );
    }

    /// Reviewer 携带有效窗口引用(T20 接真窗;此处 T2 冻结合同内的
    /// 窗口引用即受信 fixture):授权根内 FsWrite 随 authority/审批判定
    /// (Auto=Allow/Supervised=审批);terminal 正向。
    #[test]
    fn reviewer_grant_with_window_allows_fs_write_by_permission_mode() {
        let grant = frozen_duty_grant(
            TrustedProviderRole::Reviewer,
            AdapterRole::Reviewer,
            Some("window_0001"),
        );
        let auto = ClientServicePolicy::new(AdapterRole::Reviewer, ProviderPermissionMode::Auto)
            .with_trusted_duty(grant.clone());
        assert_eq!(auto.evaluate(ClientAction::FsWrite), PolicyDecision::Allow);
        assert_eq!(auto.evaluate(ClientAction::Terminal), PolicyDecision::Allow);

        let supervised =
            ClientServicePolicy::new(AdapterRole::Reviewer, ProviderPermissionMode::Supervised)
                .with_trusted_duty(grant);
        assert_eq!(
            supervised.evaluate(ClientAction::FsWrite),
            PolicyDecision::RequireApproval
        );
    }

    /// 授予的承载角色与会话 wire 角色不一致 = 装配错位,fail-closed
    /// (T2/gateway 装配面已校验,此处纵深防御)。
    #[test]
    fn carrier_mismatch_grant_is_fail_closed() {
        let grant = frozen_duty_grant(TrustedProviderRole::Author, AdapterRole::Orchestrator, None);
        let policy = ClientServicePolicy::new(AdapterRole::Executor, ProviderPermissionMode::Auto)
            .with_trusted_duty(grant);
        for action in [
            ClientAction::Terminal,
            ClientAction::FsRead,
            ClientAction::FsWrite,
        ] {
            assert!(
                matches!(policy.evaluate(action), PolicyDecision::Deny(_)),
                "{action:?}"
            );
        }
    }

    /// 广告面与实际拒绝同一岗位合同:Author 职责不广告代码编辑 fs 写
    /// (`writeTextFile:false`);其余职责保持既有布尔方言全集(广告≠
    /// 授权,请求仍逐条过 evaluate)。
    #[test]
    fn advertised_capabilities_follow_the_same_duty_contract() {
        let author =
            ClientServicePolicy::new(AdapterRole::Orchestrator, ProviderPermissionMode::Auto)
                .with_trusted_duty(frozen_duty_grant(
                    TrustedProviderRole::Author,
                    AdapterRole::Orchestrator,
                    None,
                ));
        let capabilities = author.advertised_capabilities();
        assert_eq!(capabilities["fs"]["readTextFile"], json!(true));
        assert_eq!(capabilities["fs"]["writeTextFile"], json!(false));
        assert_eq!(capabilities["terminal"], json!(true));

        for (duty, carrier, window) in [
            (
                TrustedProviderRole::Orchestrator,
                AdapterRole::Orchestrator,
                None,
            ),
            (
                TrustedProviderRole::Splitter,
                AdapterRole::WorkItemSplitter,
                None,
            ),
            (TrustedProviderRole::Handoff, AdapterRole::Handoff, None),
            (TrustedProviderRole::Coder, AdapterRole::Executor, None),
            (
                TrustedProviderRole::Reviewer,
                AdapterRole::Reviewer,
                Some("window_0001"),
            ),
        ] {
            let policy = ClientServicePolicy::new(carrier.clone(), ProviderPermissionMode::Auto)
                .with_trusted_duty(frozen_duty_grant(duty, carrier, window));
            let capabilities = policy.advertised_capabilities();
            assert_eq!(capabilities["fs"]["writeTextFile"], json!(true), "{duty:?}");
            assert_eq!(capabilities["fs"]["readTextFile"], json!(true), "{duty:?}");
            assert_eq!(capabilities["terminal"], json!(true), "{duty:?}");
        }
    }
}

/// 批2 Task 8 测试共享 fixture:受信岗位授予(T2 冻结执行合同 →
/// client-services 消费面)。仅供 crate 内 cfg(test) 消费(policy/mod/
/// tool_policy 三处测试共用,不复制三份)。
#[cfg(test)]
pub(crate) mod test_fixtures {
    use crate::cross_cutting::provider_execution::TrustedProviderRole;
    use crate::protocol::contracts::AdapterRole;

    use super::TrustedDutyGrant;

    /// 经真实 `freeze_provider_execution` 装配矩阵(职责×profile×窗口×
    /// author 岗位策略 fail-closed)产出冻结合同,再提取 client-services
    /// 消费面授予——不经 wire role/prompt 铸造。
    pub(crate) fn frozen_duty_grant(
        duty: TrustedProviderRole,
        carrier: AdapterRole,
        window: Option<&str>,
    ) -> TrustedDutyGrant {
        use crate::cross_cutting::provider_execution::{
            AuthorToolPolicy, ProviderExecutionProfile, TrustedRunIdentity,
            freeze_provider_execution,
        };

        let mut record = durable_run_record();
        record.adapter_role = carrier;
        let identity = TrustedRunIdentity::from_durable_run(&record, "ws_session_t8", duty);
        let author_policy = (duty == TrustedProviderRole::Author).then(AuthorToolPolicy::author_v1);
        let profile = if duty == TrustedProviderRole::Reviewer {
            ProviderExecutionProfile::ReviewerVerify
        } else {
            ProviderExecutionProfile::HostCoding
        };
        let frozen = freeze_provider_execution(
            &identity,
            profile,
            author_policy,
            "scope_run_t8".to_string(),
            window.map(str::to_string),
        )
        .expect("freeze provider execution");
        TrustedDutyGrant::from_frozen_execution(&frozen)
    }

    fn durable_run_record() -> crate::protocol::contracts::ProviderRunRecord {
        crate::protocol::contracts::ProviderRunRecord {
            provider_run_id: "run_t8_0001".to_string(),
            node_id: "N2".to_string(),
            provider_type: crate::protocol::contracts::ProviderType::KimiCode,
            runtime_role: crate::protocol::contracts::RuntimeRole::Orchestrator,
            adapter_role: AdapterRole::Orchestrator,
            provider_capability_ref: "cap_kimi_code_1_4_0".to_string(),
            adapter_compatibility_ref: "compat_1".to_string(),
            context_package_ref: "ctx_1".to_string(),
            adapter_input_ref: "ain_1".to_string(),
            adapter_output_ref: "aout_1".to_string(),
            raw_artifact_refs: Vec::new(),
            exit_code: Some(0),
            error_code: None,
            error_details: None,
            stdout_ref: None,
            stderr_ref: None,
            structured_output_ref: None,
            files_modified: Vec::new(),
            status: crate::protocol::contracts::ProviderRunStatus::Running,
            started_at: "2026-10-17T00:00:00Z".to_string(),
            completed_at: None,
            duration_ms: None,
            timeout_status: crate::protocol::contracts::TimeoutStatus::NotTimedOut,
            retry_count: 0,
            approval_policy: crate::protocol::contracts::ApprovalPolicy::OnRequest,
            sandbox_mode: crate::protocol::contracts::SandboxMode::FullAccess,
            constraint_check_ref: None,
            traceability_binding_refs: Vec::new(),
        }
    }
}
