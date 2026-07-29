# Tasks: relax-terminal-handoff-contract-refs

## 1. 失败测试先行（TDD RED）

- [x] 1.1 更新 `src/product/work_item_split_engine/tests/prompt_contract.rs`：将断言旧"required_fields、provided_contract_refs、reviewer_check_refs 均非空"文本的用例改为断言新规则——`provided_contract_refs` 可为空、仅列被下游消费的 ref、末端必须为空；确认测试 RED。
- [x] 1.2 检索并更新 `src/product/work_item_split_engine/tests/` 内其他引用旧非空文本的断言/夹具（如 part_01.rs），保持与 1.1 一致。

## 2. Prompt 修改（TDD GREEN）

- [x] 2.1 修改 `src/product/work_item_split_engine/prompts.rs:653` schema 记号行：`provided_contract_refs` 改为可空表述。
- [x] 2.2 修改 `src/product/work_item_split_engine/prompts.rs:660` 约束行：按 design D2 写入条件非空 + 末端必空规则；确认 1.1/1.2 测试转 GREEN。

## 3. 验证

- [x] 3.1 `cargo fmt --check` 与 `cargo clippy --all-targets --all-features --locked -- -D warnings` 通过。
- [x] 3.2 `cargo test --locked --lib work_item_split_engine` 全绿（含质量预算 12,000 断言）。
- [x] 3.3 `cargo test --locked --lib work_item_contract` 全绿（校验器语义未变的回归证据）。
- [x] 3.4 `cargo test --locked --lib workspace_engine` 全绿。
- [x] 3.5 `openspec validate relax-terminal-handoff-contract-refs --strict` 通过。

## 4. 真实验证与交付

- [ ] 4.1 提醒操作者：按项目规则需授权执行 Case A / Case B 各 10 次真实 Claude Code 验证后方可交付；未授权不勾选。
- [ ] 4.2 告知操作者存量恢复路径：在界面对 issue_0001 的 Plan 重新生成末端 Draft 并重跑 Final Compile，然后重试人工确认。
