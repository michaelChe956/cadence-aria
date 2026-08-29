# findingfix 完成报告

## 变更摘要

- 在单候选 WorkItemPlan review 的 Initial 与 Verification invocation scope 教学中加入 finding 字段白名单，明确禁止 `finding_id`、`code`、`work_item_ids` 及其他字段。
- parser 的 `RawReviewFinding` 白名单允许 provider 偶发输出 `finding_id`、`code`、`work_item_ids`，并通过 `serde(skip_deserializing)` 忽略这些字段，不影响服务端 finding 语义与路由。
- 保留 parser 对其他未知字段的显式拒绝，未扩大协议范围。

## 测试与验证

- TDD 回归测试：`review_envelope_ignores_provider_finding_identity_fields` 覆盖三字段存在时解析成功且 finding 内容仍正确。
- Prompt 断言：Initial 与 Verification 单候选 scope 测试均断言完整白名单教学句。
- `cargo fmt --check`：通过。
- `cargo clippy --all-targets --all-features --locked -- -D warnings`：通过。
- `cargo check --locked`：通过。
- `cargo test --locked --lib review_envelope_`：5 个测试通过。
- `cargo test --locked --lib review_prompt_teaches_finding_field_whitelist`：2 个测试通过（后续测试移入既有单候选测试模块，定向 prompt 回归仍通过）。
- `cargo test --locked --lib single_candidate_initial_prompt_is_derived_from_server_scope`：通过。
- `cargo test --locked --lib single_candidate_verification_prompt_replays_only_original_fingerprints`：通过。
- `cargo test --locked`：400 个单元测试通过、12 个忽略；集成测试 1 + 148 + 44 通过；文档测试 2 通过。

## 文件

- `src/product/workspace_engine/parsers.rs`
- `src/product/workspace_engine/prompts/review.rs`
- `src/product/workspace_engine/tests/single_candidate_prompt.rs`
- 本报告文件

## 残余风险

无已知残余风险。手工 provider 方差验证仍应按项目规则由操作者授权执行 Case A、Case B 各 10 次真实 Claude Code 验证；本任务未自动调用 provider。
