# H4 报告：HTTP/DTO/端点合龙（组删端点与重启收敛）

- 设计：`cadence/designs/2026-10-14_方案设计_Coding工作区整组删除与重启_v1.0.md` C/D 节 + H4 行（E 节切片表）
- Worktree：`.worktrees/feat-b-0808-add-monorepo`（分支 feat-b-0808-add-monorepo）
- 状态：DONE（隔离树红→绿→回归全绿；主树被兄弟包 `workspace_ws_handler/run/*` 在途编译错误阻塞，按设计预案「隔离树备用」取证，见「验证环境」）
- 前置消费：H2a 闭包（d44c100f）`close_plan_coding_workspace_round`/生命周期 store；H2b（a0d0cf25）`plan_execution_references`/`classify_stale_references`；D2.3 `deletion_confirmation_required` 已铺面

## 交付

### 1. 组删端点（设计 C 节服务端接口）

- **路由**：`DELETE /api/projects/{p}/issues/{i}/work-item-plans/{plan_id}/coding-workspaces`（app.rs 新增；handlers/mod.rs、coding.rs 导出）
- **Query**：`command_id`（必填，稳定幂等键）、`expected_generation`（可选 u64 互证）、`confirmed=true`（可选；服务端分类才是权威——与 D2.3 同约定）
- **`delete_plan_coding_workspaces`**（plan_start.rs，编排顺序）：
  1. 删除准入（`plan_live_admission`：Plan 真删除互斥，闭包 own 本清理）＋组串行键（与 POST 同 `work_item_group:` 锁）
  2. **clearing 在案 → 以 `clearing.command_id` 记录身份续清**（删除动作是钩子中断态的产品续清出口；pending 冻结清单即回执 deleted 集；同 command 续清/异 command 拒由闭包承载）
  3. **同 command 回执 → Replayed**（幂等重放，不扫下一代刚创建的 attempts）
  4. 服务端三分类：无对象无残留→幂等成功零弹窗；成员在场→浅（全 Created 零 start_claim）/强（任一 status≠Created、有 start_claim，或 run 已冻结 selection/start_outcome）恰一次 409 `deletion_confirmation_required`（message 中文分级文案权威，details 带 level/member_count）；仅技术残留→浅级残留文案
  5. `expected_generation` 互证先行（≠当前 409 `coding_plan_workspace_generation_conflict`，零写——晚到删除不删新轮，confirmed 不跨 generation 授权）
  6. **逐成员复用 N7 清理链**（`teardown_plan_coding_member`：abort runner→mutation lease→Git 终止协调→组 arbitration→worktree 锁释放（含 owner 幂等释放）→workspace/branch→handoff revisions→`delete_attempt`→N7 `invalidate_for_deleted_attempt`→锁残留）——服务端一个受保护操作，非浏览器四次
  7. **H2a 零成员闭包收口**（本 command 记纯 ID 回执：generation+1、退役围栏、run/advance/journal/专属资源清，Plan/WI/revisions 保留）
- **回执四字段对齐 H5 冻结名（零改动对上）**：`operation_state: accepted|replayed`、`generation`、`deleted_attempt_ids`/`remaining_attempt_ids`、additive `workspace_lifecycle`（操作后投影）＋`plan_id`/`command_id`

### 2. GET progress 扩 workspace_lifecycle 投影（半残可观察）

- `PlanCodingProgressDto` additive `workspace_lifecycle: {generation, existing_member_count, missing_member_count, state}`（types.rs 新 `PlanWorkspaceLifecycleDto`）
- `plan_workspace_lifecycle_projection`（plan_start.rs）：引用集 = run 冻结集 ∪ Ready record 绑定 ∪（已越 AttemptPersisted 的）外层 journal/per-target journal 绑定，与真实存活集（`list_attempts_for_plan`）交叉核验——不从 attempt_status=aborted 猜物理存在性；未越前缀不计缺失。state 值域 `clean|complete|incomplete|clearing`（H5 冻结）
- GET/POST 共用 `build_plan_coding_progress` 均带投影

### 3. POST 开始入口半残自动收敛（设计 A 状态表/C 节 + H2a 复审硬约束）

