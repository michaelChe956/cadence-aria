//! Role + permission-mode action policy for kimi client services.
//!
//! Declaring `clientCapabilities` never authorizes execution: every
//! terminal/fs request must pass this policy first. The policy mirrors the
//! session policy envelope's role boundaries (reviewer read-only, coding
//! limited to the target worktree) and the provider permission mode
//! (auto vs supervised).

use crate::cross_cutting::streaming_provider::ProviderPermissionMode;
use crate::protocol::contracts::AdapterRole;

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
}

impl ClientServicePolicy {
    pub fn new(role: AdapterRole, permission_mode: ProviderPermissionMode) -> Self {
        Self {
            role,
            permission_mode,
        }
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
        match self.role {
            AdapterRole::Reviewer => match action {
                ClientAction::Terminal | ClientAction::FsWrite => {
                    PolicyDecision::Deny("reviewer role is read-only for terminal and fs writes")
                }
                ClientAction::FsRead => self.evaluate_permission_mode(),
            },
            // The coding agent (Executor) is confined to its target worktree:
            // the authorized root IS the worktree, so confinement is enforced
            // by the openat/bwrap sandbox rather than an extra role branch.
            AdapterRole::Executor => match action {
                ClientAction::Terminal | ClientAction::FsRead | ClientAction::FsWrite => {
                    self.evaluate_permission_mode()
                }
            },
            // The planning agent may inspect its workspace and use the constrained,
            // sandboxed terminal, but may never modify files through client services.
            AdapterRole::Orchestrator => match action {
                ClientAction::FsRead | ClientAction::Terminal => self.evaluate_permission_mode(),
                ClientAction::FsWrite => {
                    PolicyDecision::Deny("planning role is not permitted to write files")
                }
            },
            // WorkItemSplitter, Handoff and future roles do not receive host execution
            // from the weak-model kimi client.
            _ => PolicyDecision::Deny("role is not permitted to use kimi client services"),
        }
    }

