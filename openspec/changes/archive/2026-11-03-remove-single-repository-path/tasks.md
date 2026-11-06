# Tasks: remove-single-repository-path

> **收口注记（2026-11-03）**：本 change 已于 2026-11-03 归档，delta 已同步进主 specs
> （5 份 capability；**单仓旧语义 5 条 requirement 已 REMOVED**，同 ID 新版本已就位 ——
> `REQ-ROUTE-01`→两态分流状态机 · `REQ-ROUTE-02`→全入口对齐（无单仓分支）·
> `REQ-ROUTE-03`→无逻辑归属不回退物理仓 · `REQ-ROUTE-C5-TARGET`→自动化 target 的载体判定 ·
> `REQ-MRE-02` 单仓零回归整条移除；`validate --specs` 78 passed / 0 failed）。
> 勾选口径与逐条文件:行级证据见 `cadence/notes/2026-11-03_收口_行数登记表改动清单_v1.0.md` §M.1。
>
> **剩余未勾 1 条及原因**（2026-11-04 补记：**5.2** 已于 2026-11-04 内置浏览器真机实测并勾选，不再计入未勾；证据 `cadence/reports/2026-11-04_验收报告_浏览器实测与API直连_v1.0.md` §2.1）：
> · **11.4** —— 真机验收 —— **⏳ 待实做（Main 真机验收批次：受控环境操控 + 浏览器实测；逐条做、逐条勾、逐条留证据）**
>   ⇒ 本机四家 provider（`pi` / `codex` / `kimi` / `claude`）+ `codegraph` **均已安装**（`command -v` 实测：前四家在 `/home/michaelche/.local/share/fnm/aliases/default/bin/` 与 `/home/michaelche/.kimi-code/bin/`，`codegraph` 同 fnm 路径、全局包 `@colbymchenry/codegraph@1.6.2`）
>   ⇒ **不是环境不具备，是尚未执行**；下列 1 条**全部可做**
>   逐条怎么做：
>   · **5.2** —— ✅ 2026-11-04 内置浏览器真机实测并勾选（不再计入未勾；原做法留存备查：改造后的「添加代码库」对话框即「建逻辑代码库」入口，**不新造页面** ⇒ **浏览器实测**：打开「添加代码库」对话框 → 确认其即建 LC 入口（页面无新增独立入口））
>   · **11.4** 真机验收六项 ⇒ **浏览器实测 + 本地造 1 成员与 3 成员 LC**，逐项过：
>     ① **新建项目直接多仓**；② **1 成员 LC 全链通过**；③ **3 成员 LC 仓序正确**；
>     ④ **六条布局规则各给动作**；⑤ **一键核验 4 家并发出进度、失败可单项重试**；⑥ **核验后重试聚合初始化**跑通

> owner 决策已全部拍板；技术方案见 `cadence/designs/2026-11-02_技术方案_块3_去单仓入口与统一多仓流程_v1.0.md`（owner 2026-11-02 审阅通过，2026-11-03 补范围澄清与票3 异步决策）。
> 精确文件、命令、测试与提交步骤由 `writing-plans` 写入 `cadence/plans/`；本文件只记高层工作包，每个工作包映射到相关 requirement。
> **红线**：不删任何 legacy 布局（`logical-codebase/` 目录、`migrate_legacy`、旧别名仓双路扫描、布局冲突判定）；不写「1 成员跳过某步骤」分支；不引入技术方案之外的方案。

## 跨块同文件有序串行登记（块3 ↔ 块1，5 个文件）

> **这不是 task 组，是跨块并行的顺序约束登记** —— 块3 与块1 的实施 Plan 有一部分**改同一个文件**；这些文件必须**按表定顺序串行**，否则两块的提交会撞 Git。
> 登记来源：2026-11-03 双审（事实错误与漏登）处置；与两份实施 Plan 一致（块3 Plan §4.1「跨块文件交集」、块1 Plan「并行安全与跨块协调」节）。

