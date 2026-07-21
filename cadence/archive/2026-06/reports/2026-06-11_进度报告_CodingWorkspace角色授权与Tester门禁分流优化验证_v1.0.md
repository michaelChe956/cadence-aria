# CodingWorkspace 角色授权与 Tester 门禁分流优化验证报告

## 基本信息

- 验证日期：2026-06-11
- 分支：`bugfix_test_branch`
- Worktree：`/Users/michaelche/Documents/git-folder/github-folder/cadence-aria/.worktrees/bugfix_test_branch`
- 设计文档：`cadence/designs/2026-06-11_技术方案_CodingWorkspace角色授权与Tester门禁分流优化_v1.0.md`
- 实施计划：`cadence/plans/2026-06-11_计划文档_实施计划_CodingWorkspace角色授权与Tester门禁分流优化_v1.0.md`

## 自动化验证

- `cargo fmt --check`：通过，退出码 0。
- `cargo clippy --all-targets --all-features --locked -- -D warnings`：通过，退出码 0。
- `cargo check --locked`：通过，退出码 0。
- `cargo test --locked`：通过，退出码 0；lib 223、`it_core` 140、`it_interactive` 43、`it_product` 107、`it_provider` 51、`it_task_run` 31、`it_web` 102，doc tests 0。
- `pnpm -C web test`：通过，38 个测试文件、328 个测试全部通过。
- `pnpm -C web build`：通过，退出码 0；Vite 输出单个 chunk 超过 500 kB 的体积警告，未阻塞构建。

## Controlled E2E 健康检查

- 后端 `http://127.0.0.1:4317/api/health`：返回 `{"status":"ok"}`。
- 前端 `http://127.0.0.1:5173/`：返回 `HTTP/1.1 200 OK`。
- 前端代理 `http://127.0.0.1:5173/api/health`：返回 `{"status":"ok"}`。
- 说明：4317/5173 端口在检查前已有本仓库开发服务监听；本次仅复用现有服务做健康检查，没有主动停止这些进程。

## 行为验收

- Tester 默认 `auto`：由 `role_provider_config_deserializes_legacy_json_with_default_permission_modes`、`coding_tester_uses_role_permission_mode_auto` 和前端 role config 测试覆盖。
- Coder / CodeReviewer / InternalReviewer 默认 `supervised`，Analyst 默认 `auto`：由 role config 反序列化、持久化和 snapshot 测试覆盖。
- Markdown plan repair：由 `tester_repairs_markdown_plan_output_before_blocking` 覆盖。
- repair 失败停在 Testing gate：由 `coding_ws_testing_blocked_does_not_start_analyst_automatically` 覆盖。
- Testing blocked 不进 Analyst：由 WebSocket 集成测试覆盖，且断言没有新增 Rework 节点。
- failed with evidence 进入 Analyst：由 `testing_report_requires_evidence_before_analyst_rework` 覆盖。
- Testing blocked 前端文案：由 `renders tester contract blocked gate as blocked instead of failed test` 覆盖，显示“测试被阻塞”，不显示“测试失败”。

## 风险与遗留

- 未执行人工页面长流程创建真实业务仓库 attempt；本轮完成自动化回归和服务健康检查。
- 当前会话未暴露可调用的 in-app Browser 工具，因此未记录浏览器截图类证据。
