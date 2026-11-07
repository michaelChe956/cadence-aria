# 竞态与预算源码核实

> 工作树：`/home/michaelche/workspace/github/cadence-aria/.worktrees/bugfix-b-11-02`  
> 执行 `pwd` 输出：`/home/michaelche/workspace/github/cadence-aria/.worktrees/bugfix-b-11-02`  
> 首次检索命令：`cd /home/michaelche/workspace/github/cadence-aria/.worktrees/bugfix-b-11-02 && pwd && rg -n -C 6 'bind_plan_ids_locked|supersede_terminated_generation_and_cas|supersede_terminated_plan_generation|policy_revision|payload.*immutable|revision.*mismatch|expected_revision|Disable|Enable|RootRecipeSnapshotBudget|DEFAULT: Self|max_bytes:|with_snapshot_budget|\.git|max_bytes|cumulative' ...`（目标文件列表见相应各节）。

本报告为指定 worktree 的当前源码核查，未改生产代码。

## 组 A — 自动化换代竞态

1. **仍存在（未修）：plan/session binding 会递增 revision。** `src/product/issue_automation_store_parts/cas_internals.inc.rs:49-56`：
   ```rust
   // C1 Task 1：绑定补全 durable binding current 的 plan/session
   // 身份（enable 时以空串占位；版本不递增——绑定不是换代）。
   if let Some(history) = next.binding_history.as_mut() {
       history.current.plan_id = plan_id.to_string();
       history.current.session_id = session_id.to_string();
   }
   next.policy_revision += 1;
   ```
   注：旁边注释称“版本不递增”，实际语句仍是 `+= 1`，矛盾依代码行为判定。

2. **仍存在（未修）：删除旧 plan 在 CAS 前。** `src/web/handlers/automation_enrollment_reenable.rs:190-207`：
   ```rust
   if was_terminated {
       // 单 plan 不变量（REQ-WPS-01）：重新开启＝旧 plan 作废——旧 plan 及其
       // 完整闭包在 intent 轮换前删除；失败保持既有
       // `automation_plan_cleanup_required` 语义（零副作用、CAS 未执行）。
       supersede_terminated_plan_generation(state, project_id, issue_id).await?;
   }
   let enrollment = if was_terminated {
       ...
       store.compare_and_set_reenable(
   ```
   顺序明确为 cleanup await → `compare_and_set_reenable`。

3. **仍存在（未修）：revision 不符与 enabled 异 payload 共用 Conflict。** `src/product/issue_automation_store.rs:259-272`：
   ```rust
   ) if saved.enabled
       && (saved.selection_key != *selection_key
           || saved.source != *source
           || saved.options != *options
           || saved.target.as_ref() != Some(target)) =>
   {
       CasResolution::Conflict {
           current_revision: Some(saved.policy_revision),
       }
   }
   (Some(saved), _) if expected_revision != Some(saved.policy_revision) => {
       CasResolution::Conflict {
           current_revision: Some(saved.policy_revision),
       }
   }
   ```
   两臂均为 `CasResolution::Conflict { current_revision: Some(saved.policy_revision) }`，由 `cas_internals.inc.rs:19-24` 同样映射成 `EnrollmentError::Conflict`，故错误类别相同。

4. **已修：重开后 Disable 使用刚返回的 revision；无需 Disable 前另发 GET。** `web/src/components/lifecycle/useIssueLifecycleGeneration.ts:541-547,564-569,580-585`：
   ```ts
   current = await putAutomationEnrollment(projectId, card.issueId, {
     expected_revision: current.policy_revision,
   ...
   if (current.enabled) {
     current = await putAutomationEnrollment(projectId, card.issueId, {
       expected_revision: current.policy_revision,
       command: { type: "disable" },
     });
   }
   ...
   await putAutomationEnrollment(projectId, card.issueId, {
     expected_revision: current.policy_revision,
   ```
   初始 GET 的 enrollment 传给 `runReenableSequence`；步骤① Enable 的响应覆盖 current，Disable 使用其当前 `policy_revision`。

## 组 B — 审计预算

5. **当前值：** `src/product/logical_codebase/root_recipe_receipt_types.inc.rs:60-64`：
   ```rust
   pub const DEFAULT: Self = Self {
       max_entries: 20_000,
       max_bytes: 64 * 1024 * 1024,
   };
   ```

