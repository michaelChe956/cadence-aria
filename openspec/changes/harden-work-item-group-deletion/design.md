## 背景

work item group 的删除路径与 coding attempt 的删除路径是两条不同的链路，但共享同一组产物目录。当前 group 删除路径对「环境健康」做了强假设：

| 假设 | 实际 | 后果 |
|---|---|---|
| 每个 logical work item 都有 runtime session | 半删后 session 已删 | 数量校验失败，拒绝删除 |
| worktree 目录存在且可 resolve | 用户可能已手动删 worktree | `cleanup_coding_attempt_workspace` 中断 |
| 删除是原子的 | 文件系统无事务 | 中断即留半删残留 |
| 删除路径只删它知道的产物 | 漏删 revisions/publications/shared-worktree/lock | 即使成功也残留 |

## 根因

三处独立缺陷，同源于「清理路径要求被清理对象健康」：

1. `deletion.rs:201-255` 的前置完整性校验：要求 `sessions_by_logical_id.len() == expected_logical_ids.len()`，且每个 session 的 `work_item_runtime_binding` 存在、`resolve_workspace` 成功。任一不满足即返回 `IdentityMismatch`，删除走不到清理步骤。这是用户当前「删不掉」的直接原因——上一轮删除已把 WorkItem session 删掉，数量校验 0≠2 失败。

2. `support.rs:245` 的 `product_store_api_error`：match 分支只对少数 `NotFound` kind 做了精确映射，兜底分支返回空 `details={}`。所有 `IdentityMismatch`、大多数 `Io` 都落到兜底，真实原因（哪个 kind、哪个 id）在前端不可见。

3. `plan.rs:116` 的 `delete_schema_v2_issue_work_item_plan_metadata` 只删 plan json + WorkItemPlan 类型 session。`work-item-revisions/<plan>/`（revision store 的 `plan_root`，`paths.rs:8`）、`work-item-revision-publications/<plan>/`、`issue-shared-worktree.json`、coding-attempts 残留 `.lock` 无任何删除覆盖。

## 决策

### 决策一：门禁判据是「coding attempt 记录存在」，不是「active」

用户约束是「有对应 coding workspace 就不能删除」。判定采用 `get_attempt_for_work_item_group(plan)`（`coding_attempt_store/group.rs:70`）返回 `Some` 即拒绝，**不区分 attempt 状态**。

理由：
- 该方法读 attempt json 过滤 `work_item_group_id == plan`，attempt json 不存在即返回 `None`（视为已删）；残留 `.lock` 不被读取，不影响判定。
- 「coding workspace 是否存在」以 attempt 记录为准最安全。一个 aborted 的 attempt 仍可能有代码与 review 产物，让用户显式删除 coding workspace（`DELETE /api/coding-attempts/{id}`）后再删 group，比随 group 自动清理更可控——这正是用户要的语义，替代了旧行为「删除 group 时自动 abort+delete attempt」。
- attempt 记录消失（用户已删 coding workspace）即放行，覆盖用户当前场景（attempt json 上一轮已删）。

### 决策二：尽力清理，每步「NotFound=OK」

通过门禁后，删除 MUST 尽力进行到底。每个删除步骤用 `remove_file_if_exists` / `remove_dir_all_if_exists`（`lifecycle_store/utils.rs:159,170`）或 `NotFound` 视为成功，绝不因某项产物缺失而 `return Err`。

这与旧设计的根本区别：旧设计把「完整性校验」放在删除**之前**当门禁；新设计把「容错」放在删除**之中**当原则。删除不需要被删对象先自证健康。

### 决策三：WorkItem session 收集不依赖 bindings 完整

旧路径从 `plan_revision.work_item_bindings.keys()` 取 expected logical ids，再要求 session 数量匹配。半残场景下 revision/lineage 可能不完整，bindings 不可靠。

新路径：`list_workspace_sessions` 过滤 `workspace_type == WorkItem && work_item_runtime_binding.plan_id == plan`，凡匹配者逐个 `delete_workspace_sessions_for_entity`（该方法已连带删 timeline，`workspace.rs:638`）。匹配为空也视为成功（没有可删的 session）。

这样 session 清理只依赖 session 自身的 `plan_id` 字段，不依赖 revision 健康。

### 决策四：补漏四类产物，用整目录删除

| 产物 | 路径 | 删除方式 |
|---|---|---|
| revisions 全部子产物 | `work-item-revisions/<plan>/` | `remove_dir_all_if_exists(plan_root)`——所有 revisions 子目录（lineage、plan-revisions、logical-work-items、draft-revisions、verification-plan-revisions、validation-reports、bundles、handoff、repair、amendment 等）都在 `plan_root` 下，一次清空 |
| revision-publications | `work-item-revision-publications/<plan>/` | `remove_dir_all_if_exists` |
| shared-worktree | `issue-shared-worktree.json` + `.issue-shared-worktree.json.lock` | `remove_file_if_exists` 两个 |
| attempt 残留 lock | `coding-attempts/.*.lock`、`work-item-attempt-locks/` | `remove_file_if_exists` / `remove_dir_all_if_exists` |

