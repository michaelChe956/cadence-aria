# WP4 底座适配核查报告：跨仓证据/每仓交付底座在多 target 分流形态下的适配（T4 / REQ-COD-05/06，REQ-COD-01/02/03 旁证）

- **change**: `openspec/changes/multi-repo-group-coding/`
- **计划**: `cadence/plans/2026-09-19_计划文档_阶段4-C2_多仓group编码_v1.0.md`（v1.1）Task 4（WP4.1-4.3）
- **状态**: 全量交付——核查清单全项结论+1 真缺口定点修复（`compute_issue_delivery_summary` per-WI 覆盖关系，含 fix round 1 排序修正）+一致性测试（对接 WP3 `2b184d7d` 落地面）+底座回归全绿。
- **性质**: 核查型 Task（计划 TDD 豁免合理形态）——「核查=跑通+留依据」，真缺口才定点改造；本 WP 唯一改造项限于适配不改语义（约束 2），单值 schema 零触碰。

> **fix round 1（k3 审 1×P2）已并入**：`list_attempts_covering_work_item` 排序缺陷——per-WI 与 group attempt 的 `attempt_no` 来自不同计数空间（group 锚桶首 WI），按 `(attempt_no,id)` 混排不可比（陈旧 per-WI 可压过新 group 或反向）。修法=k3 建议：`created_at` 为主排序键（`Utc::now().to_rfc3339()` 同格式字典序=时间序），`(attempt_no,id)` 仅同刻 tie-break。详见 §六。

## 一、核查对象逐项表（对象/入口锚点/结论/依据或改造项）

### 4.1 跨仓证据注入（REQ-COD-05，Step 1）

| # | 对象 | 入口锚点（实施 HEAD 实读） | 结论 | 依据 / 改造项 |
|---|---|---|---|---|
| 1 | 令牌→attempt 反查 | `evidence_mediator.rs:218-268` `resolve_attempt_by_token`（扫 attempt 分区 `evidence-token.json` 比对哈希） | **无需改造** | attempt 分区寻址天然 per-attempt：多 target 下每 target-attempt 独立分区+独立令牌。测试：`multi_target_attempts_keep_evidence_chain_partitioned_per_attempt` ①（A/B 双 token 各自命中自身 attempt，快照 logical id 分桶互不串扰） |
| 2 | 令牌签发/校验 | `evidence_token.rs:61-84` `issue_evidence_token`（worktree `.aria/evidence-token`+分区哈希记录）；`:88-115` `validate_evidence_token`（Running+常时比对） | **无需改造** | worktree 路径经 `worktree_path_for_attempt`（`tool_format.rs:90-105`）按 branch 前缀推导：`aria/issues/{issue}/{uuid}` 前缀推出 `{repo}/.worktrees/aria-issues/{issue}/{uuid}`——与 OQ2 多 target 命名**天然一致**（同测试①：B 令牌落 B 自身 worktree）。签发挂点 `coding_workspace_engine/lifecycle.rs:325` `execute_worktree_prepare`（:400-418 处 per-attempt 幂等签发、失败 warn 降级不阻断） |
| 3 | target_snapshot 锚定+ACL | `evidence_mediator.rs:114-165` 六步编排②③：`attempt.target_snapshot` 锚定（None→`evidence_not_available`/404）→ `resolve_target_member_dir`（:406-449）+manifest 成员集排除本仓/非成员 | **无需改造** | target 维度由 attempt 自身冻结快照承载（D1 同款路由权威）：A（target=api）只见 web/、B（target=web）只见 api/——方向由各自快照决定非全局状态。测试同上②②'（双向 ACL 断言） |
| 4 | 快照钉住（pin） | `evidence_mediator.rs:308-361` `load_or_pin_index`/`load_pinned_index`（attempt 分区 `evidence-index-pin.json`） | **无需改造** | attempt 分区文件天然独立。测试同上⑤（A/B 各自分区 pin 文件独立在案） |
| 5 | 预算口径 | `evidence_budget.rs`（`EVIDENCE_ATTEMPT_CHAR_QUOTA=120_000` :29；attempt 分区 `evidence-budget.json` ledger） | **无需改造** | 每 attempt 独立配额 ledger，额度与单仓完全一致。测试同上③（A 消费后 B 仍全额-自身消费；两分区 ledger 文件各自在案） |
| 6 | 审计口径 | `evidence_audit.rs:62-92` `append_evidence_audit`/`audit_path`（attempt 分区 `evidence-audit.jsonl`） | **无需改造** | 每 attempt 独立 jsonl+sequence。测试同上④（A/B 分区各 1 条、互不含对方 attempt_id） |
| 7 | 端口文件注入 | `evidence_injection.rs:73-99` `inject_web_endpoint`（workspace 根端口文件→worktree `.aria/web-endpoint` 幂等复制）；`write_evidence_query_script`（:101+） | **无需改造** | per-worktree 注入——挂点 `lifecycle.rs:419-433` 以 `journal.worktree_path`（per-attempt）调用；OQ2 命名下各 target worktree 独立接收注入物（T2 引擎族 `split_advance_creates_one_attempt_per_target_...` 已锁 per-target worktree 独立+repo 维三元键登记） |

