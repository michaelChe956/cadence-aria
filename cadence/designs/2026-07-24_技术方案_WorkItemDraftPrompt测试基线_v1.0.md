# Work Item Draft Prompt 测试基线与提醒机制

## 目标

将本次 Draft Prompt 的真实 Provider 验证沉淀为可重复的人工授权基线。它用于发现 Prompt 文案或结构化契约变更造成的首次输出格式/语义回归，不构成产品功能、CI 或自动化发布门禁。

## 基线

- 仅使用本机 Claude Code；每个 Provider 调用均为新的无持久化 `plan` 会话。
- 临时入口只存在于 `/tmp/aria-draft-prompt-validation/`，调用当前分支的 `build_work_item_draft_invocation`，再使用既有 `parse_structured_output`、`parse_work_item_draft_output` 与 `WorkItemDraftLocalValidator::validate` 判定。
- 两个 Case 各需 10 个有效首次输出，且必须各为 10/10 `pass`；因此满足不低于 95% 的目标。
- Provider 启动失败、超时、非零退出为 `provider_inconclusive`，不消耗有效样本；连续两次或累计第三次中断后停止并向操作者报告。
- 每次调用上限为 480 秒。该值来自本次成功样本：Case A 约 228 秒，Case B 约 394 秒，为慢 Case 保留约 86 秒余量。
- 只输出 Case、序号、结果码与耗时；不保存或提交完整 Prompt、Provider Draft、认证信息或目标仓库内容。

## 固定 Case

### Case A：后端登录会话

- `outline_id=outline_backend_session`，`logical_work_item_id=wi_backend_session`。
- 目标为登录会话过期检测与刷新 API；专有写入范围是 `src/product/session.rs` 和 `src/web/session_handlers.rs`；禁止修改 `web/**`。
- 可信验证命令为 `cargo test --locked --lib session`。

### Case B：紧凑时长格式化

- `outline_id=outline_implement_compact_duration`，`logical_work_item_id=wi_implement_compact_duration`。
- 目标是在 `src/formatCompactDuration.mjs` 导出 `formatCompactDuration(totalSeconds)`；覆盖 `0`、`3599`、`3600`、`86400` 的紧凑格式边界。
- 仅允许写入 `src/formatCompactDuration.mjs`；禁止修改 `test/formatCompactDuration.test.mjs` 和 `package.json`；可信验证命令目录为空。
- 它回归两类历史错误：`unknown_done_when_ref` 与 `missing_required_verification_command`。

## 触发与提醒

修改 Work Item Draft Prompt、其 Canonical Contract 投影或 Prompt 结构化约束时，交付前必须向操作者明确提醒执行本基线，并取得调用 Claude Code 的单独授权。提醒不自动触发 Provider 调用，不添加 CI、Hook、CLI 或持久化评估报告。

具体触发文件与提醒措辞由已启用的 `cadence/project-rules/work-item-draft-prompt-validation.md` 规定。
