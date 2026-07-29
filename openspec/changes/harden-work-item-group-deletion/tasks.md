## 1. 失败测试

- [x] 1.1 门禁拒绝：group 存在 coding attempt 时 `DELETE plan` 返回 `coding_workspace_exists`，details 含 plan_id 与 attempt_id；plan、revisions、attempt 记录删除后仍全部存在。（映射：Requirement 存在 coding workspace 时拒绝）
- [x] 1.2 门禁放行：group 无 attempt 但有残留 attempt lock 时，删除放行且不因 lock 拒绝。（映射：同上）
- [x] 1.3 完整删除无残留：无 attempt 的完整 group 删除后，revisions、revision-publications、plan store drafts/compiles/outlines、shared-worktree、WorkItem session 与 timeline 全部不存在。（映射：Requirement 删除无残留）
- [x] 1.4 半残删除无残留：group 半残（WorkItem session 部分缺失、worktree 目录删除、attempt json 删除但留 lock）时删除成功且无残留。（映射：同上，对应线上卡住场景）
- [x] 1.5 缺失 worktree 不阻断：worktree 目录不存在时删除不报错。（映射：同上）
- [x] 1.6 不误伤 issue 与 spec：删除成功后 issue 记录、story spec、design spec、versions、repository 注册仍存在。（映射：Requirement 不得误伤）
- [x] 1.7 不误伤其他 plan：同 issue 多 plan 时删一个，其他 plan 产物不受影响。（映射：同上；Task 4 以「共享 work-item-attempt-locks 目录播种非本 plan 锁 + 顶层 active attempt 锁保留」等价验证）
- [x] 1.8 错误透明：`product_store_api_error` 对 `IdentityMismatch` 返回的 details 含 kind 与 id。（映射：Requirement 错误透明）
- [x] 1.9 确认以上测试全部失败且失败原因是缺少实现。

## 2. 实现

- [x] 2.1 `WorkItemRevisionStore` 新增 `purge_plan_revisions`：`remove_dir_all_if_exists(plan_root)` + `remove_dir_all_if_exists(work-item-revision-publications/<plan>)`，NotFound=OK。（映射：Requirement 删除无残留）
- [x] 2.2 `lifecycle_store/worktree.rs` 新增 `delete_issue_shared_worktree`：删 `issue-shared-worktree.json` + `.lock`，NotFound=OK。
- [x] 2.3 `deletion.rs` 重写 `delete_schema_v2_work_item_plan_with_cleanup`：顶部加门禁（`get_attempt_for_work_item_group` 返回 Some 则返回 `coding_workspace_exists`）；删除旧的完整性校验段（sessions 数量匹配 + resolve_workspace 强制）；改为扫描 `runtime_binding.plan_id==plan` 的 WorkItem session 逐个删；依次删 plan 元数据、revisions、plan store、shared-worktree、attempt 残留 lock，每步 NotFound=OK。
- [x] 2.4 legacy 路径（`delete_work_item_plan` else 分支与 `delete_work_item_with_cleanup`）加同一 attempt 门禁。
- [x] 2.5 `support.rs`：新增 `coding_workspace_exists` 错误构造；`product_store_api_error` 兜底分支带 `kind/id/message` 进 details。
- [x] 2.6 调整现有 `delete_work_item_plan_cascades_*` 测试以符合新门禁语义（测试夹具需先移除 attempt 或断言有 attempt 时被拒绝），避免编码旧行为的假绿。

## 3. 验证

- [x] 3.1 `cargo fmt --check`
- [x] 3.2 `cargo clippy --all-targets --all-features --locked -- -D warnings`
- [x] 3.3 `cargo test --locked --lib`
- [x] 3.4 `cargo test --locked --test it_web`
- [ ] 3.5 线上数据验证：当前卡住的半残 group 通过接口删除成功，`find .aria/.../issue_0001 -type f` 删除后只剩 issue.json + story/design spec + versions + repository 注册，无残留。
