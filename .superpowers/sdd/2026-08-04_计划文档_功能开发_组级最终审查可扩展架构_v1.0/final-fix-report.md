# Final Review Fix Wave 报告

## 范围与提交

- `e9c80116 fix: 接入组级审查 parse 补救路径 (Critical 2)`
  - 接入严格组级 JSON 解析、一次受限 repair、repair fidelity 校验及 raw/repaired 审计引用。
  - repair 或二次严格解析失败时，持久化 `*_output_invalid` 报告而不是伪装成普通 Blocked 结论。
  - 恢复已持久化 reduction raw 时严格解析并 fail-closed。
  - 同时纳入此前未提交的 Critical 1/3、Important 1/5：stale 审计语义、transport retry/状态所有权隔离、`role_run_id` 审计字段，以及移除 orchestrator 内重复 `create_failure_gate` 副作用。
- `eb3f9279 test: 更新组级审查补救与审计回归断言`
  - 更新既有断言以反映严格解析、stale audit 与 `role_run_id` 契约。
  - malformed provider 输出覆盖「执行 repair 后失败，产出 output-invalid 报告」路径。
- `c02e150f fix: 固定组级审查 git facts 的提交边界 (Important 2)`
  - `final_diff` 与 `diff_stat` 显式使用 `git diff <base> <review_request.commit_sha>`，不再隐式读取工作树 HEAD。
  - 覆盖 HEAD 已前进但 review request 指向旧提交时，facts 只包含请求提交内容。
- `3652ed2f fix: 收口组级审查成功路径失败处理 (Important 4)`
  - 将 reduction 之后的 raw-ref 查询、review 查询、role-run refs/status 更新与 timeline 完成收进一个结果边界。
  - 任一步失败统一转入 `finalize_group_review_failure`，避免成功收尾中的裸 `?` 遗留 Running role run。
  - repair 后 reduction 的内部评审恢复从 repaired ref（最后一个引用）读取，保证严格解析的持久化输入一致。
- `34ffe874 fix: 消除组级流失败路径遗留死代码告警`
  - 标注仅由测试调用的遗留失败辅助方法，确保 `clippy -D warnings` 可通过。

## TDD 与覆盖证据

- 先运行 `cargo check --locked`，确认三个遗留 `parse_review_payload` 调用导致编译失败；接入后检查通过。
- 初次 `cargo test --locked --lib group_review` 暴露旧语义断言失败；更新为严格解析/repair/stale audit/role run 身份断言后全绿。
- `collect_group_git_facts_uses_review_request_commit_when_head_has_advanced`：覆盖 Important 2 的请求提交边界。
- `step13_14_malformed_json_output_attempts_repair_then_persists_invalid_raw_ref`：覆盖 Critical 2 的补救失败 fail-closed 行为。
- 既有 `group_final_review_*` 和 group-review 定向测试覆盖 runner 收口、stale、transport retry、raw refs 与失败门禁。

## 验证结果

- `cargo fmt --check`：通过。
- `cargo clippy --all-targets --all-features --locked -- -D warnings`：通过（在处理仅测试使用的辅助方法 dead-code 警告后）。
- `cargo check --locked`：通过。
- `cargo test --locked --lib group_review`：通过，117 passed。
- `cargo test --locked`：通过，312 passed、12 ignored；doc-tests 1 passed。
- `git diff --check`：通过。

## Park / 风险

- Important 3（stage-specific retry routing）和 Important 6（lease TTL/crash recovery）按 brief 仍为 defer，未扩大本 fix wave 范围。
- 工作树保留了前序 worker 创建的未跟踪 `.pi-subagents/` 目录；未修改、未暂存，也未纳入任何提交。
## Critical 3 聚焦修复

### 修复方法

- **错误分类**：新增 `CodingWorkspaceEngineError::ProviderProtocol` 与组级 `GroupReviewExecutionError::ProviderProtocol`。`ProviderEvent::ProtocolError` 保留原始 message 并进入该结构化变体；组级 retry 仅重试 `Transport`，协议错误立即转为对应 shard/reduction 的 `*_output_invalid` 失败报告与门禁。未解决 choice 保持既有 `ProviderStream` 文案以避免改变非组级调用方的公开错误契约，但组级 executor 按既有 `provider_choice_unresolved` 标记分流为 `ProviderProtocol`。
- **重试耗尽收口**：新增 `ShardTransportExhausted`、`ReductionTransportExhausted`。耗尽时分别写入 verdict=`Blocked`、findings=[]、raw refs=[] 的失败报告，`run_failure_code` 分别为 `shard_transport_exhausted` 与 `reduction_transport_exhausted`；统一 failure handler 建对应门禁，attempt/role run/timeline 收口为可重试的 `Blocked`。
- **门禁行为**：失败 reason 纳入 gate 优先级、失败 gate 识别与分阶段重试 action；reduction transport 耗尽提供 `RetryGroupReduction`，shard 耗尽使用 shard retry。未改 `render.rs`、未调用 `bind_unit_run_execution_context`、未补写 hash。

### TDD 与覆盖证据

- RED：先增加 `provider_protocol_error_does_not_retry`，实现前编译暴露 `ProviderProtocol` 分支未穷尽；补齐结构化错误分类及 retry match 后 GREEN。
- `step13_14_transport_error_shard_persists_failure_report`：覆盖三次 shard transport 后保存空 raw refs 的失败报告、`Blocked` verdict 与耗尽错误。
- `step13_14_transport_error_reduction_persists_failure_report`：覆盖有效 shards 后仅 reduction 重试三次、保存 reduction transport 失败报告。
- `group_final_review_transport_exhaustion_persists_shard_report_and_blocks_attempt`：真实 streaming adapter 覆盖 shard transport 耗尽 → gate + attempt Blocked。
- `group_final_review_reduction_transport_exhaustion_persists_report_and_blocks_attempt`：真实 streaming adapter 覆盖 reduction transport 耗尽 → gate + attempt Blocked。
- `group_final_review_provider_protocol_error_does_not_retry_and_is_output_invalid`：真实 `ProviderEvent::ProtocolError` 覆盖一次调用、不重试、直接 shard output-invalid 与 gate。
- 途中全量测试发现 `provider_choice_unresolved` 的既有 Display 契约；按最小范围将其保持为 `ProviderStream`，并补跑三个相关 it_product 回归测试后通过。

### 标准验证结果

- `cargo fmt --check`：通过。
- `cargo clippy --all-targets --all-features --locked -- -D warnings`：通过。
- `cargo check --locked`：通过。
- `cargo test --locked --lib group_review`：通过，122 passed。
- `cargo test --locked`：通过，1601 lib + 148 it_core + 43 it_interactive + 198 it_product + 312 it_web，12 ignored；doc-tests 1 passed。
- `git diff --check`：通过。
