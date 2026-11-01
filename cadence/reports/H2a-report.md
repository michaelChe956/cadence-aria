# H2a 报告:执行轮存储清理(设计 2026-10-14 A/C/D 节)

- Change:openspec/changes/fix-multi-repo-group-coding-start 后续热修;设计文档 cadence/designs/2026-10-14_方案设计_Coding工作区整组删除与重启_v1.0.md(owner 已确认,A/C/D 节+切片 H2)
- Worktree:.worktrees/feat-b-0808-add-monorepo(分支 feat-b-0808-add-monorepo)
- 状态:DONE(隔离树红→绿→回归全绿;主树被兄弟包在途编译错误阻塞,见「验证环境」)
- 撞面裁定:Main 授权更新两处被设计语义取代的旧 N7 测试(归因=设计行56/160 取代「Invalidated 留盘」,owner 已批)

## 交付

### Create/Modify(H2a 独占写面)

1. **`src/product/coding_plan_run.rs`**(+836/-52,含测试):
   - 生命周期元数据(设计 A 节「退役/中断围栏」):`{issue_root}/coding-plan-workspace-lifecycles/{plan_id}.json`(目录常量 `CODING_PLAN_WORKSPACE_LIFECYCLES_DIR`),锁为同目录前导点 `.{plan_id}.json.lock`(稳定 inode,重置清理不删;真实 Plan 删除纳入清理由 D2.2/H4 负责)。类型 `CodingPlanWorkspaceLifecycle`(project/issue/plan、单调 generation、retired command/attempt/advance 纯 ID、clearing、纯 ID 回执)、`CodingPlanWorkspaceClearing`(command/generation/pending 四清单/started_at)、`CodingPlanWorkspaceResetReceipt`(command/from/to/completed_at)、`CodingPlanWorkspaceResetChecklist`(冻结清单输入)。只有身份/状态,无旧日志/unit 内容/完整 run 归档。
   - Store API:`get_workspace_lifecycle`/`current_workspace_generation`(缺失=1)/`verify_workspace_round_admission`(围栏:clearing→`coding_plan_workspace_clearing`、退役 command→`coding_plan_run_retired`;无锁原子读,在 with_plan_live_write 共享区内调用与生命周期写(持 Plan 独占)互斥)/`retire_workspace_round_with_lease`(冻结清单+耐久 clearing+退役事实合并;同 command 重入刷新、异 command 拒、已回执 command 幂等零写)/`complete_workspace_reset_with_lease`(generation+1、清 pending、记回执;clearing 缺失同 command 幂等重放)。
   - 围栏接线:`prepare`/`bind_ready`/`mark_start_requested`/`record_start_outcome`/`mark_paused` 五个 run 写点在共享准入区内先核围栏(旧 command 不能因 run 文件缺失重新准备)。
   - 持租约内部入口(设计 A 节:「不二次 try-shared」):`invalidate_for_deleted_attempt_with_lease`(成员删除失效,公开路径在 Plan 独占下自拒 `entity_deleting` 的问题的解)、`invalidate_run_for_reset_with_lease`(闭包冻结时原授权 Invalidated);两者与公开 `invalidate_for_deleted_attempt` 共用 `invalidate_run_file_locked` 核体(NotFound 上抛不吞/身份核对/幂等)。租约身份核对 `verify_plan_lease_identity`(scope 全等)。
   - 测试:夹具抽出 `pub(crate) mod workspace_reset_fixtures`(与 web 闭包测试同源);新增 T1 `plan_workspace_h2a_retired_command_cannot_reprepare`、T2 `plan_workspace_h2a_clearing_fences_and_completes`、T3 `plan_workspace_h2a_invalidate_consumes_held_lease`。
