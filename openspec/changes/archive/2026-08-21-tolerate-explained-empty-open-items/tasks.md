## 1. 校验器容错（TDD）

- [x] 1.1 先写失败单测：`validate_workspace_artifact_constraints` 对「待确认项」正文为「无待确认项。Issue 已明确需求……不构成本 Story 的未决问题。」的完整 Story artifact 返回 `passed=true`（无「待确认项未通过 AskUserQuestion 交互解决」禁止内容）；运行 `cargo test -p aria --lib artifact_constraints` 确认新用例失败、既有用例不回归
- [x] 1.2 先写负例与边界单测：「无论使用哪种 runner 都需要确认」「单元测试运行器选型仍待确认。」仍判未解决；「暂无。」「none.」前缀判已解决；运行同上命令确认负例通过方向与 1.1 实现联动验证
- [x] 1.3 实现 `open_item_leading_empty_marker` 前缀判定并在 `open_item_section_is_resolved` 开头短路（含单字「无」的边界约束，见 design.md 风险项）；`cargo test -p aria --lib artifact_constraints` 全绿
- [x] 1.4 用 timeline_node_002 的真实失败样本（checkpoint cp_001 前一版待确认项文本）做回归断言：校验通过；`cargo test -p aria --lib workspace_engine` 通过

## 2. Prompt 提示收紧

- [x] 2.1 修改 Story `open_item_policy_hint`：追加「若无开放问题，正文只写『无待确认项』，不得附加解释；解释性内容写入其他章节」；新增/更新断言提示文案包含该指示的用例并运行通过

## 3. 整体验证

- [x] 3.1 运行 `cargo fmt`、`cargo clippy --workspace --all-targets`、`cargo test --workspace`，全部通过后汇报新鲜证据
