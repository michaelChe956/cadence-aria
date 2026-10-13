# 修复报告:机械报告 id 改全量内容寻址(结构性撞历史)

时间:2026-10-13 · 分支:feat-b-0808-add-monorepo · 提交:`fix: 机械报告 id 改全量内容寻址(结构性撞历史)`

## 1. 问题(诊断:local://diag-report-identity-mismatch.md)

`report_id = report-{ir.source_revision_hash[..16]}` 只对 source 寻址,而 report body 是 (IR, 上下文, validator 代码版本) 的函数。F2b(dfbdd410)给所有 findings 强制 typed `kind` 后,daemon 用新二进制对同一 source(逐字节重放)重新 validate → 新 report(kind=Some)与旧存储记录(无 kind)同 id 不同 body → `put_immutable` 冲突闭包按锚字段全等落 else 分支 = `SOURCE_STORE_IDENTITY_MISMATCH`(work_item_plan_source_store.rs put_mechanical_report),plan 用户重试被确定性阻塞(session_0003)。

## 2. 修法(诊断推荐主案)

report_id 改为**完整 record 内容寻址**,对齐 provenance 全 sha256 惯例:

- `PlanCandidateMechanicalReportRecord::content_addressed_id()`:对全量记录体(序列化后剔除 `id` 与 `content_hash` 两字段)取 sha256,id = `report-{digest[..16]}`,长度对齐 `source-`/`ir-` 的 16-hex 惯例。
- 语义:同内容 → 同 id,重放走 put_immutable 整记录相等分支幂等 Ok;内容变(findings/kind/compiler_version 任一)→ 新 id 追加写入,零冲突;旧 id 格式存量记录(如 `report-a2a60836…`)自然不再被撞,不迁移。
- 哈希辅助泛化:`hash_without_content_hash` 委托新 `hash_record_without_fields(record, removed)`。

### 改动点(3 个生产代码文件 + 2 个测试文件)

| 文件 | 改动 |
|---|---|
| `src/product/work_item_plan_source_store.rs` | 新增 `content_addressed_id()`;哈希辅助泛化为 `hash_record_without_fields` |
| `src/product/workspace_engine/single_candidate.rs` | author 路径 report_id 构造段(:545)改用 `content_addressed_id()` |
| `src/product/workspace_engine/conversational_gate_parts/revision_turn.inc.rs` | 门内人工修订路径(:277)同改(第二生产路径,同 bug) |
| `src/product/work_item_plan_compiler/tests.rs` | 恢复被 2b801f5a 误删的 `mod publish_freshness;` 挂载(见 §5) |
| `src/product/work_item_plan_compiler/tests/publish_freshness.rs` | 新增 2 测试 + 3 helper(红→绿) |

### 兼容性核查(get/validate 路径)

- `get_mechanical_report`(:338)/`get_source_revision`(:203)/`get_plan_candidate_ir`(:271)的 `record.id != 路径 object_id` 检查:put 始终以 `record.id` 命名文件与构造 canonical_ref,恒等成立,新旧格式皆兼容。
- `validate_mechanical_report_record`:只校验 source/IR 链与 hash/compiler_version 相等,无 id 格式假设。
- `validate_id`(= `validate_relative_id`):仅要求非空、无路径分隔符;`report-{16hex}` 合法。
- `canonical_ref` 构造一致性:put 返回值与 get 解析共用同一 `record.id`,无第二构造点。

## 3. TDD 证据

红(修复前,主树):

```
error[E0599]: no method named `content_addressed_id` found for struct
`work_item_plan_source_store::PlanCandidateMechanicalReportRecord`
```

绿(修复后,/tmp/report-id-verify 隔离树 = feat-b HEAD a4c97dd6 + 仅本修复五文件,主树编译被 SG2 在途 contract_autorepair 面挡住):
```
$ cargo test --lib product::work_item_plan_compiler::tests::publish_freshness
running 6 tests
test ...::verify_publish_freshness_fails_closed_for_stale_source_compiler_and_report ... ok
test ...::source_store_put_rejects_malformed_scope_and_object_ids_with_stable_code ... ok
test ...::source_store_ref_error_precedence_and_immutable_hashes_are_stable ... ok
test ...::legacy_source_addressed_report_id_collides_on_validator_kind_evolution ... ok
test ...::content_addressed_report_id_appends_evolved_report_without_collision ... ok
test ...::source_store_round_trips_typed_immutable_records_and_provenance ... ok
test result: ok. 6 passed; 0 failed
```


测试(新增,`publish_freshness.rs`):

1. `legacy_source_addressed_report_id_collides_on_validator_kind_evolution` —— 回归锚:旧 id 方案下,同 source、findings 仅 kind 不同(None vs typed)→ 第二次 put 报 `SOURCE_STORE_IDENTITY_MISMATCH`(锁定撞车机理,证明内容寻址是免撞必要条件)。
2. `content_addressed_report_id_appends_evolved_report_without_collision` —— 主案验收:同 source+不同 kind 两 record 内容寻址 id 不同、各自落盘成功;存量旧 id 记录不被确定性撞;同 record 重放幂等 Ok 且 ref 稳定;get 按各自 canonical ref 取回正确;同内容重放派生同 id(确定性)。

## 4. 回归

```
cargo test --lib product::work_item_plan_compiler::tests::publish_freshness
  → 6 passed   # source_store 全族 + 新增 2 测试
cargo test --lib product::workspace_engine::tests::single_candidate
  → 70 passed  # author 生产路径族(含 recovery/prompt/phase_machine)
cargo test --lib product::work_item_split_validator::tests::sg_f2
  → 5 passed   # F 线报告族(sg_f2b typed findings)
cargo test --lib product::workspace_engine::tests::conversational_gate
  → 109 passed # revision_turn 门内人工修订消费面
cargo test --lib product::workspace_engine::compile
  → 8 passed   # compile/publish 消费面
合计 198 passed / 0 failed(/tmp/report-id-verify 隔离树 = feat-b HEAD + 仅本修复文件)
```

范围:source_store 全族(publish_freshness,含恢复挂载后复活的 4 个既有测试)、single_candidate 测试族、sg_f2*(split_validator F 线)、conversational_gate 族(revision_turn 消费面)、compile 族。

## 5. 附带发现与修复:publish_freshness 测试族失编

`2b801f5a`(C1-T1)提交把 `tests.rs` 中 `mod publish_freshness;` 误覆盖为 `mod preflight_options;`(应为插入),导致 source_store 全族 4 个测试(round_trips / malformed ids / error precedence / immutable hashes)自该提交起**静默失编**。本修复恢复挂载,该族测试随绿测一并复活并通过。

## 6. 影响面

- 存量数据:旧 `report-{source_hash[..16]}` 记录不迁移、仍可 get;新 payload 使用内容摘要生成 id(摘要输入与旧方案的 source 摘要不同,新旧 id 同为 `report-{16hex}` 形、并非命名空间隔离)。16-hex 为 64-bit 截断、非绝对唯一:残余理论碰撞退化为 64-bit 随机碰撞,仍由 put_immutable 不可变守卫兜底(同内容幂等、不同内容显式 IdentityMismatch,绝不静默覆写),无迁移/清理需求。
- session_0003 解卡:新代码重放同 source 时新 report 获新 id 正常落盘,IdentityMismatch 不再复现。
- provenance 引用:`mechanical_report_ref` 由 put 返回的 canonical ref 透传,新 id 自动进入新 provenance;旧 provenance 引用旧 id 记录,照常可读。