- `converge_stale_plan_coding_round`（组串行键内、prepare 之前）：
  - **clearing 在案：以 `clearing.command_id` 身份续清**（Main 转达 H2a 复审硬约束：开始入口必须读 `clearing.command_id` 续清，否则钩子中断态无可达续清出口），完成后按请求正常新建
  - 零成员且旧执行引用已证断裂（AllTargetsDead 等价，`require_broken_proof=true`——正常初始化前缀不重置）→ H2a 零成员闭包完整重置，回执记在请求 command 下（同 command 重试返回/续做这一次操作）
  - **同 command 承接**（设计 C 行135）：现场恰有请求 command 的空 Prepared 前缀 run（无 claim/provider 事实）→ 先清前缀再闭包，避免闭包退役请求 command 后同动作自拒；原已 Started/Invalidated run 一律退役
  - 有成员现场零动作（N2/prepare 权威语义不变；存量半残无 lifecycle 元数据也能收敛——闭包在 retire 时建立元数据，设计行140）

### 4. 生命周期元数据纳入真 Plan 删除闭包（H2a 登记移交项）

- `CodingPlanRunStore::purge_workspace_lifecycle_with_lease`（coding_plan_run.rs，**H2a 文件最小加法**，H2a 已 DONE 闲置）：purge `{plan_id}.json`＋同目录稳定前导点锁 `.{plan_id}.json.lock`（workspace 重置闭包绝不删它，仅此安全删除窗口 unlink）；在位时先读校验身份（坏 JSON fail-closed）；缺失=已清
- deletion.rs 接线：run 本体 purge 之后调用；残留复核（⑨）增列 lifecycle 元数据（在位→`entity_deletion_residue` 拒绝记完成）

### 5. 错误面（H1 白名单补齐——H5 移交④落地）

- error.rs 409 白名单增五码：`coding_plan_workspace_clearing` / `coding_plan_workspace_generation_conflict` / `coding_plan_workspace_members_present` / `plan_coding_run_resource_busy` / `plan_coding_run_resource_unavailable`（此前落 500；H2a 透出码＋闭包资源占用的设计 B 节裁定）
- DELETE 自身的 generation 互证错误直构 `ApiError`（稳定码），不经 `product_store_api_error` 泛化

### 6. 测试（TDD 红先→绿）

- plan_start_tests.rs 新 h4 族 6 件（真实四仓 N3 夹具，handler 直调）：
  - `plan_coding_h4_group_delete_end_to_end_then_new_round`：验收矩阵 1——四仓首启→GET 事实→未确认 409（强文案）→确认整删（run/advance/journal 清、Plan/WI/revisions 保留、generation 2、回执四字段）→同 command Replayed→新 command 新轮（新 attempt IDs、Started）→晚到同 command Replayed 不扫新轮→过期 generation 409 零删
  - `plan_coding_h4_partial_delete_projection_and_remaining_cleanup`：矩阵 2——单删→incomplete/缺失计数→清剩余（deleted=剩余 3）→可重启
  - `plan_coding_h4_start_converges_half_dead_scene`：矩阵 3——旧 run/Ready/外层 journal 指向已亡全集→开始自动收敛（旧引用清、新 4 IDs 不复用、新轮 Intact、同 command 重放）
  - `plan_coding_h4_start_carries_same_command_prepared_prefix`：行135——空 Prepared 前缀同 command 承接，不自拒
  - `plan_coding_h4_clearing_resume_from_recorded_command`：组删入口以 `clearing.command_id` 续清（deleted=pending 冻结集）后新轮
  - `plan_coding_h4_start_resumes_clearing_before_new_round`：开始入口同上（H2a 硬约束双面）
- lifecycle_tests_n7.inc.rs 新 1 件：`plan_coding_h4_plan_delete_purges_workspace_lifecycle_metadata`（真 Plan 删除清本 plan 元数据+锁、他 plan 保留）
- it_web 路由冒烟 1 件（tests/it_web/web_lifecycle_api/part_07.rs）：路由接线＋无对象幂等成功零弹窗（回执字段形状断言）。**深行为归 h4 族**（真实四仓夹具在 plan_start_tests，不在 it_web 复制第二套夹具）

