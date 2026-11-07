# 失败簇收尾：`web::workspace_ws_handler::tests::plan_repair_activation`（2 条 → 绿）

- worktree：`.worktrees/bugfix-b-11-02`
- 接手自：`FixWebCluster`（其报告 §12.4 留的定位线索）
- 处置口径：**只改夹具**（断言零改动、零实现改动、零授权需求）
- 结论：**2 passed / 0 failed**（`cargo test --locked --lib plan_repair_activation`）

---

## 1. 诊断通道怎么造的

前一个 worker 的两处盲区（`cadence/notes/2026-11-03_失败簇收敛_workspace_ws_handler_v1.0.md`
§10.2）让它只能推测、无法看到真实失败点。本轮补了三条通道：

| # | 通道 | 造的��式 | 解决什么盲区 |
|---|---|---|---|
| ① | **双 socket 全帧读取** | 在测试文件里临时加 `diagnostic_plan_repair_activation_channel`：用 `tokio::select!` 同时读 coding WS 与 child WS 的**每一个**文本帧并原样 `println!` | 原 `receive_type`（`plan_repair_activation.rs:55-61`）**静默丢弃**非目标 type 的帧 —— `coding_protocol_error` 会被吃掉、只剩「超时」；child socket 在 confirm 之后的帧**根本没人读** |
| ② | **租约/工作树事实直读** | 调 `store.classify_worktree_lease_for_attempt(&attempt)` 打印 `disposition` / `lease_id` / `evidence`；读 `get_repo_shared_worktree` / `get_issue_shared_worktree` 打印落盘记录 | 前一个 worker 的第①站（「evidence 变了没」）—— 直接看到租约三态判定原文 |
| ③ | **durable 侧状态直读** | 打印 attempt `status` / `target_snapshot.is_some()` / `current_work_item_id`、delivery 状态、amendment journal 相位/错误、chat-entries（手动恢复诊断）、`tracing` 捕获 | runner 死亡路径的 `tracing::warn!` 与 durable 死因证据 |

**通道纪律**：诊断段整段带 `// ==== TEMP DIAGNOSTIC（定位后删除） ====` 标记，
**定位完成后整段删除**（含诊断用例本身与全部 `println!("DIAG ...")`）。
收口实测：`grep -rn 'TEMP DIAGNOSTIC' src/ --include=*.rs` = **0 命中**（全树清零）。

---

## 2. 真实失败原因原文

### 2.1 先纠正一个误判：症状**不是** websocket 超时

接手后第一次窄跑（`flock /tmp/aria-cargo.lock` 排队后）实测：

```
running 3 tests
test web::workspace_ws_handler::tests::plan_repair_activation::diagnostic_plan_repair_activation_channel ...
thread '...' panicked at src/web/workspace_ws_handler/tests/plan_repair_activation.rs:398:10:
called `Option::unwrap()` on a `None` value
FAILED
test web::workspace_ws_handler::tests::plan_repair_activation::repeated_confirmation_recovers_delivery_mark_failure_without_duplicate_application ...
thread '...' panicked at src/web/workspace_ws_handler/tests/plan_repair_activation.rs:157:10:
called `Option::unwrap()` on a `None` value
FAILED
test web::workspace_ws_handler::tests::plan_repair_activation::zero_socket_plan_amendment_activation_resumes_attempt_with_unsent_delivery ...
thread '...' panicked at src/web/workspace_ws_handler/tests/plan_repair_activation.rs:284:10:
called `Option::unwrap()` on a `None` value
FAILED

test result: FAILED. 0 passed; 3 failed; 0 ignored; 0 measured; 5968 filtered out
```

⇒ **两条真用例的失败点都在第 157 / 284 行**，即
`store.get_attempt_for_work_item_group("project_0001","issue_plan_0001","work_item_plan_0001", None).unwrap().unwrap()`
的**第二层 `unwrap()`** ⇒ 根本没走到 websocket。
⇒ 「websocket 超时」是**前一个 worker 加完第三层改法之前**观测到的旧症状；
第三层落盘后症状前移成了「查找落空」。

### 2.2 根因：`attempt_target_bucket` 的桶身份，与用例正文的查找口径冲突