| # | 文件 | 顺序（先 → 后） | 两块各改什么 | 性质与处置 |
|---|---|---|---|---|
| 1 | `web/src/components/lifecycle/WorkItemPlanOptionsDialog.tsx` | **块1 片4 → 块3 片3** | 块1 片4：`:404-408` 文案面（「重新开启后关闭再开启生效」→「换 provider 会先删除现有工作项组」）+ `SubmitErrorState` 加 `code`；块3 片3：`enrollmentTargetLabel` 的 `case "single_repository"`（`:533-534`）渲染保留 + Enable 提交路径改用 `carrierTargetFromEnrollmentTarget`（为 `null` 时禁用提交 + 提示） | 同文件**不同位置**、语义分离 ⇒ **必须串行**；块3 片3 **不得**删 `SubmitErrorState.code` / `errorCode` 分支，块1 片4 **不得**删 `single_repository` 分支 |
| 2 | `web/src/components/lifecycle/WorkItemPlanOptionsDialog.reenable.test.tsx` | **块1 片4 → 块3 片3** | 块1 片4：改写 `:195` 用例 + 新增「主文案 + 次要行机器码」用例；块3 片3：`:36`/`:80` 夹具保留 + 新增「`single_repository` 终止记录 → 重新开启禁用 + 提示」用例 | 同文件**不同用例** ⇒ **必须串行**；后落方**只加不删**先落方的用例 |
| 3 | `web/src/components/lifecycle/useIssueLifecycleGeneration.reenable.test.ts` | **块1 片4 → 块3 片3** | 块1 片4：改写 `:603`/`:643`/`:661`（三步塌缩为一次提交）；块3 片3：`:88`/`:159`/`:523` 夹具保留（按该用例是否走提交决定保留 legacy 或改 `logical_codebase`） | 同文件**不同用例** ⇒ **必须串行**；后落方**只加不删**先落方的用例 |
| 4 | `src/web/handlers/automation_enrollment.rs` | **块3 片1 → 块1 片3** | 块3 片1：删 `:384` 的 `AutomationCarrierResolution::SingleRepository` 载体判定臂（Enable / prepare / Enrolled advance）；块1 片3：只改 `:582-591` 的 `enrollment_api_error`（按 kind 分流；如确需可改 `:20-25` 的 `enrollment_conflict` helper） | 同文件**互不相干的位置**（非语义冲突）⇒ 属 **Git 级同文件并发写**，**必须串行**；块1 片3 **MUST NOT** 动 `:384` 的载体判定 |
| 5 | `src/web/error.rs` | **块3 片1 → 块1 片3** | 块3 片1：新增稳定码 `issue_not_bound_to_logical_codebase` → 400 BAD_REQUEST（稳定码→状态表 `:160-215`）；块1 片3：新增两条稳定码（运行态限制 / 版本过期）→ 409 | 同文件**不同码表条目**（非语义冲突）⇒ 属 **Git 级同文件并发写**，**必须串行**；后落方不得删改先落方的码条目 |

**登记规则（两块一致）**：① 表中「先落方」必须**独立提交**落地后，「后落方」才可起工，且开工前须 rebase 到先落方提交之上；② 后落方**不得**删改先落方在该文件里的行，合并冲突时**手工保留双方行**；③ #4 #5 改的是同一文件里**互不相干的位置**，不是语义冲突，但同文件并发写会撞 Git ⇒ 仍按串行处理。

**明确不构成交集、不要误登记的文件**：`WorkItemPlanOptionsDialog.plan-approval.test.tsx`、`WorkItemPlanOptionsDialog.target.test.tsx`、`useIssueLifecycleGeneration.plan-approval.test.ts` 三份在**块1 片4 的禁碰清单**里（块1 侧零改动，块3 片3 单方独占）⇒ **不构成跨块交集，不需要定序**。

## 1. 路由三态 → 两态，`Legacy` 变体整个移除

映射 requirement：`REQ-ROUTE-01`

