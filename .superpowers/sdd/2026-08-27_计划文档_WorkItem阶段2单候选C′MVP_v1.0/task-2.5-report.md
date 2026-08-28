# Task 2.5 实施报告：publish freshness 与 immutable provenance

## 完成内容

- 新增 `verify_publish_freshness`：以当前 source bytes 的 lowercase SHA-256 校验 IR source hash、固定 compiler version、机械报告 hash/version 以及零 Error finding，任一不匹配 fail-closed。
- 新增唯一 durable owner `WorkItemPlanSourceStore`：提供 source revision、候选 IR、机械报告和 publication provenance 的 typed put/get API；canonical ref 固定为 `project/{project_id}/issue/{issue_id}/plan/{plan_id}/{object_kind}/{object_id}`。
- 固化 canonical-ref 失败优先级：MalformedRef → WrongKind → ScopeMismatch → DanglingRef；内容、identity、source hash、compiler version 漂移均返回稳定码。
- 每条 durable record 的 `content_hash` 由排除自身字段后的 canonical JSON bytes 计算 lowercase SHA-256；put 与 get 均复核。相同 ID 且完全相同内容幂等；跨进程文件锁阻止并发覆盖同 ID。
- `PlanCandidatePublicationProvenance` 落至 compiler 公开类型；publication provenance 会校验 source/IR/report 的 scope、关联 ID、source hash 与 compiler version。
- `WorkItemPlanRevision` 与 `InitialPlanPublicationArtifacts` 加入 legacy-safe optional provenance ref/hash；journal fingerprint 覆盖这些字段，并要求 plan revision 与 artifacts 的 ref 一致、ref/hash 同时存在或同时缺失。
- 所有既有 `WorkItemPlanRevision` literal 明确迁移为 `publication_provenance_ref: None`，保持 legacy 行为不变。

## 修改文件

- `src/product/work_item_plan_source_store.rs`（新增）
- `src/product/work_item_plan_compiler/freshness.rs`（新增）
- `src/product/work_item_plan_compiler/tests/publish_freshness.rs`（新增）
- `src/product/mod.rs`
- `src/product/models/work_item_revision.rs`
- `src/product/work_item_plan_compiler/mod.rs`
- `src/product/work_item_plan_compiler/tests.rs`
- `src/product/work_item_plan_compiler/types.rs`
- `src/product/work_item_revision_store/initial_publication.rs`
- `src/product/work_item_revision_store/tests/initial_publication.rs`
- `src/product/workspace_engine/plan_projection.rs`
- legacy-compatible `WorkItemPlanRevision` fixture/production literals：
  - `src/product/coding_workspace_engine/tests.rs`
  - `src/product/coding_workspace_engine/tests/plan_amendment.rs`
  - `src/product/coding_workspace_engine/tests/runtime_handoff_impact.rs`
  - `src/product/models/tests.rs`
  - `src/product/plan_repair/engine.rs`
  - `src/product/plan_repair/engine/topology.rs`
  - `src/product/plan_repair/tests/amendment.rs`
  - `src/product/work_item_revision_store/tests.rs`
  - `src/product/work_item_revision_store/tests/handoff_deletion.rs`
  - `src/product/workspace_engine/tests/part_21.rs`
  - `src/product/workspace_engine/tests/part_22.rs`
  - `src/product/workspace_engine/tests/part_23.rs`
  - `src/product/workspace_engine/tests/part_25.rs`
  - `src/product/workspace_engine/tests/part_26.rs`
  - `src/web/coding_ws_handler/tests/failed_review_recovery/support/schema_v2.rs`
  - `src/web/coding_ws_handler/tests/plan_repair/support.rs`
  - `src/web/test_controls/plan_repair/seed.rs`
  - `tests/it_product/product_coding_workspace_engine/part_12.rs`
  - `tests/it_web/web_coding_attempt_api/part_02.rs`
  - `tests/it_web/web_coding_ws_handler/fixtures_authoritative_group.rs`

## 新增或更新测试

- `work_item_plan_compiler::tests::publish_freshness`：3 项
  - source 编辑、compiler version 漂移、机械报告 hash/version/Error finding 均拒绝；
  - 四类 canonical-ref 失败码及优先级；
  - immutable record/provenance round-trip、同 ID source/hash/compiler version 漂移、磁盘 content hash 篡改。
- `work_item_revision_store::tests::initial_publication`：新增 provenance fingerprint 覆盖，当前 11 项。
- 既有 Rust/unit/integration fixture literal 迁移，确保 legacy `None` 序列化兼容。

## 验证记录

| 命令 | 结果 | 摘要 |
| --- | --- | --- |
| `cargo test --locked --lib work_item_plan_compiler::tests::publish_freshness -- --list` | 通过 | 已验证匹配 3 项（实现前基线为 0 项）。 |
| `cargo test --locked --lib work_item_plan_compiler::tests::publish_freshness` | 通过 | 3 passed。 |
| `cargo test --locked --lib work_item_revision_store::tests::initial_publication -- --list` | 通过 | 已验证匹配 11 项。 |
| `cargo test --locked --lib work_item_revision_store::tests::initial_publication` | 通过 | 11 passed。 |
| `cargo test --locked --lib work_item_runtime_reader` | 通过 | 当前无匹配单元测试，0 passed。 |
| `cargo check --locked` | 通过 | 无警告/错误。 |
| `cargo fmt --check` | 通过 | 格式一致。 |
| `cargo clippy --all-targets --all-features --locked -- -D warnings` | 通过 | 无 warning。 |
| `cargo test --locked --lib kimi_code_provider::client_services::terminal` | 通过 | kimi terminal flaky 家族单跑：12 passed。 |
| `cargo test --locked --test it_core large_file_guard::product_source_and_test_files_stay_under_line_limit` | 通过 | 首次全量发现 1201 行门禁后，最小删空行恢复至 1200 行。 |
| `cargo test --locked` | 通过 | 全量最终通过；2804 lib tests passed、2 ignored，其他 integration/doc tests 均通过。 |

## 自审结论与剩余风险

- 自审未发现 blocker：canonical ref grammar、失败优先级、content hash 排除自身、source/IR/report/provenance 关联和 journal fingerprint 均有实现与回归覆盖。
- 本任务严格遵守 controller 范围边界：未实现 Approval CAS、reservation CAS 或 engine 接线；这些仍归 3.4/5.1。因此尚未由运行时路径触发 source store/freshness/provenance 写入与恢复。
- coding runtime reader 未新增 markdown/compiler freshness gate，符合 brief 的明确边界。

## 暂存状态

提交前将只以精确路径暂存本报告列出的文件；提交后会另行核验无 staged files。