6. **仍存在（未修）：`new()` 硬取 DEFAULT。** `src/product/logical_codebase/root_recipe_receipt_auditor.inc.rs:20-24`：
   ```rust
   pub fn new() -> Self {
       Self {
           allowlist: default_allowlist(),
           budget: RootRecipeSnapshotBudget::DEFAULT,
       }
   }
   ```

7. **结论：`.git` 是分类，而非遍历排除；是否忽略由 gitignore 规则决定。** `root_recipe_receipt_store.inc.rs:581-600` 将包含 `.git` 的路径归类为 `MemberGit` / `MemberWorktree`；遍历 `:786-791` 只在 `ignores.is_ignored(...)` 时跳过，否则调用 `scale.charge_entry`。原文：
   ```rust
   /// 成员 `.git`/worktree 判定：路径任一段为 `.git` 即成员 Git 面；其下
   /// 再出现 `worktrees` 段为成员 worktree 面（分类优先于 allowlist：任何
   /// 位置出现的 `.git` 都不因位于 allowlist 内而放行）。
   if component == ".git" {
       saw_git = true;
   }
   if ignores.is_ignored(&relative, file_type.is_dir()) {
       continue;
   }
   scale.charge_entry(budget, &path)?;
   ```

8. **生产调用点数量：0。** 命令：`rg -n 'with_snapshot_budget\s*\(' src --glob '!**/tests/**'`。检索结果只有 `src/product/logical_codebase/root_recipe_receipt_auditor.inc.rs:34` 的定义，没有生产调用；调用命中位于测试文件 `root_recipe_receipt_tests.inc.rs`。

9. **累计字节报错构造位置及 `{path}` 语义：** `src/product/logical_codebase/root_recipe_receipt_store.inc.rs:735-750`：
   ```rust
   fn charge_bytes(
       &mut self,
       budget: &RootRecipeSnapshotBudget,
       path: &Path,
       len: u64,
   ) -> Result<(), ProductStoreError> {
       self.bytes = self.bytes.saturating_add(len);
       if self.bytes > budget.max_bytes {
           return Err(ProductStoreError::Io(format!(
               "audit snapshot budget exceeded at {}: {} cumulative bytes > max_bytes {} \\
   ```
   参数原文在 `:748-750` 为 `path.display(), self.bytes, budget.max_bytes`。遍历侧 `:814-818` 先读当前文件，再 `scale.charge_bytes(budget, &path, bytes.len() as u64)?`。因此 `{path}` 是处理到的、使累计读取字节超过 max_bytes 的当前文件，而不是累计文件集合或任意路径。

## 组 C — 行数门禁实测

10. **GRANDFATHERED 条目数：58。** 用脚本解析 `tests/it_core/large_file_guard.rs` 中该表的路径/cap 元组计数。

11. **实测：actual == cap 为 40；actual > cap 为 0；actual < cap 为 18；缺失为 0。**

12. **与 notes 的 109 / 89 / 0 不一致。** 当前登记 58（少 51），顶格 40（少 49），超限仍为 0；所有登记路径存在。`cadence/notes/2026-10-30_待办_行数上限治理_v1.0.md:20-23` 记录登记 109、顶格 89、超限 0。报告仅确认当前 worktree 数字差异，不推断其来源。

**实测命令（从指定 worktree 根目录执行）：**
```sh
cd /home/michaelche/workspace/github/cadence-aria/.worktrees/bugfix-b-11-02 && pwd
python3 -c 'import pathlib,re; s=pathlib.Path("tests/it_core/large_file_guard.rs").read_text(); block=s.split("const GRANDFATHERED:",1)[1].split("];",1)[0]; xs=re.findall(r"\(\s*\"([^\"]+)\"\s*,\s*(\d+)\s*\)",block); vals=[]
for p,c in xs:
 q=pathlib.Path(p); vals.append((p,int(c),len(q.read_text(errors="replace").splitlines()) if q.is_file() else -1))
print("registered",len(vals),"missing",sum(n<0 for _,_,n in vals),"actual_eq_cap",sum(n==c for _,c,n in vals),"actual_gt_cap",sum(n>c for _,c,n in vals),"actual_lt_cap",sum(n>=0 and n<c for _,c,n in vals))'
```
输出：
```text
/home/michaelche/workspace/github/cadence-aria/.worktrees/bugfix-b-11-02
registered 58 missing 0 actual_eq_cap 40 actual_gt_cap 0 actual_lt_cap 18
```