### 4.2 每仓独立交付（REQ-COD-06，Step 2）

| # | 对象 | 入口锚点 | 结论 | 依据 / 改造项 |
|---|---|---|---|---|
| 8 | git_operation 交付链 journal | `git_operation.rs:90-281`（`prepare_coding_git_operation`/`advance_...`/`complete_review_...`/`reopen_failed_...`——journal 落 attempt 分区 `coding-git-operations`）；`build_journal` :480-514 绑定 attempt 自身 `branch_name`/`base_branch`/`worktree_path`（身份不匹配即 `IdentityMismatch`） | **无需改造** | 每 target-attempt 独立 branch/journal（OQ2 命名各自落地）；跨 target 串扰在身份校验面被结构性排除。测试：`multi_target_delivery_chains_are_independent_per_repo`（真实双仓双 bare 远端：A 仓 pre-receive 拒绝→Completed(Failed)；B 仓 Completed(Pushed)；journal/branch 互异断言） |
| 9 | 交付链执行（commit/push/ReviewRequest） | `internal_pr_review.rs:836-1166` `execute_review_request`（per-attempt；交付前 cross-target 越界统一门 :848-853；push 失败不阻断、`push_error` 显式） | **无需改造** | A 仓 push 失败不影响 B 仓交付链（同测试：A Failed 与 B Pushed 并存、重推 A 后双 Pushed）。ReviewRequest 存储 `report.rs:41-60/:78-95` attempt 分区独立（同测试：各分区恰 1 条） |
| 10 | **issue 级 per-WI 交付聚合** | `issue_delivery.rs:53-126` `compute_issue_delivery_summary`（消费面：`handoffs.rs:450-465` 完成门 `maybe_complete_issue_delivery`+`web/handlers/lifecycle.rs` DTO 组装） | **真缺口→已定点修复** | 原实现 per-WI 关联仅 `attempt.work_item_id == wi`（`attempt.rs:319-339`），而 group attempt 仅在 `work_item_id` 承载桶内拓扑序首个 WI（`group.rs:251-292` `create_group_attempt`）——多 target（及单 target group）增殖后非首个 WI 条目永远 `attempt_status=None`→issue 永远 `Partial`、完成门永不触发，且与 WP3 per-target 口径不一致。**改造项**（限于适配不改语义）：新增 `list_attempts_covering_work_item`（:128-169）=「`work_item_id` 直接绑定（WorkItem scope 既有语义零变化）∪ WorkItemGroup attempt 经物化 coding units 覆盖（`unit.logical_work_item_id`——与 T2 web 投影消费面 `lifecycle.rs:172-199` units 索引模式一致）」，取最新 attempt（排序口径经 fix round 1 修正为 `created_at` 主序，见 §六） |
| 11 | per-target 投影（WP3 对接） | `plan_group_projection.rs:107-190` `compute_plan_group_projection`（`2b184d7d` 落地） | **判定一致性已验证** | 测试：`issue_delivery_and_plan_projection_agree_on_multi_target_facts`——同一多 target durable 事实三相位（未启→全交付→partial failure）：每 target 交付 ⟺ 桶内全部 WI 条目满足（Completed+Pushed）；attempt 身份/分支/展示名（`resolve_repository_name` :176 vs `resolve_plan_target_repository_name`）两口径同源；未启相位 issue=Partial（条目 Created 在案非 None）与 plan=NotStarted（未启优先）语义同构非成功；失败分桶一致（api Failed+push_error 显式/web Pushed 不受影响） |
| 12 | 展示名解析 | `issue_delivery.rs:176-197` `resolve_repository_name`（`work_item.target_repository_id` per-WI 经 strict 解析取 checkout 末段，缺省回落 id 串） | **无需改造** | 多 target 下各 WI 各自 target 解析各自展示名（一致性测试断言 `checkout_repo_api`/`checkout_repo_web` 分桶正确） |
| 13 | RuntimeBindingStore | `runtime_binding_store.rs`（`CreateRuntimeBindingInput.repo_id` :15；`find_by_repo_and_task` :101——`(project,issue,repo_id,task_id)` 键） | **无需改造（登记）** | per-repo 记录天然分仓；消费面 `compatibility_scan.rs:76/:220`（每仓独立 binding 建立/查重）与 `dto.rs:677`（只读列表）均不经编码执行分流路径，多 target 无行为耦合 |