2. **`src/product/coding_attempt_store/attempt.rs`**(+28):`list_attempts_for_plan(project,issue,plan)`——按真实 group attempts(work_item_group_id==plan)枚举,「全删」判据的权威存活集(不以 issue 空集/run 冻结集代替)。
3. **`src/web/handlers/support_parts/coding_cleanup.inc.rs`**(+802,含测试):
   - `PlanCodingWorkspaceResetOutcome{Reset{generation},InitializationPrefix,Clean}` + `close_plan_coding_workspace_round(app_paths,p,i,plan,command_id,expected_generation,require_broken_proof)`:零成员执行轮完整关闭闭包(检测、耐久 clearing、重新准入同一受保护操作——`try_acquire_delete(Plan)` 独占租约,**不立 Plan 墓碑**)。入口:clearing 同 command 重入续清/异 command 409、同 command 回执幂等重放(不再次清新轮)、expected_generation 不符→`coding_plan_workspace_generation_conflict`、Plan 墓碑在位→`plan_coding_run_resource_unavailable`(真删除闭包 own)。租约内:复核(plan 在/零成员/分类)→冻结清单+退役+原授权 Invalidated→清 journal 残留+WI 孤儿锁→目标仓专属资源(复用 D2.2 `cleanup_plan_shared_worktrees`,他人保留)→advance retry→外层 journal→AdvanceRecords→run→复核归零→轮次切换。分类(断裂证明)=run 冻结集非空 ∪ H2b `AdvancePlanExecutionReferences::classify_stale_references(&[])` AllTargetsDead ∪ per-target journal≥AttemptPersisted;仅 Prepared 前缀→`InitializationPrefix`(require_broken_proof=true 时零变更续原协议);无任何引用→`Clean`。
   - `workspace_reset_api_error`:四个执行轮状态冲突码原样透出(409 映射归 H1 白名单,error.rs 未动)。
   - **最后成员钩子**(设计 D 节行160):`finalize_coding_attempt_deletion` 尾部——group attempt 删除后 `list_attempts_for_plan` 为空即接同一零成员闭包(command 确定性 `plan_workspace_close_{plan}_g{generation}`);失败保持 clearing 耐久、如实上抛、下一次产品动作续清。
   - 测试模块 `h2a_workspace_reset_tests`(复用共享夹具):T4 `h2a_last_member_delete_closes_old_round_completely`(partial 中间态+全删后 run/advance/journal 清、Plan/WI/revisions 保留、generation 2+回执+退役围栏、新 command 开新轮、同 command 幂等重放不清新轮)、T5 `h2a_initialization_prefix_is_not_reset_automatically`(前缀保护+显式关闭两态)、T6 `h2a_clearing_resume_and_command_exclusivity`(中断续清+异 command 409)。
4. **`src/web/handlers/support_task11_tests.rs`**(+21/-?,Main 授权):`plan_coding_n7_member_delete_invalidates_authorization` 改双成员 fixture——固定在设计行55 partial 分支(删其一→Invalidated 保留+迟到写拒),不触发最后成员闭包;最后成员新语义由 h2a 测试覆盖。
5. **`src/web/handlers/lifecycle_tests_n7.inc.rs`**(+20/-?,Main 授权):C 块改闭包语义断言(唯一真实成员删除→run 清+lifecycle generation 2),迟到写改用原内存 run 快照(仍 Err+不复活)。
6. **`cadence/reports/H2a-report.md`**:本报告。

### 跨包消费(按冻结接口,零并写)

- H2b(ImplH2b 冻结通报,双方回执在案):`AdvanceStore::plan_execution_references` + `AdvancePlanExecutionReferences::classify_stale_references`(分类)、`CodingAttemptStore::purge_group_initialization_journals_with_lease`(journal 清理,租约消费不重取 scope)。
- D1.2c 既有:`list_plan_artifacts`/`purge_plan_artifacts`(advance 清理,零改动);D2.2 既有:`cleanup_plan_shared_worktrees`/`collect_entity_deletion_manifest`(资源清理)。

## 证据(隔离树 /tmp/h2a-iso,HEAD 12102eb8+全部在途文件,5 个 conversational 在途坏文件与 review/routing.rs 回退 HEAD——主树同批文件编译错误非本包引入)