## 证据（隔离树 /tmp/h4-iso，HEAD d44c100f＋仅本包 12 文件）

红窗方法（H2a 同款）：对 4 个新入口（delete handler/converge/投影/purge_with_lease）打 `todo!("h4 red window")` 钝化后运行——全部以行为缺失 panic 失败（非编译失败），恢复后全绿。

| 命令（cargo test --locked …） | 红（钝化） | 绿/回归 |
|---|---|---|
| `--lib plan_coding_h4` | **0 passed / 7 failed**（全部 `not yet implemented: h4 red window`） | **7 passed / 0 failed** |
| `--lib plan_coding_n3_` | — | 7 passed |
| `--lib plan_coding_n7_` | — | 4 passed |
| `--lib h2a` | — | 6 passed |
| `--lib h2b` | — | 11 passed |
| `--lib finalize_coding_attempt_deletion` | — | 3 passed |
| `--lib coding_attempt_store` | — | 208 passed |
| `--lib`（全量） | — | **4672 passed / 0 failed / 7 ignored**（首轮 1 failed 未捕获到用例名，同代码复跑两次全绿，判定 flaky——记录方式沿 H5 P2 轮先例） |
| `--test it_web plan_coding_workspaces_delete_route` | — | 1 passed |
| `rustfmt --edition 2024 --check`（仅本包 12 文件，格式化后复验） | — | 干净 |

验证环境：主树开工 `git status` 对表——本包 12 文件零在途；红绿期间主树被兄弟包 `src/web/workspace_ws_handler/run/*` 在途编辑（E0425/E0308 系）阻塞 lib 编译，非本包引入（开工首检 `cargo check --lib` 曾通过，其后兄弟文件变更）；按设计预案在隔离树 /tmp/h4-iso（HEAD＋本包文件）取证，隔离树需复制 web/dist（rust-embed 编译期嵌入，干净 checkout 缺前端产物）。

## 边界与移交

- **H5 对齐（移交①—④收口）**：①回执字段名按 H5 冻结名原样落地（`deleted_attempt_ids`/`remaining_attempt_ids`/`operation_state: accepted|replayed`），**client+视图零改动即可对上**；DELETE 回执 additive 多带 `workspace_lifecycle`（H5 可选消费）②GET `workspace_lifecycle` 四字段与值域原样落地③`expected_generation` 定为 OPTIONAL（未知时省略，不阻断；已知而漂移→typed 409）④清理中/资源 busy 稳定码：`coding_plan_workspace_clearing`、`plan_coding_run_resource_busy/unavailable`、`coding_plan_workspace_generation_conflict`、`coding_plan_workspace_members_present`（全部 409）。**请 Main 转告 H5：无需反向改名。**
- `PlanCodingStartRequest` 未加 REQUIRED `expected_generation`（设计 C 行135 拟新增）：不在本包 brief 任务清单，POST 请求契约零变化；如需切换归 Main 另行调度（所有 caller/tests 同次切换）
- 组删的逐成员 teardown 不走 `finalize_coding_attempt_deletion`（其最后成员钩子以确定性命令闭包，会吞掉 DELETE command 的回执/续清身份）：N7 各步骤（abort/git/workspace/handoff/删除/失效/锁残留）按同序复用既有 helper；仓维/issue 维 shared worktree 清理**由 H2a 闭包 step④（`cleanup_plan_shared_worktrees`，D2.2 owner 复核语义）承担**，不在逐成员面重复
- v1 单 attempt 端点/确认/导航零改动；组删范围=服务端枚举的真实 group attempts，不收前端 ID 列表
- error.rs（H1 面）与 coding_plan_run.rs（H2a 面）各一处最小加法（五码白名单/purge 方法），已在报告登记，供 H1/H1 后续 owner 知悉

## 四字段

