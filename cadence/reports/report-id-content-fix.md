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

## 7. 复审 P2 修复:评估 CAS 接受等价内容的 report ref 轮换

提交:`fix: 评估CAS接受等价内容的report ref轮换`

**回归机理**(Main 复审):持有旧方案 durable `mechanical_report_ref` 的会话重启重放 author(Evaluate 重入是既有支撑路径)时,同内容重放在新方案下轮换 id → `compare_and_save_single_candidate_evaluation`(lifecycle_store/workspace_single_candidate.rs:621-628)对 ref 不等一律 `Conflict` —— 旧行为是整记录相等分支幂等 Ok,新方案把旧会话重放从幂等变确定性失败。

**修法**:评估 CAS 在 ref 不等时,按 stored ref 与 incoming ref 各自经完整校验链(`get_mechanical_report`)加载 record,比对剔除 `id`/`content_hash` 后的全量内容摘要(`content_addressed_id` 相等 ⇔ 等价内容,与 id 格式无关):等价 → 接受轮换(更新 session ref,重入幂等语义与内容寻址化前一致);内容真变或任一 ref 悬空/校验失败(无法证明等价)→ 维持 `Conflict` fail-closed。

**TDD**:
- 红(修复前,/tmp/p2-verify 隔离树 = da2e35b4 + 仅新测试):
  `panicked: equivalent-content report ref rotation must stay idempotent: Conflict { kind: "single_candidate_evaluation", id: "workspace_session_0001" }`
- 绿(修复后):新测试 `evaluation_cas_accepts_report_ref_rotation_for_equivalent_content` 过(旧格式 ref 会话 + 同内容轮换 → Ok 且 ref 更新;轮换后同 ref 重放幂等;内容真变 typed kind → 仍 Conflict);CAS 族 9/9。

**回归**:
```
cargo test --lib product::lifecycle_store
  → 105 passed # 全族(含 CAS/轮换/预约 + 新测试)
cargo test --lib product::work_item_plan_compiler::tests::publish_freshness → 6 passed
cargo test --lib product::workspace_engine::tests::single_candidate       → 70 passed
cargo test --lib product::work_item_split_validator::tests::sg_f2         → 5 passed
cargo test --lib product::workspace_engine::tests::conversational_gate   → 109 passed
cargo test --lib product::workspace_engine::compile                       → 8 passed
合计 303 passed / 0 failed(/tmp/p2-verify 隔离树 = da2e35b4 + 本修复两文件)
```

范围:lifecycle_store 全族(含 CAS/轮换/预约)+ 原 198 面(publish_freshness/single_candidate/sg_f2/conversational_gate/compile)。

## 8. 格式轮:P2 修复 rustfmt 违规

提交:`fix: P2修复轮格式化`(仅 lifecycle_store 两文件,纯格式零行为变化)。

- 违规 3 处(复审指出):`workspace_single_candidate.rs` import 合并行 102 字符(该合并行系格式轮开工前树内既存);`workspace_single_candidate_tests.rs` 测试内 import 组与超长 `assert_eq!`。
- 修复:`rustfmt --edition 2024` 对两文件,`--check` 复核零 diff。
- 验证:CAS 族 9/9 仍绿(/tmp/fmt-verify 隔离树 = 2e2b1bed + 仅格式化两文件;主树当时被 SG3a.0 在途 `structure_report` 补齐面挡,45 错均非本轮引入)。
