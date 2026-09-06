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