- [x] 1.1 `RepositoryRouting` 移除 `Legacy` 变体，只留 `Logical` / `FailClosed`；全部 match 穷尽化
- [x] 1.2 `resolve_issue_logical_codebase_id` 返回 `String`（issue 不存在仍 `NotFound`），删除 `None` 分支
- [x] 1.3 `issue_codebase_stores` 只接受 `Some(lc_id)`，删除 project 级 store 的 `None` 回退
- [x] 1.4 原 `(None, None)` 判定改为落 `FailClosed`（稳定错误码），**不得**回退物理仓
- [x] 1.5 **红线核对**：`migrate_legacy`、`logical_codebase_root` 及其下路径、`dangling_reference/patrol/lc_partition.rs` 的别名仓双路扫描、`repository_routing_resolver.inc.rs` 的布局冲突判定**零改动**；逐处核实留下的 `legacy` 命名代码属布局而非单仓语义
- [x] 1.6 用例：`(Some, Some)`→`Logical`；各类不一致→`FailClosed` + 稳定错误码；`(None, None)`→`FailClosed`（不再有 Legacy 臂）

## 2. issue 归属字段收敛与自动化载体判定

映射 requirement：`REQ-ROUTE-03`、`REQ-ROUTE-C5-TARGET`

- [x] 2.1 issue 记录的 `logical_codebase_id` 从 `Option<String>` 改为 `String`；**不做**数据迁移脚本
- [x] 2.2 老记录（字段缺失）反序列化返回明确错误 `issue_not_bound_to_logical_codebase`，**不得**补默认值、不得推导为单仓
- [x] 2.3 自动化 target 投影 / Enable / prepare / Enrolled advance / 自动 StartCoding 的载体判定只剩 LC 载体；删除单仓载体分支与「无 LC 归属→单仓」判定
- [x] 2.4 该错误码的中文文案接入失败面单一来源（块2 `REQ-BOOT-08`）
- [x] 2.5 用例：无归属 issue 的投影与 Enable 返回明确 4xx 且不写 enrollment；有归属但成员不一致仍 fail-closed；老记录报错而非静默回退

## 3. 生产 32 处 / 19 文件的 `Legacy` 出现点逐处改写与全部入口对齐（含测试文件则 34 处）

映射 requirement：`REQ-ROUTE-01`、`REQ-ROUTE-02`

- [x] 3.1 以 `grep -rn "RepositoryRouting::Legacy" src/ web/src/ --include=*.rs --include=*.ts --include=*.tsx | grep -v '_tests' | grep -v 'tests\.inc' | grep -v '/tests/' | grep -v '\.test\.'` 为清单逐处改写 —— **命令自带排除测试文件的条件，照抄即得生产 32 处、分布 19 个文件**（其中 **26 个匹配臂消费 + 6 个构造点产生**；不排除测试文件则为 34 处）：`Legacy` 臂删除，原逻辑**合并进 `Logical` 臂或改为 `FailClosed`**，不新增第三种形态。**清单含 2 处 `#[cfg(test)]` 内的命中**（`src/product/coding_attempt_store/group_validation.rs:695`、`src/web/handlers/support_parts/coding_cleanup.inc.rs:171`）—— 按文件名过滤会一并列出；这两处**必须一并处置**（改写为 `Logical`/`FailClosed`，或删除该测试/夹具），**处置方式由实施者决定，两种做法都不违反契约**（详见段末附注）
- [x] 3.2 入口逐个对齐：`workspace_repository_for_session`、`coding/group.rs`、`coding.rs` 删除路径、`coding_ws_handler/context.rs`、`resolve_work_item_repository`、`coding_evaluation_context/builder.rs`、`socket.rs`、`compile_support.rs`
- [x] 3.3 删除「issue 创建时校验单仓 `repo_id` 物理存在性」这一前置门
- [x] 3.4 **前端 enrollment 载体收窄（方案 A：只收窄发起侧 + legacy 投影保留；原措辞「删除 `single_repository` 类型变体及其消费分支」已按块3 Plan §6 R19 更正，不得再按原措辞执行）**：**保留** plan / cockpit 侧 `single_repository` 的**类型变体**与**只读渲染** —— `EnrollmentTarget`（`web/src/api/types/lifecycle.ts`）三变体不删；`WorkItemPlanOptionsDialog.tsx` 的 `enrollmentTargetLabel` 与 `web/src/state/workspace-cockpit-inbox-items.ts` 的 `c1TargetLabel` 的 `case "single_repository"` 渲染分支照旧输出「单仓 repo_x」，**MUST NOT** 退化为「target 未知」；对应 legacy 展示用例（`WorkItemPlanOptionsDialog.target.test.tsx`、`workspace-cockpit-projection.inbox.test.ts`）一并保留；**只在发起侧收窄** —— `web/src/api/types/lifecycle.ts` 新增 `EnrollmentCarrierTarget`（`logical_codebase` / `logical_codebase_members` 两变体）供 Enable / Rebind 命令使用，`web/src/components/lifecycle/automation-reenable.ts` 新增守卫 `carrierTargetFromEnrollmentTarget`（`single_repository` → `null`），`WorkItemPlanOptionsDialog.tsx` 的 Enable / Rebind 提交路径按该守卫判定：为 `null` ⇒ **禁用提交 + 提示「需重建为逻辑代码库授权」**，**不得**静默降级、**不得**猜 LC；**照删**的是 Rust 侧载体判定 `AutomationCarrierResolution::SingleRepository`（见 2.3 与上文跨块串行登记 #4，本任务不重复实施）；**禁删** `EnrollmentTargetDto.single_repository`（`web/src/api/types/work-item-plan.ts:678-689`）、`EnrollmentTarget::SingleRepository`（`src/product/logical_codebase/types.rs:31`）、`RepositoryTargetKind::SingleRepo`（`src/product/logical_codebase/repository_routing_types.inc.rs:151-152`）—— 依据 `openspec/specs/work-item-group-structure/spec.md:14` 的多仓候选声明合同（`intent_target_kind: single_repository`）。依据：块3 Plan §2.3.1 第 3 条 / §2.3.2 / §2.3.3 / §6 R19
- [x] 3.5 用例：各入口对同一状态结果一致；无归属一律 fail-closed 且不返回 200；多仓既有行为逐条回归不变