`src/product/coding_attempt_store/group.rs:77-101`：

```rust
pub fn get_attempt_for_work_item_group(
    &self, project_id: &str, issue_id: &str, plan_id: &str,
    target: Option<LogicalRepositoryId>,
) -> Result<Option<CodingExecutionAttempt>, ProductStoreError> {
    ...
    .filter(|attempt| {
        attempt.work_item_group_id.as_deref() == Some(plan_id)
            && Self::attempt_target_bucket(attempt) == target   // ← 桶过滤
    })
```

`src/product/coding_attempt_store/group.rs:132-140`：

```rust
/// attempt 的 target 桶键：快照在场取 `logical_repository_id`，否则 `None`
pub(crate) fn attempt_target_bucket(attempt: &CodingExecutionAttempt)
    -> Option<LogicalRepositoryId>
{
    attempt.target_snapshot.as_ref().map(|s| s.logical_repository_id)
}
```

⇒ **第三层改法**（`seed_lc_authority_for_plan_repair` 里
`attempt.target_snapshot = Some(snapshot_for(...))` + `write_coding_attempt_for_test`）
把该 attempt 的**检索桶身份**从 `None` 改成了 `Some(logical_repository_id)`；
⇒ 而用例正文写死第四个参数 `None` ⇒ 过滤不中 ⇒ `Ok(None)` ⇒ 第二层 `unwrap()` panic。

**根因一句话**：不是「attempt 没落地」（它是落盘了的），而是
**「attempt 的 target 桶被第三层改法改了，而取它的查找口径仍按旧桶写死」**。

### 2.3 桶改对之后，租约链真的通了（诊断通道实测原文）

修好查找口径后重跑诊断通道，租约与投递事实原文：

```
DIAG attempt.status = AwaitingPlanAmendment
DIAG attempt.target_snapshot.is_some = true
DIAG attempt.current_work_item_id = Some("wi_registration")
DIAG lease.disposition = ActiveWait
      / lease.lease_id = "coding_attempt_2476ac3b0a4b469a91de929fcf1c44b0"
      / evidence = ["worktree lock owner: coding_attempt_2476ac3b0a4b469a91de929fcf1c44b0",
                    "active work item: Some(\"wi_registration\")",
                    "owner attempt coding_attempt_2476ac3b0a4b469a91de929fcf1c44b0 is active (AwaitingPlanAmendment)"]
DIAG repo worktree record = Some(IssueSharedWorktree { repository_id: "0deb4d50-…", worktree_path: "/tmp/.tmphWZxRH/worktree",
      status: Running, current_active_work_item_id: Some("wi_registration"),
      current_lock_owner_id: Some("coding_attempt_2476ac3b0a4b469a91de929fcf1c44b0"), … })
DIAG issue shared worktree record = None
DIAG coding frame = {"type":"coding_gate_required","gate":{"gate_id":"coding_stage_gate_0001",…}}
DIAG coding frame = {"type":"coding_session_state",…}
DIAG coding frame = {"type":"plan_amendment_updated","event_id":"coding_plan_amendment_updated_coding_attempt_2476…_plan_amendment_5a89…",…}
DIAG after attempt.status = Running
DIAG runner_count = 1
DIAG delivery = Ok((Delivered, "coding_plan_amendment_updated_coding_attempt_2476…_plan_amendment_5a89…"))
DIAG journals = Ok([("plan_amendment_5a89…", Completed, None)])
DIAG lease after = ["worktree lock owner: coding_attempt_2476…", "active work item: Some(\"wi_registration\")",
                     "owner attempt coding_attempt_2476… is active (Running)"]
DIAG tracing = ""
```

**读法（这一段回答了前一个 worker 的三站）**：

1. **第三层确实生效**：快照落盘了（`target_snapshot.is_some = true`），
   仓维 worktree 记录在（`repo worktree record = Some(...)`），
   锁绑到了本 attempt（`current_lock_owner_id = Some("coding_attempt_2476…")`）。
2. **租约三态不再是 `LeaseUnknown`**：`disposition = ActiveWait`，
   且 `lease_id` **就是本 attempt 自己** ⇒ 自持活跃租约 ⇒
   `lease_exclusion_option` 不拦截（自持不构成冲突事实）。
   ⇒ 前一个 worker 担心的 `target_snapshot_missing_for_logical` **已被第三层消除**。
