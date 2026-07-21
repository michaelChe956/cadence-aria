# Workspace Review Gate 与内存治理验证报告

## 变更范围

- Review Gate 分级：完成
- 非阻塞 review 允许确认当前版本：完成
- 刷新恢复消息 hydration：完成
- Bounded content cache：完成

## 自动化验证

| 命令 | 结果 |
| --- | --- |
| `cargo test --locked --lib parse_review_verdict` | 通过 |
| `cargo test --locked --lib optional_review_findings_enter_human_confirm_for_all_workspace_types` | 通过 |
| `cargo test --locked --lib strong_review_findings_enter_review_decision_for_all_workspace_types` | 通过 |
| `cargo test --locked --lib review_prompt_limits_revise_to_strong_findings` | 通过 |
| `pnpm -C web exec vitest --run src/state/workspace-content-cache.test.ts src/state/workspace-ws-store.test.ts src/components/chat-workspace/entries/p1-entries.test.tsx src/pages/ChatWorkspacePage.test.tsx` | 通过 |
| `pnpm -C web build` | 通过，存在 Vite chunk size warning |
| `cargo check --locked` | 通过 |
| `cargo fmt --check` | 通过 |

## 真实 E2E 检查项

- Story：reviewer 只有可选建议时，页面进入 `human_confirm`，可点击 `确认使用当前版本`。
- Design：reviewer 有 `strong_recommend_fix` 时，页面进入 `review_decision`，显示返修按钮。
- Work Item：刷新已落盘 workspace 后，选中 reviewer timeline node 能加载完整 review 输出。
- 多轮 review 后，前端 content cache 不无限增长，超过预算会淘汰旧内容。

## 残余风险

- 旧落盘 review 没有 findings 时只能进入人工确认兼容路径。
- Provider 仍可能输出不合规 JSON，后端会降级为 `needs_human`。
