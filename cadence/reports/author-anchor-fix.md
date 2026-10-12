# E 锚点消费缺口修复：author launch 恒用 native_launch_anchor（PlanningReadOnly 成员分支缺口）

日期：2026-10-12 ｜ worktree：`feat-b-0808-add-monorepo` ｜ 定论：max-task（数学级，Main 转发冻结）

## 1. 背景与根因

E 包（fix-planning-session-anchor-recovery REQ-PLN-08，6c6cec48）已交付
`WorkspaceRepositoryContext.native_launch_anchor` 双输出（LC 规划会话恒聚合根视图），但
`author_root_launch` 消费面存在缺口：

- 断点：`src/product/workspace_engine/provider_drive/author_root_launch.inc.rs`
  - `:205` 读 `business_repository`；
  - `:258` `Some(checkout)` 分支仍以业务成员 checkout 构造 `PolicyTarget::checkout`；
  - `:268-269` E anchor 只在 `None`（deferred）臂生效。
- 现场指纹（今晨 design 修订漂移）：聚合根 `388779…`（生成轮，相等）vs busi 成员
  `dda858…`（修订轮，不等）。design 生成轮无业务态（involved 空 → business=聚合视图 →
  deferred 臂聚合根 target）；修订轮 scope 回写 involved=4+change_order 后 business 恢复
  首成员解析 → `Some(checkout)` 臂成员 target → resume 指纹四漂移域
  （target logical id / checkout / worktree / git identity，`SessionResumeFingerprint::
  from_envelope` 五参签名域）全动 → `resume_or_start` 判漂移 → supersede → 修订死区。
- story 同构造侥幸通过：生成轮 focus 已回写成员 → 两轮同为成员 target → 恰好相等。

同族消费点排查（防第三处漏网）：
- `review/drive.rs:890`（revision 路径）经 `build_root_launch_request` 同一内核——随本修一并收敛；
- `gateway_start.rs`（plan author）已消费 `workspace_native_launch_anchor_for_session`（B2b1/E 包已修）；
- `coder_root_launch.inc.rs` 是 `CodingTargetWrite` 写边界（attempt target_snapshot 语义），非
  PlanningReadOnly 消费点，不动；
- `work_item_split_engine/prompts.rs`、`provider_workspace_runner.rs`、`followups.rs` 等为
  business 材料面（prompt/compile/结构摘要），保持原字段（B1 分层铁律：SC compile/language.md
  走原字段，business 材料来源不变）。

## 2. 修法（冻结口径）

`build_root_launch_request` 的 native launch target **一律**取 `native_launch_anchor`：

```rust
let target = match native_launch_anchor {
    Ok(anchor) => PolicyTarget::aggregate_root(anchor.path.clone()),
    Err(error) => return Some(Err(format!(
        "resolve logical author native launch anchor failed: {error}"
    ))),
};
```

- PlanningReadOnly（story/design/plan）的 anchor 恒 LC 聚合根视图（`primary_checkout_id=None`），
  scope 回写（involved/focus/change_order/target）不迁移锚——resume 指纹四漂移域恒定；
- `business_repository` 保留原用途：gateway 注入谓词 fail-closed（`logical_repository_id.is_none()`
  拒收）与业务材料面，不再派生 native launch target；
- 原 `Some(checkout)` 成员臂删除；`PolicyTarget::checkout` 在本函数不再使用；
- cwd/材料面零变化：`input.working_dir`（业务材料锚，成员 checkout）与
  `envelope.working_directory`（恒 canonical root）不受影响；
- 注释同步：`resolve_author_root_launch`/`build_root_launch_request`/`provider_drive.rs`
  LC 分支/`review/drive.rs` revision 决策 doc 的旧「成员 checkout target」语义更新为锚口径。

## 3. TDD 证据

红（修复前，`cargo test --lib anchor_t12_design_revision_launch_fingerprint`）：