3. **仓维锁的 `work_item_id` 是对的**：`current_active_work_item_id = Some("wi_registration")`
   与 `attempt.current_work_item_id = Some("wi_registration")` 一致
   （基座 seed.rs:274 建 attempt 时写死 `"wi_registration"`）。
4. **投递面通了**：`plan_amendment_updated` 帧真实到达 coding WS，
   delivery = `Delivered`，journal = `Completed`，attempt → `Running`，runner_count = 1。

⇒ **第三层方向是对的，只是缺配套的查找键修正**（前一个 worker 的「链子没打通」结论下早了：
它只看到症状前移，没看到症状前移后的真实下游其实是通的）。

---

## 3. 改法（只改夹具 + 一段经授权的 doc comment，零断言 / 零行为改动）

### 3.1 改前 → 改后

**改前**（3 处，写死 `None` 桶的查找）：

```rust
let attempt = store
    .get_attempt_for_work_item_group(
        "project_0001", "issue_plan_0001", "work_item_plan_0001",
        None,                       // ← 与「attempt 已带快照」冲突
    )
    .unwrap()
    .unwrap();                      // ← 第二层 unwrap 在 Ok(None) 上 panic
```

**改后**（新增一个共用 helper，3 处调用点统一走它）：

```rust
/// 取基座 plan 级 group attempt。
///
/// 用 `list_attempts_for_work_item_group`（`group.rs:106`，public）而**不是**
/// `get_attempt_for_work_item_group(..., target)`：后者按
/// `attempt_target_bucket(attempt) == target` 过滤，而
/// `attempt_target_bucket`（`group.rs:132-140`）在**快照在场时取
/// `logical_repository_id`**——本夹具给 attempt 补上与权威一致的冻结快照后
/// 桶从 `None` 变 `Some(logical_repository_id)`，写死 `None` 的查找口径
/// 就会落空（第二层 `unwrap()` panic）。`list_attempts_for_work_item_group`
/// 只按 `work_item_group_id` 过滤、不看桶 ⇒ 与「该 attempt 有没有快照」解耦，
/// 两种形态都命中同一条基座 attempt（基座只创建这一条 group attempt）。
fn plan_repair_attempt(store: &CodingAttemptStore) -> CodingExecutionAttempt {
    store
        .list_attempts_for_work_item_group("project_0001", "issue_plan_0001", "work_item_plan_0001")
        .expect("plan repair attempt list")
        .into_iter()
        .next()
        .expect("plan repair attempt must exist")
}
```

调用点（3 处，全部在 `seed_lc_authority_for_plan_repair` 之后 / 之内）：

| 位置 | 上下文 | 改前 | 改后 |
|---|---|---|---|
| `seed_lc_authority_for_plan_repair`（读 attempt 以补快照） | 第三层落盘**前** | `get_attempt_for_work_item_group(..., None).unwrap().unwrap()` | `plan_repair_attempt(&store)` |
| `repeated_confirmation_…`（用例正文取 attempt） | 第三层落盘**后** | 同上（`:157` panic） | `plan_repair_attempt(&store)` |
| `zero_socket_…`（用例正文取 attempt） | 第三层落盘**后** | 同上（`:284` panic） | `plan_repair_attempt(&store)` |

**为什么选 `list_attempts_for_work_item_group` 而不是「从 attempt 派生桶再传 `Some(...)`」**：

- 它**不看桶** ⇒ 与「该 attempt 有没有快照」彻底解耦 ⇒
  第三层**在/不在**、查找口径**变/不变**都不会再互相打断（不脆）。
- 它是 `pub`（`group.rs:106`）⇒ 不需要任何实现侧改动、无需授权。
- 基座只创建**一条** group attempt（`test_controls/plan_repair/seed.rs:274`
  `create_group_attempt` 是唯一的创建点）⇒ 取首个无歧义
  （该函数按 `attempt_no` 再 `id` 排序，取首个即最早那条）。

### 3.2 第三层本身**保留**（不回退）

