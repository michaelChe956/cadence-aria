// Task 2.8（lc-root-initialization，REQ-PLN-07）：prompt 路由引用的 root cwd
// 解析。物理拆分（1200 行守卫）：经 include! 挂载，模块域与 lifecycle.rs 相同
//（与 provider_drive/author_root_launch.inc.rs 同型）。
impl WorkspaceEngine {
    /// Story/Design author/revision/review prompt 注入用的路由引用上下文。
    ///
    /// 逻辑会话(已注入 gateway 且有 planning cwd)时经 gateway `validate` 冻结
    /// `PlanningReadOnly` envelope 后派生 `Logical`;无 gateway/无 cwd/validate 失败
    /// 一律回落 `Legacy`(与改造前 `_legacy()` 字节一致)。C-2:投影 request 的
    /// provider ref 由 `session.author_provider` 经集中映射派生——author 配置
    /// Codex 时以 Codex ref 校验(被 REQ-ENV-05 路由级硬门阻断后回落 Legacy)，
    /// Pi/KimiCode 等无 gateway dialect 的 provider直接回落 Legacy(prompt 路由
    /// 引用不假装 Logical；真实启动在 gateway 集中映射处 fail-closed)。
    ///
    /// 只用于 prompt 路由引用分流,不改变 provider 启动路径(Task 4 范围)。
    pub(crate) fn routing_reference_context(
        &self,
    ) -> crate::product::cadence_skills::routing_reference::RoutingReferenceContext {
        use crate::product::cadence_skills::routing_reference::{
            RoutingReferenceContext, routing_reference_context_from_policy,
        };
        use crate::product::logical_codebase::{PolicyTarget, ProviderRef, SessionLaunchRequest};

        let Some(gateway) = self.logical_provider_gateway() else {
            return RoutingReferenceContext::Legacy;
        };
        let Some((project_id, working_dir)) = self.logical_planning_launch() else {
            return RoutingReferenceContext::Legacy;
        };
        // 镜像 factory 的 canonicalize 语义:aggregate_root target worktree 用
        // canonicalize 后形态,失败回退原值(resolver 会再 canonicalize 复验)。
        let target_worktree =
            std::fs::canonicalize(&working_dir).unwrap_or_else(|_| working_dir.clone());
        // Task 2.8（REQ-PLN-07）：cwd 恒为 gateway 冻结的 canonical root（与真实
        // 启动链同源）；成员 checkout 只作 target。prompt 路由引用面解析失败仍
        // 回落 Legacy——真实启动路径在 launch 链 fail-closed，不受此处影响。
        let root = gateway.authority_root().to_path_buf();
        // C-2:provider ref 随 session.author_provider;不支持的 provider 回落
        // Legacy(启动路径在集中映射处 fail-closed,此处仅 prompt 路由引用)。
        let provider =
            ProviderRef::from_provider_name(&self.session.author_provider, "cap_managed_snapshot");
        let request = match provider {
            Ok(provider) => SessionLaunchRequest {
                project_id,
                provider,
                action: crate::product::logical_codebase::SessionPolicyAction::PlanningReadOnly,
                target: PolicyTarget::aggregate_root(target_worktree),
                // cwd=root ≠ target（结构体字面量显式分离；`planning()` 构造器的
                // cwd==target 默认仅适用单仓形态）。
                working_directory: root.clone(),
                readable_roots: vec![root],
                writable_roots: Vec::new(),
                config_artifact_ref: "sha256:managed-config-artifact".to_string(),
            },
            Err(error) => {
                tracing::warn!(
                    %error,
                    "workspace_engine routing_reference_context: author provider has no gateway dialect, falling back to Legacy"
                );
                return RoutingReferenceContext::Legacy;
            }
        };
        match gateway.validate(request) {
            Ok(policy) => routing_reference_context_from_policy(&policy),
            Err(error) => {
                tracing::warn!(
                    %error,
                    "workspace_engine routing_reference_context: gateway validate failed, falling back to Legacy"
                );
                RoutingReferenceContext::Legacy
            }
        }
    }
}
