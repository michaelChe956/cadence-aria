# S2a 无远端本地交付后端(实施报告)

- 日期:2026-10-14
- 工位:S2a(本地交付后端;设计权威 history://DesignPushLocal2「Q1/Q1'/切片 S2a」)
- 注:`S2a-report.md` 已被更早的「S2a 续跑全自动化」占用,本报告另名存放。

## 语义

- 新增 `PushStatus::SkippedNotConfigured`,序列化 `skipped_not_configured`;`PushStatus::is_delivered()` = `Pushed | SkippedNotConfigured`。
- `GitWorkspaceService::git_remote_configured(worktree, remote)`:以 `git remote` 列表精确匹配判定,不看 `get-url`/`ls-remote` 退出码。
- `execute_review_request`:`phase==PushStarted` 之后、`remote_head`/`git_push` 之前判定;未配置 → `finish_review_git_operation(SkippedNotConfigured)`。新进入与「停在 PushStarted 的 journal 重入」同路径收敛。已配置但不可达/被拒保持原行为(`Indeterminate` 报错 / `Failed`)。
- ReviewRequest:`kind` 仍 `GitBranchOnly`,`push_error=None`,`manual_instructions` 为「本地交付:分支 X / commit Y,未配置远端,未推送」。
- timeline:Completed,摘要「本地交付(未配置远端,已跳过推送)」。
- `issue_delivery.rs`(AllPushed)与 `plan_group_projection/mod.rs`(delivered)统一走 `is_delivered()`。
- `dto.rs push_status_text` 补 `skipped_not_configured`。
- `reopen_failed_review_git_operation` 未改:仍只重开 `Failed`,补配 origin 不自动补推(非目标)。
- `pointer_publication_coordinator.rs` 无 exhaustive match,编译通过,未改(其无 origin 行为属设计开放项,未触碰)。

## 测试(均写入既有文件)

- `retry_push.rs`:无 origin 执行→Skipped/journal Completed/attempt 有 review_request_id/timeline Completed;PushStarted 重入收敛;已配置不可达首次与重入均 Err 且 journal 停 PushStarted。
- `git_workspace_service/tests.rs`:`git_remote_configured` 无/有(不可达)/他名。
- `coding_models/tests.rs`:serde 往返 + 谓词。
- `tests/issue_delivery.rs`、`plan_group_projection/tests.rs`、`delivery_completion.rs`:Pushed+Skipped → AllPushed / AllDelivered / issue Completed。

## 验证

- 红:仅回退 internal_pr_review.rs / issue_delivery.rs / plan_group_projection/mod.rs 到 HEAD,5 个新行为测试 FAILED(无 origin 执行、PushStarted 重入、AllPushed、AllDelivered、issue Completed);不可达守卫与既有重推测试通过。
- 绿:恢复后 `cargo test --lib -- retry_push skipped_not_configured git_remote_configured delivery_completion` 9 passed。
- 回归:`cargo test --lib -- maybe_complete_issue_delivery retry_push multi_target_delivery git_operation_reconcile push_decision issue_delivery plan_group_projection coding_models git_workspace_service pointer_publication pointer_review_request coding_attempt_store::tests::git_operation runner_support coding_attempt_dto runner_fallback_commit cross_target_check group_completion_authority sc_group_dependency_gate` 198 passed / 0 failed。
- 未验证:前端(`PushStatus` TS 联合类型与标签属 FE-1);全量 cargo test;集成测试 `tests/`。
