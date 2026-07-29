## Why

删除一个 work item group（`DELETE /api/projects/{p}/issues/{i}/work-item-plans/{plan}`）在实测中出现三处缺陷叠加，导致用户**删不掉**一个已经卡住的 group，且删除即使成功也**删不干净**：

1. **半删后无法自愈**。删除路径 `delete_schema_v2_work_item_plan_with_cleanup`（`src/web/handlers/lifecycle/deletion.rs:135`）假设「对象完整、环境健康、原子成功」。上一轮删除在 `cleanup_coding_attempt_workspace` 访问已被手动删除的 worktree 时中断，留下半删残留（attempt 数据与 WorkItem session 已删，plan / revisions / plan-store / shared-worktree 仍在）。第二次删除时，前置完整性校验（`deletion.rs:201-255` 要求每个 logical work item 都有 runtime session 且 `resolve_workspace` 成功）发现自己上一轮删掉的 session 不见了，直接返回 `IdentityMismatch::runtime_binding_missing`，删除根本走不到清理步骤。

2. **错误信息被吞**。`product_store_api_error`（`src/web/handlers/support.rs:245`）把所有 `ProductStoreError` 压成 `code=product_store_error / message="product store operation failed" / details={}`。`runtime_binding_missing` 这个 kind、具体哪个对象缺失——全在转换时丢失，前端只看到笼统报错，无法定位。

3. **即使完整删除也漏删四类产物**。`delete_schema_v2_issue_work_item_plan_metadata`（`src/product/lifecycle_store/plan.rs:116`）只删 plan json 与 WorkItemPlan 类型 session。`work-item-revisions/<plan>/`、`work-item-revision-publications/<plan>/`、`issue-shared-worktree.json`、coding-attempts 残留 `.lock` 这四类**没有任何删除路径覆盖**。

**根因是同一模式**：删除路径把「被删除对象必须处于完整健康状态」当作前置条件，违背「清理路径不应要求被清理对象健康」原则。它自己制造半删残留后，就无法再清理——因为它把自己要校验的数据删掉了一半。

## What Changes

- **新增删除前置门禁**：删除 group 前，若该 group 存在对应的 coding attempt（`get_attempt_for_work_item_group` 返回 `Some`），MUST 拒绝删除并提示用户先删除 coding workspace。这是行为变化——旧行为是「删除 group 时自动 abort 并删除 attempt」，新行为是「有 coding workspace 则拒绝，保护 coding 劳动成果不被随 group 误删」。attempt 记录消失后才放行。
- **删除改为尽力清理 + 容错**：通过门禁后，MUST 尽力删除 group 的全部产物。每个删除步骤 MUST 把「产物不存在」视为成功，绝不因某项产物缺失、worktree 目录缺失或 revision 半残而中断整个删除。
- **WorkItem session 收集改为不依赖 bindings 完整**：MUST 扫描所有 `WorkspaceType::WorkItem` session 中 `work_item_runtime_binding.plan_id == plan` 的记录逐个删除，而不是依赖 plan revision 的 `work_item_bindings` 数量匹配。这是「半残也能删」的关键。
- **补齐漏删的四类产物**：revisions 整目录、revision-publications 整目录、shared-worktree、attempt 残留 lock。
- **错误透明**：`product_store_api_error` 的兜底分支 MUST 把 `ProductStoreError` 的 `kind` / `id` / `message` 带进 `details`，不再返回空对象。
- 同步为 legacy 删除路径（`delete_work_item_plan` 的 else 分支与 `delete_work_item_with_cleanup`）加入同一门禁，保证两条路径语义一致。

## 非目标

- 不改 `DELETE /api/coding-attempts/{id}` 接口本身的 worktree 容错（`cleanup_coding_attempt_workspace`）。该接口同样依赖 worktree 健康，worktree 缺失时删 attempt 也会失败，属独立缺陷，另行立项。
- 不改 `purge_plan_artifacts`（已正确清理 plan store）。
- 不引入「删除前快照 / 回滚」的事务机制——文件系统下尽力清理 + 容错已是合理选择。
- 不为历史持久化数据提供迁移。
- 不改变 `get_attempt_for_work_item_group` 的判定逻辑（保持读 attempt json 过滤 `work_item_group_id`）。

## Capabilities

### New Capabilities

- `work-item-group-deletion-resilience`：work item group 删除的健壮性契约，包括 coding workspace 存在性门禁、尽力清理与容错原则、产物删除完整性边界、以及删除失败的错误透明性。

### Modified Capabilities

（无。现有 specs 未覆盖 group 删除的健壮性与门禁语义。）

## Impact

- `src/web/handlers/lifecycle/deletion.rs`：重写 `delete_schema_v2_work_item_plan_with_cleanup`（门禁 + 尽力清理 + 去掉旧完整性校验）；legacy 路径同步加门禁；新增 attempt 残留 lock 清理。
- `src/product/work_item_revision_store/`：新增 `purge_plan_revisions`，删除 `work-item-revisions/<plan>/` 与 `work-item-revision-publications/<plan>/` 整目录（复用 `plan_root`，`paths.rs:8`）。
- `src/product/lifecycle_store/worktree.rs`：新增 `delete_issue_shared_worktree`，删除 `issue-shared-worktree.json` 与 `.lock`。
- `src/web/handlers/support.rs`：`product_store_api_error` 兜底分支带 `kind/id/message` 进 `details`；新增 `coding_workspace_exists` 错误码。
- `tests/it_web/web_coding_attempt_api/`：新增门禁拒绝、完整删除无残留、半残删除无残留、错误透明的端到端测试；现有 `delete_work_item_plan_cascades_*` 按新门禁语义调整。
- 受影响的用户可见行为：group 有 coding workspace 时删除被拒绝并给出明确提示；删除成功后该 group 的全部产物消失，issue 下的 spec / issue 本身 / 仓库注册不受影响。

## 依赖与顺序

本 change 与现有各 change 独立。删除路径与其他 change 修改的 prompt / 校验 / reviewer 逻辑无交集。