**3.1 附注一（枚举定义与两个口径的处数分布）**：`RepositoryRouting::Legacy` 的枚举定义在 `src/product/logical_codebase/repository_routing_types.inc.rs:47`（`Legacy { repository_id: String }`）。`Legacy` 出现点有两个口径 —— ① **按文件名排除测试文件**（即 3.1 的命令）：**32 处 / 19 个文件**，其中 **26 个匹配臂消费 + 6 个构造点产生**（含测试文件则 34 处）；② **按 `#[cfg(test)]` 精确切分**（真正编进生产二进制的部分）：**30 处 / 19 个文件**，其中 **25 个匹配臂 + 5 个构造点**。两口径差 **2 处**：`src/product/coding_attempt_store/group_validation.rs:695`（构造点，在该文件 `:640` 起的 `#[cfg(test)] mod tests` 内）与 `src/web/handlers/support_parts/coding_cleanup.inc.rs:171`（匹配臂，在 `:161` 的 `#[cfg(test)] fn cleanup_shared_worktree_by_routing` 内）—— 这两个文件都另有生产命中，故**两个口径的文件数都是 19**。

**3.1 附注二（实施范围：32 处都是必处置项）**：3.1 的命令给出的是**必须逐处处置的完整清单（32 处）** —— `Legacy` 变体整体移除后，**任何残留的 `RepositoryRouting::Legacy` 引用都会编译不过**，**`#[cfg(test)]` 代码也不例外**。因此上面 2 处 test-only 命中**必须一并处置**：可选择**改写为 `Logical` / `FailClosed`**，或**删除该测试 / 夹具**；**两种做法都不违反本 change 的契约**，但**不得留下 `Legacy` 引用**。

**3.1 附注三（与技术方案 §1.2 的对照）**：技术方案 §1.2 **已按实测更正**为「**32 处出现点**（其中 **26 个匹配臂消费、6 个构造点产生**），分布在 **19 个文件**」，并补了**两口径对照段**（口径① 按文件名排除测试文件 = **32 处 / 19 个文件** = 改写清单口径；口径② 按 `#[cfg(test)]` 精确切分 = **30 处 / 19 个文件**）；**本 change 与技术方案口径一致**，实施一律以 3.1 的清单命令为准（照抄即得 32 处 / 19 文件）。

## 4. 前端单仓入口删除

映射 requirement：`REQ-REG-16`、`REQ-MRE-01`