### REQ-COD-01/02/03 旁证（约束 2：语义不动，T4 仅核查）

| # | 对象 | 结论 | 依据 |
|---|---|---|---|
| 14 | REQ-COD-01 单目标仓 worktree 执行 | 行为不变 | `execute_worktree_prepare`（`lifecycle.rs:325`）per-attempt 调用+`worktree_path_for_attempt` 前缀推导含 OQ2 嵌套形态（#2）；per-target worktree 独立由 T2 `split_advance_creates_one_attempt_per_target_with_frozen_snapshots_and_units` 锁定 |
| 15 | REQ-COD-02 attempt 冻结快照 | 语义不动 | 证据链②③以 `attempt.target_snapshot` 为权威（#3——A/B 快照各自承载 target 维度）；本 WP 零触碰 `coding_models/execution.rs` 单值字段 |
| 16 | REQ-COD-03 三元键 worktree | 行为不变 | repo 维 `upsert_repo_shared_worktree`/`try_acquire_repo_worktree_lock` 三件套由 T2 分流循环接线（`b154839f`，T2 报告§八）；本 WP 复核无新增耦合 |

## 二、TDD 红绿证据（改造项 #10）

- **红**（隔离树 `/tmp/c2t4-verify`，HEAD=`58cc2df5`+新测试、实现回退预修复形态）：
  - `group_split_delivery_covers_bucket_work_items_via_units` FAILED（:419 `overall`——期望 AllPushed 实得 Partial）；
  - `group_split_partial_failure_is_explicit_per_bucket` FAILED（`left: None, right: Some(Completed)`——w2 条目 attempt_status 缺失即缺口本体）；
  - `multi_target_delivery_chains_are_independent_per_repo` FAILED（:228 `left: None, right: Some("aria/issues/issue_0001/{uuid}")`——w2/w3 条目分支缺失）。
- **绿**（修复后同树+主树双验）：上述 3 测试+`latest_covering_attempt_wins_across_direct_and_unit_association`（最新覆盖序语义钉）+一致性测试全绿。
- **核查型测试（预期绿=「无需改造」依据）**：`multi_target_attempts_keep_evidence_chain_partitioned_per_attempt` 红绿两态均绿——证据底座按 attempt 分区天然适配，零代码改动。