```
assertion `left == right` failed: Design 两轮 launch 指纹必须相等（native target 恒聚合根锚，scope 回写不迁移）
  left:  "…sha256:44707c204ec8182bc1c1b39d645a80bc211e11e81949542d420360bb826e1c1d"  （生成轮，聚合根）
  right: "…sha256:fd92d56931b92b931437a4e802710232d4f9c3709bed17e48737c3e1ead27cf7"  （修订轮，busi 首成员）
```

修复后绿：`anchor_t12…` ok（0.16s）。


新增用例（`root_cwd_revision.inc.rs`，四成员 LC + Design/Story 夹具与
`logical_revision_fixture` 登记链同构）：

- `anchor_t12_design_revision_launch_fingerprint_stable_across_scope_writeback`
  （主锁）：Design 生成轮（无业务态）→ 回写 involved=4+change_order → 修订轮；两轮
  `build_root_launch_request` → `gateway.validate` 指纹相等 + 修订轮 target 聚合根形态。
- `anchor_t13_story_focus_writeback_launch_fingerprint_stays_aggregate`
  （max-task 定案补验①）：Story 生成轮 focus=None → `update_story_spec_aggregate` 回写
  involved/focus → 修订轮；两轮指纹相等、锚恒聚合根（「story 侥幸通过」变必然）。
- `anchor_t19_story_revision_fresh_dispatch_resolves_after_completion` ③ 段
  （max-task 定案补验②）：Waiting → claim → dispatch（显式 fresh）→ 完成 → Resolved →
  **普通修订重驱不再被 LC_RESUME_RECOVERY_ACTIVE 拒**：真实 spawn、原样续接 fresh native
  （`resume_provider_session_id == fresh native`，锚恒聚合根 → 指纹不漂 → 不 supersede）、
  recovery 保持 Resolved 不复活（一次 fresh 即解锁，非永久锁）。
- socket 入站门正例（`socket.rs` anchor_t12 测试）：Resolved 后普通 `RequestRevision`
  经 `lc_resume_recovery_inbound_gate` 放行（Waiting 期拒收负例已有，补齐解锁正向锁）。

既有形态断言同步（修复后新语义）：

- `logical_revision_rebinds_root_cwd_and_preserves_revision_target`：envelope target
  断言从成员 checkout 改为聚合根锚（cwd 仍 root、`input.working_dir` 仍成员材料锚）；
- `logical_author_and_choice_followup_keep_root_cwd_and_target_separate`（web）：同上。

「不要」清单遵守：不忽略指纹域、不清 Waiting、不自动 fresh。

## 4. 回归

| 回归面 | 命令 / 覆盖 | 结果 |
|---|---|---|
| 引擎侧 author/revision 全族（t07/t11/t12/t13、resume 漂移 supersede、主用例新锚断言） | `cargo test --lib author_revision_loop` | 26 passed / 0 failed（0.67s） |
| web 侧 fresh 族（t19 story/design 正例含 ③ 解锁段、漂移负例、prestart 失败回等待） | `cargo test --lib anchor_t19` | 4 passed / 0 failed |
| socket 入站门（anchor_t12 三态 + 新增 RequestRevision 解锁正例） | 全量内 | ok |
| E/B0 resolver 族（anchor_t01/t02、plan S6、SC 材料面不随锚 anchor_t17） | 全量内 | ok |
| planning / gateway_start / conversational gate / recovery_task7 / review drive | 全量内 | ok |
| **全量** | `cargo test --lib` | **4407 passed / 0 failed / 7 ignored（51.64s）** |

## 5. 提交

- `fix: author launch 恒用 native_launch_anchor（PlanningReadOnly 成员分支缺口）`
- 文件：`author_root_launch.inc.rs`（修）、`provider_drive.rs`/`review/drive.rs`（注释同步）、
  `root_cwd_revision.inc.rs`（t12/t13 + 主用例断言）、`root_cwd_launch.inc.rs`（web 断言）、
  `author_revision_fresh.rs`（t19 ③）、`socket.rs`（门正例）、本报告。