- [x] 4.1 `AddCodebaseDialog.tsx` 删「单仓库」radio 与「继续添加单仓库」按钮；默认态改为多仓
- [x] 4.2 `CreateLifecycleIssueDialog.tsx` 删「单仓库代码库」选项；提交必须携带 LC
- [x] 4.3 同步改写钉住单仓路径的测试（`AddCodebaseDialog.test.tsx`、`CreateLifecycleIssueDialog.test.tsx` 等），**不得**只改组件不改测试
- [x] 4.4 用例：对话框无单仓选项；无 `logical_codebase_id: null` 的提交路径；缺 LC 提交被稳定错误码拒绝

## 5. 新建项目直接多仓，「建逻辑代码库」为独立步骤

映射 requirement：`REQ-REG-16`

- [x] 5.1 新建项目流程固定为 创建项目 → 建 LC → 建 issue；界面不得出现单仓选项
- [x] 5.2 确认改造后的既有「添加代码库」对话框即「建逻辑代码库」入口（真机走一遍），**不新造页面**
  - ✅ 2026-11-04 内置浏览器真机实测（API 4319 独立实例 + vite 5173）：「添加代码库」对话框标题仍是「添加代码库」（**未新造页面**）、说明为「以聚合根目录组织多个成员仓库，创建后在本弹窗内登记成员（自动发现）」、按钮文案即「**创建逻辑代码库**」；`单仓库|单仓|单个仓|单代码库|继续添加单` 全文正则**零命中**、无 radio ⇒ 单仓入口确已删除。证据：`cadence/reports/2026-11-04_验收报告_浏览器实测与API直连_v1.0.md` §2.1
- [x] 5.3 建 LC 的独立步骤承载 聚合根选择 + 成员登记 + 聚合初始化；**不得**塞进创建项目表单
- [x] 5.4 用例：全新项目直接多仓；未建 LC 时无法创建 issue；建 LC 步骤可独立进入并完成登记

## 6. 1 成员与 N 成员走完全同一套路径

映射 requirement：`REQ-REG-17`

- [x] 6.1 **实施第一步**：grep 成员数判断（`members.len() == 1`、`member_ids.len()`、`member_count`、`require_multiple_members` 之类），区分生产分支与测试夹具断言，列出清单
- [x] 6.2 移除全部「至少 2 个成员」类门禁（owner 裁定不禁止 1 成员 LC）
- [x] 6.3 删除任何「成员数为 1」的生产分支；多仓机制按自然退化（零条自动边 / 单成员索引 / 单前缀）而非「跳过检查」
- [x] 6.4 源码级断言：不存在 `members.len() == 1` 专门分支；不存在拒绝 1 成员 LC 的门禁（防回归）
- [x] 6.5 用例：1 成员 LC 完成首建全链（聚合初始化、索引、编码、验收）；仓序自动边零条；成员数 1↔N 变化无需特殊流程

## 7. 布局规则表、目录树示意与失败面动作

映射 requirement：`REQ-LAYOUT-01`、`REQ-LAYOUT-02`、`REQ-LAYOUT-04`、`REQ-LAYOUT-05`（文案依赖块2 `REQ-BOOT-08` / `REQ-BOOT-10` / `REQ-BOOT-11` / `REQ-BOOT-12`）

- [x] 7.1 引用块2 `failure_copy.rs` 的共享规则与文案结果（六条规则 → 七个对外稳定码 → 「用户该怎么摆」）作为本能力实现与验收的共同来源；MUST NOT 在块3 自建规则表或文案表
- [x] 7.2 多仓创建/登记入口与聚合初始化卡片加目录树示意 + 「`.worktrees/` 下的代码不可被索引」说明
- [x] 7.3 七个对外稳定码（`aggregate_root_is_git` / `aggregate_root_member_outside_root` / `aggregate_root_member_symlink_escape` / `aggregate_root_nested_worktree` / `aggregate_root_overlap` / `aggregate_index_layout_unsupported` / `aggregate_index_member_coverage_failed`）逐一接入三段式失败面：问题 / 原因 / 怎么做；内部码 `member_path_outside_root` / `member_symlink_escape` / `nested_worktree` 只作反查
- [x] 7.4 布局失败面只渲染块2 `REQ-BOOT-10` / `REQ-BOOT-11` 登记的 `failure_copy.rs` 成品（`title_zh`/`reason_zh`/`action_zh`/`triage`/`machine_line` 五字段必填），**不另抄一份**；本片显式依赖块2 片2 交付，块2 未落地时先落映射并标注依赖，MUST NOT 在块3 另建文案来源
- [x] 7.5 用例：每个错误码的失败面都有对应动作文案（逐条断言）；入口展示目录树