## 三、底座语义零变化回归

| 域（`cargo test --locked --lib <filter>`） | 结果 |
|---|---|
| `logical_codebase`（证据底座全族） | 275 passed / 0 failed |
| `issue_delivery`（交付聚合+一致性） | 14 passed / 0 failed |
| `delivery`（engine 交付族含既有 `maybe_complete_issue_delivery_*`） | 36 passed / 0 failed |
| `coding_attempt_store`（存储面全量） | 175 passed / 0 failed |
| `coding_workspace_engine`（引擎全量） | 523 passed / 0 failed |
| `cargo fmt --check` / `cargo clippy --locked --lib --all-targets` | 幂等 / 0 error |

既有 7 个 issue_delivery 测试（WorkItem scope 语义）零改动全绿——覆盖关系扩展为超集，WorkItem scope 直接绑定语义逐字节保持。

## 四、单值 schema 零改动自查

- `src/product/coding_models/execution.rs`、`src/product/coding_attempt_store/inputs.rs`：`git diff 58cc2df5..HEAD` 与工作树均零 diff（单值字段结构零触碰）。
- 改造项仅 `issue_delivery.rs` 聚合读取面+测试——无 durable 记录 schema 变更、无 wire 变更。

## 五、结论

REQ-COD-01..06 底座在多 target 分流新形态下：**13 项无需改造**（attempt 分区寻址/冻结快照权威/OQ2 命名兼容结构性成立，测试逐项钉死）、**1 项真缺口已定点修复**（#10 per-WI 覆盖关系，适配性扩展不改判定语义）、**1 项 WP3 对接口径一致性验证通过**（#11）。核查清单全项闭环；底座回归全绿。

## 六、fix round 1（k3 P2：跨计数空间排序缺陷，2026-09-19）

**缺陷**：#10 改造项首版排序按 `(attempt_no, id)`——但 per-WI attempt 的编号取自该 WI 自身计数空间（`list_attempts_for_work_item(wi)` 计数），group attempt 的编号取自桶内首个 WI 的计数空间（`create_group_attempt` 锚 `current_work_item_id`）。两空间互不可比：非桶首 WI 的陈旧 per-WI attempt（编号可达 2+）可压过更新的 group attempt（编号 1），使该 WI 条目永远持旧失败态、`maybe_complete_issue_delivery` 完成门永不触发；反向形态则误提前持旧 group 的 Completed。

**修法**（k3 建议）：排序主键改 `created_at`（两类 attempt 均以 `Utc::now().to_rfc3339()` 落盘——同格式字典序=时间序），`(attempt_no, id)` 降为同刻 tie-break。同计数空间内（纯 per-WI 或纯 group 序列）created_at 与编号同向，既有顺序语义不变。

**TDD 红→绿**：
- 红（修复前，主树定向）：①`latest_covering_prefers_newer_group_over_stale_per_wi_attempts`——w2 持 2 个陈旧 per-WI（编号 1/2、created_at 更早、交付失败）+新 group（锚 w1 编号 1、created_at 更新、Pushed）→实得 `Partial≠AllPushed`（陈旧编号 2 压过 group）；②`latest_covering_prefers_newer_per_wi_over_stale_group_attempt`（反向）——旧 group（编号 2、created_at 更早、Failed）+新 per-WI（编号 1、created_at 更新、Pushed）→实得分支=group 非 per-WI。
- 绿（修复后）：双测通过；issue_delivery 全模块 13 测试、`delivery` 族 38、`coding_attempt_store` 179、`coding_workspace_engine` 523 全绿；clippy 0 error；本文件 `rustfmt --edition 2024 --check` 干净（主树全树 `cargo fmt` 因兄弟任务 Rust 中间态暂不可运行，单文件归一等价）。
