// Task 2.6（lc-root-initialization）：LC Coder/retry 的 root launch 解析。
// 物理拆分（large_file_guard 1200 行上限）：经 include! 挂载，模块域与
// provider_retry.rs 相同。
impl CodingWorkspaceEngine {
    /// Task 2.6（REQ-ENV-10/ENV-11，映射 openspec tasks 2.4 的 Coder/retry 行）：
    /// LC Coder/retry 的 root launch 解析。cwd = gateway 冻结的 canonical
    /// authority root（manifest `provider_context_root`，与 Task 2.1/2.3 的
    /// author/revision launch 同源）；target = attempt target snapshot 的成员
    /// checkout（嵌套 coding worktree）；唯一 writable root 恒为该 worktree——
    /// cwd 重绑 root 绝不扩大写边界（REQ-ENV-03）。
    ///
    /// 分流与 `resolve_launch_policy_for_role` 一致：未注入 gateway 或非逻辑
    /// attempt（`target_snapshot` 为 `None`）返回 `Ok(None)`（Legacy 直连/由
    /// stream 层 fail-closed）；逻辑 attempt 的解析/校验失败错误透传，绝不
    /// 静默回落 member cwd 直连。
    pub(crate) fn resolve_coder_root_launch_policy(
        &self,
        attempt: &CodingExecutionAttempt,
        worktree_path: &Path,
    ) -> Result<Option<ValidatedSessionLaunchPolicy>, ProviderGatewayError> {
        let Some(gateway) = self.logical_provider_gateway.as_ref() else {
            return Ok(None);
        };
        let Some(snapshot) = attempt.target_snapshot.as_ref() else {
            return Ok(None);
        };
        let root = gateway.authority_root().to_path_buf();
        let role_config = self.store.get_role_provider_config_snapshot(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
        )?;
        // Coder 角色 provider 恒存在（reviewer 缺失门是 reviewer 角色的兜底）；
        // 集中映射 fail-closed：不支持 gateway dialect 的 provider 显式失败。
        let provider_ref = provider_ref_for_name(&role_config.coder)?;
        let request = SessionLaunchRequest {
            project_id: attempt.project_id.clone(),
            provider: provider_ref,
            action: SessionPolicyAction::CodingTargetWrite,
            target: PolicyTarget::checkout(
                snapshot.logical_repository_id.0.to_string(),
                snapshot.checkout_id.0.to_string(),
                worktree_path.to_path_buf(),
            ),
            // Task 2.6：cwd=canonical root（REQ-ENV-10），独立于 target；readable
            // roots 随 cwd root 化（成员位于 root 子树内的聚合布局，与 Task
            // 2.1/2.3 同口径）；writable root 恒=target worktree（REQ-ENV-11）。
            working_directory: root.clone(),
            readable_roots: vec![root],
            writable_roots: vec![worktree_path.to_path_buf()],
            config_artifact_ref: "sha256:managed-config-artifact".to_string(),
        };
        Ok(Some(gateway.validate(request)?))
    }
}

impl CodingWorkspaceEngine {
    /// Task 1b 段③:LC Coder/retry root launch 的 prepare 入口。request 构造
    /// 与 `prepare_streaming_launch_for_role(Coder)` 同源(Coder 恒
    /// `role_config.coder`,cwd=canonical root、target/writable root=attempt
    /// worktree),prepare 前绑定 run-bound sink;分流与
    /// `resolve_coder_root_launch_policy` 一致(未注入 gateway 或非逻辑
    /// attempt 返回 `Ok(None)`,Legacy 直连)。
    pub(crate) fn prepare_coder_root_launch_streaming(
        &self,
        attempt: &CodingExecutionAttempt,
        worktree_path: &Path,
        input: crate::cross_cutting::streaming_provider::StreamingProviderInput,
    ) -> Result<
        Option<crate::cross_cutting::session_launch::ValidatedStreamingProviderInput>,
        ProviderGatewayError,
    > {
        self.prepare_streaming_launch_for_role(
            attempt,
            crate::product::coding_models::CodingProviderRole::Coder,
            worktree_path,
            input,
        )
    }
}