红窗方法:对 7 个新 API 入口(store 五方法+list_attempts_for_plan+闭包入口)打 `todo!("h2a red window")` 钝化后运行——全部测试以行为缺失 panic 失败(非编译失败),恢复后全绿。

| 命令(cargo test --locked --lib …) | 红(钝化) | 绿 |
|---|---|---|
| `h2a` | 0 passed / 6 failed(全部 `not yet implemented: h2a red window`) | 6 passed |
| `plan_coding_n2_` | — | 8 passed |
| `plan_coding_n7_`(含两处 Main 授权更新) | — | 4 passed |
| `finalize_coding_attempt_deletion`(含 t11 双成员更新) | — | 3 passed |
| `coding_attempt_store` | — | 208 passed |
| rustfmt(edition 2024,仅本包 5 文件)后复验 | — | `h2a` 6 / `plan_coding_n7_` 4 / `plan_coding_n2_` 8 passed |

## P2 修复轮(Main 审查派回,commit 见文末)

- **缺陷**:闭包 entry 生命周期判定为无锁读,锁内 `retire_workspace_round_with_lease` 返回值被丢弃——并发同 command(双最后成员钩子/H4 沿用同 command)在读→租约间交错时,陈旧「无 clearing」判定继续 ③-⑦ 破坏性清理(可清新轮);钩子 `expected_generation: None` 使 generation 守卫生产不生效。
- **修复**(按审查 suggestion):锁内接 retire 返回的 frozen 权威核验——clearing 缺失(同 command 回执已在)→Ok Reset 幂等返回、**不继续 ③-⑦**;installed.command_id≠本次→409 `coding_plan_workspace_clearing`(retire 已拒,防御性复核同拒);最后成员钩子传 `Some(generation)`。
- **证据**(隔离树,同批次回退 conversational/amendment 在途坏文件):新增 `h2a_concurrent_same_command_interleave_keeps_new_round` 先红(交错窗下新轮 run 被陈旧清理清除,`expect("new round run must survive...")` panic)→修复后 `h2a` 7 passed;回归 `plan_coding_n7_` 4 / `plan_coding_n2_` 8 / `finalize_coding_attempt_deletion` 3 / `coding_attempt_store` 208 全绿;rustfmt 后 `h2a` 7 复验绿。

主树状态:本包五文件 `cargo check --lib --tests` 零错误零警告(仅一条预存 `coding_store` 未用变量警告,HEAD 既有,非本包引入);主树整体编译被兄弟包 conversational_gate 系在途错误阻塞(E0308/E0425/E0596/E0382),故红绿在隔离树取证(设计预案「隔离树备用」)。

## 边界与移交

- 闭包不删 Planning 会话/Plan/WI/revision/source/compile/上游 Story/Design(测试断言 plan/lineage 保留);attempt 目录残留(attempt JSON 已亡的外部路径删除形态)不在本闭包清理面,归 H4 显式删除/成员删除路径。
- 生命周期元数据的 Plan 真删除纳入清理(删 `{plan_id}.json`+锁)归 D2.2 闭包消费方(H4/deletion.rs,设计 H4 行「真实Plan删除同时清新增生命周期元数据」)——**未实现**,需 H4 接线。
- `expected_generation` HTTP 参数/GET lifecycle DTO/DELETE 整组编排归 H4;advance/engine/run 真实入口围栏消费归 H3(store 级围栏与入口 API 已冻结可用)。
- 错误码 409 映射(`coding_plan_workspace_clearing`/`coding_plan_run_retired`/`coding_plan_workspace_generation_conflict`/`coding_plan_workspace_members_present`)归 H1 白名单(error.rs),本包只保证码原样透出。
- H2b 若调整 `classify_stale_references`/`purge_group_initialization_journals_with_lease` 签名,本包消费点集中在 `classify_plan_execution_references` 与闭包③步,替换面窄。