## 8. 入口的依赖前置提示（codegraph）

映射 requirement：`REQ-LAYOUT-03`

- [x] 8.1 `codegraph_missing` / `codegraph_version_mismatch` 进入口依赖前置清单，文案与 `REQ-BOOT-08` / `REQ-BOOT-10` / `REQ-BOOT-11` / `REQ-BOOT-12` 同源
- [x] 8.2 安装/更新入口复用块2 `provider-cli-installation`（`REQ-PCI-02` 按钮显示规则、`REQ-PCI-05` 异步作业与装完重新探测）；**不得**自带第二套安装实现或文案
- [x] 8.3 用例：缺失给【安装】、版本不符给【更新】并显示当前 vs 要求版本；点击走块2 的作业

## 9. 票3：批量核验的异步作业（后端）

映射 requirement：`REQ-PCR-03`

- [x] 9.1 新增 `POST .../capabilities/revalidate-all` → 立即返回 `job_id`；`GET .../capabilities/revalidate-all/{job_id}` → 作业状态 + 逐家结果
- [x] 9.2 四家全并发执行，**直接复用**既有单家 `revalidate`（不改其内部语义，继承幂等短路与 CLI 缺失快速失败）
- [x] 9.3 单家失败不阻断其它家；`skipped` 带稳定原因（如 `cli_not_installed`）
- [x] 9.4 作业状态 durable（job + 每家状态），刷新后可按 `job_id` 继续；终态时未完成项标超时
- [x] 9.5 用例：发起即返回（不阻塞）；并发而非串行（时间戳断言）；单家失败不影响其它家；已核验家秒回；CLI 缺失标 skipped；超时如实标注

## 10. 票3：一键按钮与逐项进度（前端）

映射 requirement：`REQ-PCR-04`

- [x] 10.1 能力卡加「一键核验全部」按钮，发起后 2~3 秒轮询，终态停止
- [x] 10.2 进行中禁用按钮并显示「核验中（N/4）」；逐项状态（已确认 / 核验中 / 失败 / 跳过）实时更新
- [x] 10.3 失败项每行保留「重试」，走既有单 provider 端点；`skipped` 项引导【安装】、版本落后项引导【更新】（复用块2 `REQ-PCI-02`/`REQ-PCI-05`）
- [x] 10.4 按钮区域说明「逐 LC 核验」「会真实跑一次探测、可能几分钟」
- [x] 10.5 用例：一键发起与进度渲染；刷新后按 `job_id` 恢复；失败项单独重试不重跑整批；终态停止轮询

## 11. 回归与门禁

映射 requirement：全部（`REQ-ROUTE-01/02/03/C5-TARGET`、`REQ-REG-16/17`、`REQ-LAYOUT-01/02/03`、`REQ-PCR-03/04`、`REQ-MRE-01`）

- [x] 11.1 多仓全流程逐条回归（冷启动、索引、group coding、跨仓验收）行为逐字不变
- [x] 11.2 单测全绿 + 行数门禁（`large_file_guard`）绿 + 隔离与纪律测试绿
- [x] 11.3 前端 vitest 全绿（含删除的单仓相关测试）；`build` 通过
- [x] 11.4 真机验收：新建项目直接多仓；1 成员 LC 全链通过；3 成员 LC 仓序正确；六条布局规则各给动作；一键核验 4 家并发出进度、失败可单项重试；核验后重试聚合初始化跑通
      ✅ 2026-11-07 owner 真机实测通过（六项逐项过，owner 回报并授权勾选；owner 侧操作端口 5173 开发期 / 4317 后端）。★证据文件待补★ —— 本行证据来自 owner 回报，未附截图/日志文件；owner 提供后再补链接。
- [x] 11.5 本 change 的 proposal / design / 五份 spec delta / tasks 自检 + `openspec validate remove-single-repository-path --strict`