revisions 用整目录删除而非逐文件，因为 `plan_root` 下的全部内容都属于该 plan，无跨 plan 共享文件。publications 同理。

### 决策五：错误透明改兜底，不逐变体映射

`product_store_api_error` 的兜底 `match _` 分支改为带 `ProductStoreError` 的内容进 `details`：`IdentityMismatch { kind, id }` → `{kind, id}`；`Io(msg)` → `{message: msg}`；其它变体带其可用字段。

不逐变体精确映射，因为：精确映射工作量大且与本次目标（让删除失败可见）弱相关；兜底带细节已能让前端看到 `runtime_binding_missing` 这类真因，覆盖所有变体。已存在的精确映射分支（NotFound 各 kind）保留不动。

### 决策六：legacy 路径同步加门禁，但 revisions/publications 用容错调用

`delete_work_item_plan` 的 else 分支（legacy，无 schema_v2 lineage）与 `delete_work_item_with_cleanup` 加入同一 attempt 门禁，保证「有 coding workspace 就拒绝」语义在两条路径一致。

legacy 路径理论上没有 revisions/publications（schema v2 专属），但调用 `purge_plan_revisions` 也无妨——`remove_dir_all_if_exists` 对不存在的目录返回成功。统一调用减少分支差异。

### 决策七：产物删除完整性边界（不删多 + 不残留）

删除成功后，issue 目录的最终状态 MUST 是：

**保留**：`issue.json`、`story-specs/`、`design-specs/`、`versions/`（spec 历史）、`repository-initializations/`（仓库注册）、以及不属于本 plan 的其他 plan 产物。

**消失**：本 plan 的全部 8 类产物（plan json、WorkItemPlan session、WorkItem sessions、plan store drafts/compiles/outlines、revisions、revision-publications、shared-worktree、attempt 残留 lock）。

实现上「不删多」由两点保证：① 所有删除路径都以 `plan_id` 为参数定位目录/文件，不触碰其他 plan；② shared-worktree 与 attempt lock 是 issue 级但属该 group 的产物（一个 issue 一个 shared-worktree、attempt 锁绑定 work_item），删除 group 时清理它们正确，不影响其他 group（一个 issue 当前只有一个 group plan）。

「不残留」由决策二（尽力清理）+ 决策四（补漏四类）共同保证。

## 边界

- 不改 `DELETE /api/coding-attempts/{id}` 的 worktree 容错（见已知缺口一）。
- 不改 `purge_plan_artifacts`。
- 不改 `get_attempt_for_work_item_group` 判定逻辑。
- 不改 `delete_workspace_sessions_for_entity`（已正确连带删 timeline）。
- 不引入删除前快照/回滚的事务机制。
- 不改 coding attempt 自身的删除接口语义，只改 group 删除接口对 attempt 存在性的门禁。

## 已知缺口

1. **DELETE coding-attempts 接口的 worktree 容错**（本次非目标）：`cleanup_coding_attempt_workspace`（`support.rs:359`）在 worktree 目录缺失时 `remove_worktree` / `prune_worktrees` / `delete_local_branch` 会失败。若 group 有 attempt 且 worktree 已坏，用户先删 attempt 会失败，从而间接卡住 group 删除。本次只保证「attempt 不存在时 group 能删干净」；attempt 存在 + worktree 坏的组合需删除 attempt 容错立项后才能彻底解决。
2. **`work_item_plan_store` 的 lock 残留**：`purge_plan_artifacts` 清 drafts/compiles/outlines 内容，但目录级 `.lock` 是否随目录消失取决于实现，需在实施时确认无 lock 残留。
3. **legacy `delete_work_item_with_cleanup` 的死代码 abort 循环**（code review Important #1）：该函数门禁（`list_attempts_for_work_item` 有则拒绝）后又读一次 attempts 做 abort+cleanup 循环，门禁通过时该循环永不执行。是「自动 abort」旧语义的残留，留在代码里会误导维护者。本次不删（删它牵连 `find_repository`/`work_item` 连锁清理，且 `delete_attempts_for_work_item` 可能仍清理半残 attempt 目录，需先确认），留待后续立项清理或加防御性注释。
4. **schema v2 删除路径重复读 plan json**（code review Important #2）：`delete_schema_v2_work_item_plan_with_cleanup` 先 `get_issue_work_item_plan` 取 `work_item_ids`，`delete_schema_v2_issue_work_item_plan_metadata` 又读一次。两次读之间存在理论 TOCTOU（外部进程删 plan json），概率极低且后果仅是返回错误而非误删。可接受，留待后续抽「从已加载 plan 删除」的变体优化。
5. **plan json 本身缺失的半残边缘**（code review Minor #2）：删除路径入口 `delete_work_item_plan` 依赖 `get_issue_work_item_plan` 成功。spec 的半残 scenario 覆盖 session/worktree/attempt json 残缺，不含 plan json 本身缺失——若 plan json 也半残，删除仍会 404。属 spec 范围外的边缘。