第三层（补与权威一致的冻结快照 + 仓维 worktree 记录 + 锁绑到本 attempt）
**方向正确且已被诊断实测证实有效**（§2.3：租约从 `LeaseUnknown` 变 `ActiveWait`
自持 ⇒ 不拦截；投递面真实送达 `Delivered`）⇒ 不回退，只补配套的查找键。

### 3.3 附带补的注释（防复发）

**① 夹具侧**（`seed_lc_authority_for_plan_repair` 的文档注释新增一段）：显式记下
「③ 的**副作用**：快照落盘后 target 桶从 `None` 变 `Some(logical_repository_id)`
⇒ 此后取这条 attempt 必须用不看桶的 `list_attempts_for_work_item_group`」，
避免下一个维护者再写回 `None` 桶的查找。

**② 实现侧文档警示**（**Main 明确授权，零行为变化、零断言**）：
`src/product/coding_attempt_store/group.rs::get_attempt_for_work_item_group`
的 doc comment 新增一段：

```rust
/// ⚠️ **`target` 是【桶键】，不是「按 plan 取唯一 attempt」的便捷参数**：
///    桶键 = `attempt_target_bucket(attempt)` = 有 `target_snapshot` ⇒
///    `Some(logical_repository_id)`，无 ⇒ `None`。
///    ⇒ **给 attempt 补 `target_snapshot` 会改变它的桶**，
///      此后用旧桶键（`None`）检索会落空（返回 `Ok(None)`）。
///    ⇒ 取「按 plan 取 group attempt」请用 `list_attempts_for_work_item_group`
///      （只看 `work_item_group_id`，不看桶）。
```

⇒ **只改 doc comment，零行为变化**；
`cargo check --locked --lib` = **0 error**（仅既有 warning）；
改后复跑 `plan_repair_activation` = **2 passed / 0 failed**（行为中性已证）。

---

## 4. 测试红 → 绿

```
$ flock /tmp/aria-cargo.lock -c 'cargo test --locked --lib plan_repair_activation -- --nocapture 2>&1 | tail -30'

running 2 tests
test web::workspace_ws_handler::tests::plan_repair_activation::zero_socket_plan_amendment_activation_resumes_attempt_with_unsent_delivery ... ok
test web::workspace_ws_handler::tests::plan_repair_activation::repeated_confirmation_recovers_delivery_mark_failure_without_duplicate_application ... ok

test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 5968 filtered out; finished in 1.26s
```

**改前**：`0 passed; 3 failed`（2 条真用例 + 1 条诊断用例），panic 原文见 §2.1。
**改后**：**2 passed; 0 failed**（诊断用例随整段删除 ⇒ 测试数从 3 回到 2）。

回归（`web::workspace_ws_handler` 整簇）见 §6。

---

## 5. 硬约束核对

| 约束 | 实测 |
|---|---|
| 只改夹具 | ✅ 行为改动**只**在 `src/web/workspace_ws_handler/tests/plan_repair_activation.rs` 一个文件；实现侧**仅一段 doc comment**（Main 明确授权，`group.rs::get_attempt_for_work_item_group`），**零行为变化**、零断言。实现目录（`src/product/**` 其余、`src/web/workspace_context/**`、`src/web/workspace_ws_handler/decisions.rs`、`plan_repair_activation.rs`（实现侧））**零改动** |
| 不改断言凑绿 | ✅ 两条用例的**全部 `assert!` 逐字未动**（只换了取 attempt 的那一行的**查找 API**） |
| 不改实现 | ✅ **零行为改动**；唯一的实现侧改动是 Main 明确授权的 doc comment（`cargo check --lib` 0 error，复跑 2 passed 行为中性已证） |
| 不恢复 `RepositoryRouting::Legacy` 变体 | ✅ 未触碰（该变体在 `repository_routing_types.inc.rs` 仍是「已随去单仓入口整体移除」状态） |
| 不自造 reason_code | ✅ 未新增任何 reason_code / 稳定码 |
| 不改 spec / Plan / 技术方案 | ✅ 未触碰任何 openspec / Plan / 技术方案文件 |
| 不删断言 | ✅ 零断言删除 |
| 不 `git add` / `commit` / `stash` / `checkout` | ✅ 全程只用 `edit` / `write` + `rm` 临时文件；`git status` 显示该文件仍是 `M`（未 staged） |
| 行数 < 1200 | ✅ **372 行**（改前 367，净 +5：新增 `plan_repair_attempt` helper + 文档注释；诊断段曾临时到 562，删除后回落）⇒ 远低于硬线 |
| 不碰 `tests/it_core/large_file_guard.rs` | ✅ **未触碰**（该文件 mtime 18:44:02，早于本会话首个编辑 ~20:04 ⇒ 是并行 worker 的改动，非本轮） |
| TEMP DIAGNOSTIC 清零 | ✅ `grep -rn 'TEMP DIAGNOSTIC' src/ --include=*.rs` = **0 命中**；临时文件 `/tmp/plan_repair_clean.rs` 已 `rm` |
| mtime 避让 | ✅ 改前每次 `ls -la --time-style=+%H:%M:%S`；19:29:32（前一 worker 定稿）→ 20:22 窗口过后才动手；期间 Main 确认「无人写该文件」；`lc_fixture_support.rs`（19:04:41）**全程未改**；`logical_test_fixture.rs`（归 `FinalClusterClose2`）**未碰**；`group.rs` 改动前 mtime 20:08:03（距当时 29 分钟，窗口外），且改前实测其内容与 HEAD 一致（`git status` 无输出）⇒ 无并发写者 |