    fn evaluate_permission_mode(&self) -> PolicyDecision {
        match self.permission_mode {
            ProviderPermissionMode::Auto => PolicyDecision::Allow,
            ProviderPermissionMode::Supervised => PolicyDecision::RequireApproval,
        }
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

    #[test]
    fn reviewer_denies_terminal_and_fs_write_but_allows_fs_read() {
        let policy = ClientServicePolicy::new(AdapterRole::Reviewer, ProviderPermissionMode::Auto);
        assert_eq!(
            policy.evaluate(ClientAction::Terminal),
            PolicyDecision::Deny("reviewer role is read-only for terminal and fs writes")
        );
        assert_eq!(
            policy.evaluate(ClientAction::FsWrite),
            PolicyDecision::Deny("reviewer role is read-only for terminal and fs writes")
        );
        assert_eq!(policy.evaluate(ClientAction::FsRead), PolicyDecision::Allow);
    }

    #[test]
    fn reviewer_denies_fs_write_even_in_supervised() {
        let policy =
            ClientServicePolicy::new(AdapterRole::Reviewer, ProviderPermissionMode::Supervised);
        assert!(matches!(
            policy.evaluate(ClientAction::FsWrite),
            PolicyDecision::Deny(_)
        ));
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
            PolicyDecision::Deny(message) if message.contains("planning")
        ));
    }

    #[test]
    fn reviewer_and_executor_unchanged() {
        let reviewer =
            ClientServicePolicy::new(AdapterRole::Reviewer, ProviderPermissionMode::Auto);
        assert!(matches!(
            reviewer.evaluate(ClientAction::Terminal),
            PolicyDecision::Deny(_)
        ));
        assert!(matches!(
            reviewer.evaluate(ClientAction::FsWrite),
            PolicyDecision::Deny(_)
        ));

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

    #[test]
    fn work_item_splitter_and_handoff_still_denied() {
        for role in [AdapterRole::WorkItemSplitter, AdapterRole::Handoff] {
            let policy = ClientServicePolicy::new(role, ProviderPermissionMode::Auto);
            assert!(matches!(
                policy.evaluate(ClientAction::FsRead),
                PolicyDecision::Deny(_)
            ));
            assert!(matches!(
                policy.evaluate(ClientAction::Terminal),
                PolicyDecision::Deny(_)
            ));
            assert!(matches!(
                policy.evaluate(ClientAction::FsWrite),
                PolicyDecision::Deny(_)
            ));
        }
    }

    /// F3 Task 4.1 修复轮（restrict-role-write-tools，GC12）：kimi 四角色
    /// 完整笛卡尔决策矩阵回归锁 = 4 角色（Orchestrator/WorkItemSplitter/
    /// Reviewer/Executor）× 2 权限档（Auto/Supervised）× 3 动作（FsRead/
    /// Terminal/FsWrite）共 24 格逐一冻结既有决策（含冻结 Deny 文案）；
    /// Handoff 作为表外边界额外按两种权限档全动作覆盖（整表 Deny）。
    /// 任何一格收紧/放宽都必须能被对应格断言捕获——既有 client services
    /// 决策不因本 change 变化。
    #[test]
    fn kimi_four_role_client_service_table_stays_unchanged() {
        use ProviderPermissionMode::{Auto, Supervised};

        fn deny_planning_write() -> PolicyDecision {
            PolicyDecision::Deny("planning role is not permitted to write files")
        }
        fn deny_services() -> PolicyDecision {
            PolicyDecision::Deny("role is not permitted to use kimi client services")
        }
        fn deny_reviewer_write_side() -> PolicyDecision {
            PolicyDecision::Deny("reviewer role is read-only for terminal and fs writes")
        }

        for (role, mode, action, expected) in [
            // Orchestrator：fs读/terminal 随权限档，fs写两种档恒 Deny。
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
                deny_planning_write(),
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
                deny_planning_write(),
            ),
            // WorkItemSplitter：无宿主执行，整表 Deny（两种档 × 三动作全覆盖）。
            (
                AdapterRole::WorkItemSplitter,
                Auto,
                ClientAction::FsRead,
                deny_services(),
            ),
            (
                AdapterRole::WorkItemSplitter,
                Auto,
                ClientAction::Terminal,
                deny_services(),
            ),
            (
                AdapterRole::WorkItemSplitter,
                Auto,
                ClientAction::FsWrite,
                deny_services(),
            ),
            (
                AdapterRole::WorkItemSplitter,
                Supervised,
                ClientAction::FsRead,
                deny_services(),
            ),
            (
                AdapterRole::WorkItemSplitter,
                Supervised,
                ClientAction::Terminal,
                deny_services(),
            ),
            (
                AdapterRole::WorkItemSplitter,
                Supervised,
                ClientAction::FsWrite,
                deny_services(),
            ),
            // Reviewer：terminal/fs写两种档恒 Deny，fs读随权限档。
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
                deny_reviewer_write_side(),
            ),
            (
                AdapterRole::Reviewer,
                Auto,
                ClientAction::FsWrite,
                deny_reviewer_write_side(),
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
                deny_reviewer_write_side(),
            ),
            (
                AdapterRole::Reviewer,
                Supervised,
                ClientAction::FsWrite,
                deny_reviewer_write_side(),
            ),
            // Executor（Coder 档）：三动作均随权限档（Auto=Allow/Supervised=审批）。
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
            // Handoff（表外边界）：两种权限档 × 三动作整表 Deny。
            (
                AdapterRole::Handoff,
                Auto,
                ClientAction::FsRead,
                deny_services(),
            ),
            (
                AdapterRole::Handoff,
                Auto,
                ClientAction::Terminal,
                deny_services(),
            ),
            (
                AdapterRole::Handoff,
                Auto,
                ClientAction::FsWrite,
                deny_services(),
            ),
            (
                AdapterRole::Handoff,
                Supervised,
                ClientAction::FsRead,
                deny_services(),
            ),
            (
                AdapterRole::Handoff,
                Supervised,
                ClientAction::Terminal,
                deny_services(),
            ),
            (
                AdapterRole::Handoff,
                Supervised,
                ClientAction::FsWrite,
                deny_services(),
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
}