- **状态**：DONE
- **提交**：`fix: H4 组删端点与重启收敛`（12 文件显式 add）
- **验证**：红 0/7→绿 7/0→回归六族全绿→全量 lib **4672/0**→it_web 路由 1/0→rustfmt（限本包文件）干净；P2 修复轮后 h4 族 7/0＋回归族复跑全绿（见下节）
- **关切**：①主树整体编译被兄弟在途文件阻塞，本包主树红绿待 Main 收口后统一回归（隔离树证据完整）②error.rs/coding_plan_run.rs 跨面最小加法两处（登记）③POST REQUIRED `expected_generation` 契约切换未做（不在 brief）④验收矩阵 8 浏览器真实路径归 Main 统一调度

## P2 修复轮（Main 审查 1×P2，按建议精确落）

两处均在 `delete_plan_coding_workspaces`（plan_start.rs）：

- **P2-① remaining 硬编码空集**：回执 `remaining_attempt_ids` 改为服务端
  枚举当前存活集（`list_attempts_for_plan`，排序去重）——设计 C「前端不
  假装全删/失败展示剩余数」；Replayed 于新轮成员在场时亦如实报告（新内容
  非本 command 范围，不删不瞒）。
- **P2-② Replayed 不限轮次**：同 command 回执仅当 `to_generation == 当前
  generation` 才判 replayed；更旧回执（其后又经历轮次切换）不按旧 command
  判重放，按当前状态走分类路径（新成员按新操作门禁/generation 互证先行，
  不删新轮、返回当前事实）。顺带消除 else 分支内重复的 generation 绑定。
- 测试同步（e2e）：`late_same` 断言补 `generation==2`＋`remaining==round2_ids`
  （新轮 4 成员在场）；新增「更旧回执 fall-through」段——round2 再整删
  （generation 3）后，h4_1 的回执（to=2）过期→accepted 幂等成功（零成员零
  残留）、deleted/remaining 空、generation 3。

证据（隔离树 /tmp/h4-iso＝`d1d862f9`＋本轮 2 文件）：`plan_coding_h4` **7/0**
→回归族 `plan_coding_n3_` 7/0、`plan_coding_n7_` 4/0、`h2a` 6/0、`h2b` 11/0、
`finalize_coding_attempt_deletion` 3/0、it_web 路由 1/0→rustfmt（2 文件）干净。
其间一次 lib 编译失败为本轮编辑中间态（`operation_state` 声明被覆盖，立即
修复后全绿），非交付缺陷。

提交：`fix: H4 回执按轮次限定Replayed与真实remaining`（3 文件显式 add：
plan_start.rs＋plan_start_tests.rs＋本报告）。

## P1 修复轮（Main 复审 1×P1，按 suggestion 精确落）

P1（P2 轮 fall-through 引入）：陈旧回执的旧 command（to_generation < 当前）
+ `expected_generation` 缺省 + `confirmed=true` + 新轮成员在场 → 走完整破坏
路径**整删新轮**——设计 C「confirmed=true 不是跨 generation 授权」被破。
修（plan_start.rs `!members.is_empty() || residue` 分支内）：本 command 存在
旧轮回执（`to_generation < 当前`）即与 `expected_generation` 互证**同返 409
`coding_plan_workspace_generation_conflict`**（零写拒绝，不进破坏路径）；
零成员 fall-through（清残留/幂等收尾）不进本分支，与既有测试相容。

TDD：红＝e2e 尾部新增段——round3 整删（generation 3）后开 round4（4 成员），
旧 command `plan_delete_ui_h4_1`（回执 to=2）+无 generation+confirmed → 现实现
**整删 round4**（回执证据：deleted=4 新成员，正是 P1 危害）→ 断言 409 拒绝
panic；绿＝修复后 409 + round4 成员保留。

证据（隔离树＝`e69f6d65`+本轮 2 文件）：红 1 failed（cross-generation 断言）
→绿 `plan_coding_h4` **7/0**→回归族 n3 7/n7 4/h2a 7/h2b 11/finalize 3/
it_web 路由 1 全绿→rustfmt（plan_start.rs）干净。

登记（不修，Main 裁定）：H5 UI 未消费 `operation_state`/`remaining` 即
`onBack`——服务端语义已对，UI 消费补齐归 H5 收尾轮（验收后）。

提交：`fix: H4 陈旧回执不构成跨代授权`（3 文件显式 add：plan_start.rs＋
plan_start_tests.rs＋本报告）。