---

## 6. 回归与 concerns

### 回归

- `cargo test --locked --lib plan_repair_activation` → **2 passed / 0 failed**
- `cargo test --locked --lib workspace_ws_handler`（整簇）→ **229 passed / 0 failed**
  （`5741 filtered out; finished in 9.82s`）

⇒ **本簇 61 → 0**（前一个 worker 收口时是 61 → 2，本轮把最后 2 条收掉）。

### concerns（交给 Main 裁决，本轮未自行处理）

1. **「无快照 + routing=Logical」这个状态在生产不可达，但夹具可以造出来。**
   `get_attempt_for_work_item_group` 的 `None` 桶在实现契约里就是
   「legacy 语义桶」；生产上 routing=Logical 的 attempt 一定带快照。
   ⇒ 本轮采取的「桶不参与查找」口径是**夹具层**的，不改实现契约。
   ⇒ **已按 Main 授权补文档警示**（零行为变化）：
   `src/product/coding_attempt_store/group.rs::get_attempt_for_work_item_group`
   的 doc comment 新增一段，明确 `target` 是【桶键】而非「按 plan 取唯一 attempt」的
   便捷参数，并指向 `list_attempts_for_work_item_group`。

2. **【后续立项】把「按 plan 取 group attempt」提升到共享夹具层。**
   本轮 `plan_repair_attempt` 是 **web 侧本地 helper**（`plan_repair_activation.rs`），
   其它簇若踩同一个坑仍需各自写一份。
   ⇒ 建议在 product 级共享夹具（`logical_test_fixture.rs` 一侧）提供统一入口。
   ⇒ **属本波范围外**，Main 已明确不授权本轮做。

3. **`lc_fixture_support.rs` 仍是未入库新文件**（`git status` 显示 `??`）。
   本轮未 `git add`（纪律要求）⇒ 收口时由 Main 统一入库（Main 已确认为其活）。

4. **本簇口径更新**：`web::workspace_ws_handler` 61 → **0**（本轮把最后 2 条收掉）。
   前一个 worker 的报告 §「仍未解项 #8
   `gateway_start::logical_plan_validate_failure_is_reported_by_handler`」
   需 Main 复核：那条**不在**本簇文件内，本轮**未触碰**；
   但整簇复跑 `229 passed / 0 failed` 实测**未再出现该条红**
   ⇒ 它应已被并行 worker 收掉（Main 侧可核）。

5. **诊断通道本身有残留风险（已清零，但值得记）**：`receive_type` 静默丢弃非目标
   type 帧这个盲区**仍在实现里**（`plan_repair_activation.rs:55-61`）。
   本轮只是在自己的测试里绕开（诊断段读全帧）。
   ⇒ 若后续还有「websocket 超时」型失败，建议沿用本轮做法：
   **先加临时全帧读取通道拿到真实帧，再动手**。
