# C2-T5 独立复审（k3）— WP5 增殖审计 / 恢复矩阵

- 被审交付：C2-T5（WP5，REQ-MTG-05，multi-repo-group-coding）
- 复审轮次：
  - round 1：`866bdea0`（增殖审计 + 恢复矩阵主体）→ 出 1×P2（split-audit 无删除路径）；该轮报告此前未落盘，本轮补记于本文件「二」。
  - round 2（本次）：`ac961c2d`（fix round 1，5 文件 +107）范围化复审 —— 结论 **PASS**。

---

## 一、round 2 结论（ac961c2d）

**PASS**：`delete_split_audit` 按 attempt_id 寻址、接线于 `delete_attempt` 的 journal 删除之后，覆盖全部生产删除漏斗；断链面（写入/接线/委托/寻址）无缺陷，红绿断言非空转。未出新增 finding。

### 1.1 接线位置与生命周期（对照范围项 1）

| 检查点 | 证据 | 判定 |
|---|---|---|
| 删除落点 | `coding_attempt_store/attempt.rs:386-390`：`delete_group_initialization_for_attempt` → `delete_split_audit` → 删 attempt json/目录 | PASS（与 journal 同生命周期、同失败语义：任一步 Err 都使 attempt 保留，审计-在职一致性不破） |
| 生产漏斗覆盖 | `web/handlers/support.rs:558`（`finalize_coding_attempt_deletion`）、`web/handlers/coding.rs:335`（bind 失败回滚）、`web/handlers/lifecycle/deletion.rs:126`（work item 删除）三处全部经 `delete_attempt`；`coding_workspace_engine/handoffs.rs:309 handle_delete_attempt` 只做 abort/释放锁/清 worktree，不删数据 | PASS（无绕过 store 的删除面） |
| 委托覆盖 | `attempt.rs:396-405`：`delete_attempts_for_work_item` 逐条委托 `delete_attempt` | PASS |
| 唯一的 journal 删除者 | `delete_group_initialization_for_attempt` 全仓仅被 `delete_attempt` 调用（grep 无第二调用方） | PASS（不存在「journal 已删、attempt 与审计仍在」的新裂缝） |

### 1.2 无误删面（对照范围项 2）

- 落点派生：`split_audit.rs:56-70` `split_audit_path` = `issue_lifecycle_root(project,issue)/split-audit/{attempt_id}.json`，三段 id 均过 `validate_relative_id`；写入侧 `record_split_audit:96-99` 硬校验 `id == attempt_id`，故路径键与记录身份一一对应。
- attempt id 形如 `coding_attempt_{uuid_v4_simple}`（`attempt.rs:231-233`），无复用/无跨 target 撞名 ⇒ 删 A 不影响同 plan 的 B（sibling 审计零波及）。
- 删除用 `remove_file_if_exists`（`utils.rs:312-321`，NotFound=Ok）⇒ 无审计的 attempt（含 fixture 内被删的种子单 target attempt，`tests/advance_split_targets.rs:108`）零副作用。

### 1.3 红绿验证（对照范围项 3）

测试 `workspace_engine/tests/advance_split_recovery_matrix.rs:291-358`（`split_audit_is_deleted_with_attempt_and_retry_stays_unique_per_target`）：

- 红面非空转：删两个 target-attempt 后 `split_audits(...).is_empty()`（:322）——撤回 `delete_split_audit` 时该断言必红（`get_split_audits_for_plan` 仍返回 2 条残留）；重试后 `audits_after.len() == 2`（:342）也会红（残留 2 + 新 2 = 4）。两条都是「残留必红」的有效锁。
- 前置可达性：审计写在 `advance_split.rs:207`（早于外层集绑定 `:252`），attempt 持久化 failpoint `AttemptPersisted`（:317）早于本测试所用 `WorktreeBound`（:424）⇒ 崩溃后审计 2 条、attempt 与 journal 均 durable，`delete_attempt` 可寻址。
- 零残留 + 重试唯一：`live_ids.len()==2`、`audits_after.len()==2`、target 去重后 2、且每条 `audit.attempt_id ∈ live_ids` ⇒ 审计集不复现已删身份。
- 失配不耦合断言的合理性：重试仍以同一 `record` 走 `load_or_prepare_advance_initialization_for_attempts`，持久 journal 的 `target_attempt_ids` 为旧集 ⇒ 既有语义 `IdentityMismatch` fail-closed（`advance_store.rs:507-516`）。测试用 `let _ = handle_advance(...)` 忽略该 outcome、只断言 durable 审计态，是**合理**的：被测契约在 store 面（per-(plan,target) 唯一），把它绑到外层集绑定的成功/失败上反而会越界耦合 T2 语义。

### 1.4 观察项（非阻断，不计 finding）

- 测试注释写「全新 attempt UUID」，但断言未直接排除「journal 未被删而重试复用同 id」这一路径（该路径下同 id 幂等重写审计，四条断言仍会通过）。删 journal 属既有面且另有覆盖，风险极低；如需更紧可用 `assert!(!live_ids.contains(&old_id))` 加锁。
- `delete_split_audit` 为 `pub` 但唯一调用方在 store 内（`pub(super)`/`pub(crate)` 更贴切）；纯风格项。

---

## 二、补记 round 1（866bdea0）的 P2 与前轮取证

**P2（前轮，已在 ac961c2d 修复）**：增殖审计只有写路径，无删除路径——`delete_attempt` 只清 attempt + per-target journal，`split-audit/{attempt_id}.json` 永久残留。触发面：中断后删除 target-attempt（UI `handle_delete_attempt`/work item 删除）再以同 command_id 重试，重试为新 UUID 落新审计 ⇒ 同 `(plan,target)` 双审计，破坏 REQ-MTG-05「per-(plan,target) 唯一、MUST NOT 依赖取最早」的检索不变式，并污染二期跨 attempt 依赖门的证据面。

- 修法核对：controller 裁定「同生命周期删除」已按原样落地（写入侧 id=attempt_id 已钉死 ⇒ 无需配套 sibling 清理逻辑，无越界改动）。
- 报告一致性：`cadence/reports/stage4-c2-t5-report.md:96-101`（第七节）与 `cadence/reports/multi-repo-group-coding/wp5-recovery-matrix.md`（新增对照行）描述与代码/测试相符。

---

## 三、复审方法

- 只读面：`git show ac961c2d`（5 文件 full diff）逐文件通读；对 `delete_attempt` / `delete_attempts_for_work_item` / `delete_group_initialization_for_attempt` / `split_audit_path` / `remove_file_if_exists` / `list_json_records` / `advance_store` 集绑定 / `advance_split.rs` 相位顺序做调用面与消费面核对。
- 未执行构建/测试（工作树含并行 agent 的未提交改动：`workspace_engine/advance_split.rs` 未跟踪 + `mod.rs`/`advance.rs` 在改），故红绿判定基于断言语义与相位顺序的静态推演，未由执行输出复核。
