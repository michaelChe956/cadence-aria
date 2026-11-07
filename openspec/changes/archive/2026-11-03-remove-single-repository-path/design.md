# 设计：去单仓入口 + 统一多仓流程

> 完整技术方案见 `cadence/designs/2026-11-02_技术方案_块3_去单仓入口与统一多仓流程_v1.0.md`（owner 2026-11-02 审阅通过，2026-11-03 补范围澄清与票3 异步决策）。
> 本文件是 OpenSpec 层的**决策记录**：背景 / 决定 / 理由 / 代价。落点与验收标准见技术方案与 `tasks.md`。
> 标注「（实测）」的代码事实为本 worktree 亲自读取/检索的结果；标注「（技术方案）」的为技术方案给定、本轮未逐行复核的事实。

## 决策 0：只删「单仓语义」，legacy 布局一个字不动

**背景**：技术方案 §1 的范围澄清指出「legacy」在两处指的不是同一件事。

| | 名字里的 legacy 指 | 处置 |
|---|---|---|
`RepositoryRouting::Legacy` 变体 | **单仓语义** —— issue 未持久归属 LC 时回退其 `repo_id` 指向的物理仓 | **删** |
`logical-codebase/` **磁盘目录**、`migrate_legacy`、旧别名仓双路扫描、布局冲突判定 | **布局版本** —— v1.2 的 project 级布局，v1.3 之后仍是**多仓数据的物理落点** | **保留，一个字不动** |

（实测）`src/product/logical_codebase/repository_routing_types.inc.rs:37-60`：`Legacy { repository_id }` 的文档注释自己写着「`Legacy` 即**单仓（single-repository）**语义 …… 命名保留（不改名），它描述的是先于逻辑代码库分流存在的物理仓解析路径」。

**决定**：本 change 只删单仓语义路径；`logical_codebase_root` 及其下所有路径、`migrate_legacy`、别名仓双路扫描、布局冲突判定全部零改动。实施时若某处「legacy」相关代码属于**单仓语义**（而非布局兼容），才纳入 `Legacy` 消费点改写清单 —— **逐处核实，不按名字猜**。

**理由**：
- （技术方案）`src/product/app_paths.rs:75-178` 有十几个生产路径基于 `logical_codebase_root`（`aggregate_indexes_root` / `aggregate_policy_artifact_path` / `aggregate_initializations_root` / `aggregate_recipe_receipts_root` / `provider_trust_root` / `registration_preflights_root` / `registration_batches_root` / 各 lock 路径），`coding_attempt_store/paths.rs:130-149` 的 pointer-publications 也在其下 → **删掉该目录 = 删掉当前所有多仓数据**
- `logical_codebase/store_purge.inc.rs:183-188` 记录了自愈机制（技术方案）：旧根还在而墓碑已删时，下一次 `migrate_legacy` 会把 LC 复活 —— 说明该函数是**布局版本迁移**，与单仓语义无关
- owner 2026-11-03：**「保证多仓流程是没问题和现在保持一致的」** —— 布局是数据落点，动它就是动多仓

**代价**：仓库里继续存在名字带 legacy 但与单仓无关的代码（`migrate_legacy`、别名仓双路扫描等）。这是刻意保留的**命名债**，用注释与 spec 说清语义，不用删除换「整洁」。

**明确不做**：不因「反正叫 legacy」而顺手删除任何布局相关代码或迁移函数。

## 决策 1：路由三态 → 两态，`Legacy` 变体整个移除

**背景**：（实测）`RepositoryRouting` 现为三态 —— `Legacy { repository_id }`（`:47`）、`Logical { manifest, selection }`（`:48-51`）、`FailClosed { code, reason }`（`:52-55`）；`resolve_issue_logical_codebase_id`（`:63-77`）返回 `Option<String>`，`None` 同时承载「单仓」与「旧数据」；`issue_codebase_stores`（`:79-95`）接受 `lc_id: Option<&str>` 并把 `None` 映射到 project 级 store。（实测）`RepositoryRouting::Legacy` 在生产代码有 **32 处出现点**（其中 **26 个匹配臂消费、6 个构造点产生**），分布在 **19 个文件**；清单命令自带排除测试文件的条件，照抄即得该口径：`grep -rn "RepositoryRouting::Legacy" src/ web/src/ --include=*.rs --include=*.ts --include=*.tsx | grep -v '_tests' | grep -v 'tests\.inc' | grep -v '/tests/' | grep -v '\.test\.'`；集中在 `workspace_repository.rs`(4)、`web/handlers/lifecycle.rs`(3)、`web/handlers/coding/group.rs`(3)、`coding_attempt_store/group_validation.rs`(3)、`coding_attempt_store/admission.rs`(3)、`web/handlers/support_parts/coding_cleanup.inc.rs`(2)、`coding_attempt_repository.rs`(2)，其余 12 个文件各 1 处（含测试文件计则 34 处）。

**口径说明**（与本 change 的 `tasks.md` 段 3、`proposal.md` 同源）：`Legacy` 出现点有两个口径 —— ① **按文件名排除测试文件**（即上面那条清单命令，**改写清单以此为准**）：**32 处 / 19 个文件**（26 个匹配臂消费 + 6 个构造点产生；含测试文件则 34 处）；② **按 `#[cfg(test)]` 精确切分**（真正编进生产二进制的部分）：**30 处 / 19 个文件**（25 个匹配臂 + 5 个构造点）。两口径差 **2 处**：`src/product/coding_attempt_store/group_validation.rs:695`（构造点，在该文件 `:640` 起的 `#[cfg(test)] mod tests` 内）与 `src/web/handlers/support_parts/coding_cleanup.inc.rs:171`（匹配臂，在 `:161` 的 `#[cfg(test)] fn cleanup_shared_worktree_by_routing` 内）—— 这两个文件**都另有生产命中**（`group_validation.rs:265`/`:578`、`coding_cleanup.inc.rs:222`），故**没有任何文件整体退出** ⇒ **两个口径的文件数都是 19**。

（与技术方案 §1.2 的对照）：技术方案 §1.2 **已按实测更正**为「**32 处出现点**（其中 **26 个匹配臂消费、6 个构造点产生**），分布在 **19 个文件**」，并补了**两口径对照段**（口径① 按文件名排除测试文件 = **32 处 / 19 个文件** = 改写清单口径；口径② 按 `#[cfg(test)]` 精确切分 = **30 处 / 19 个文件**）；**本 change 与技术方案口径一致**，实施一律以上面那条清单命令为准（照抄即得 32 处 / 19 文件）。

**决定**：
1. `RepositoryRouting` 只留 `Logical` 与 `FailClosed` 两个变体；`Legacy` **整个移除**，不留「永不出现但保留」的臂
2. `resolve_issue_logical_codebase_id` 返回 `String`（issue 不存在时仍 `NotFound`），`Option` 的 `None` 分支消失
3. `issue_codebase_stores` 只接受 `Some(lc_id)`，删除 project 级 store 的 `None` 回退
4. 32 处出现点逐处改写：`Legacy` 臂删除；该臂原逻辑**要么合并进 `Logical` 臂、要么改为 `FailClosed`** —— 不新增第四种形态
5. 原「无 manifest 且无 selection」→ 现在落 `FailClosed`（稳定错误码），不再回退物理仓
6. **`#[cfg(test)]` 内的 2 处命中同样必须处置** —— `Legacy` 变体按决定 1 **整体移除**后，**任何残留的 `RepositoryRouting::Legacy` 引用都会编译不过**，`#[cfg(test)]` 代码也不例外 ⇒ `group_validation.rs:695` 与 `coding_cleanup.inc.rs:171` 二选一处置：**改写为 `Logical`/`FailClosed`**，或**删除该测试 / 夹具** —— **两种做法都不违反本 change 的契约，取舍由实施者决定**

**理由**：
- owner 裁定「不保留死代码」：留着永不出现的 `Legacy` 臂，下一个改这块的人仍会以为单仓可用
- 每个消费点的「如果是单仓怎么办」分支，就是「单仓专属流程」的藏身处；不删分支就删不掉路径
- `FailClosed` 已能承载全部不一致（含「压根没有 LC 归属」），不需要第三种表达

**代价**：
- 机械改动面大（32 处 / 19 个文件），需要分模块回归锁住多仓既有行为
- 有些消费点原本在 `Legacy` 臂里做「按 `repo_id` 找仓」的兜底，删除后**必须显式判为失败**，可能让此前「侥幸能跑」的旧数据现场变成明确报错 —— 这正是 owner 要的（不得静默回退）
- （技术方案 §8.1）`logical-codebase` 目录别名是否仍有生产写入方未逐条核完：实施时 `grep -rn '"logical-codebase"' src --include=*.rs` 区分读/写；仍有写入方则**保留**（不强删），本 change 不含该项

## 决策 2：`issue.logical_codebase_id` 从 `Option<String>` 改 `String`，老记录明确报错

**背景**：issue 记录的 `logical_codebase_id` 现为 `Option<String>`；`None` 既表示「单仓」也表示「旧数据」。这是 durable 记录格式。

**决定**：字段类型改为 `String`。**不做存量数据迁移脚本**（owner：默认没有任何存量单仓数据）。反序列化遇到缺失值 MUST 返回明确错误 `issue_not_bound_to_logical_codebase`，**不得静默回退单仓**。

**理由**：
- 类型上就不能表达「无归属」，比在每个消费点写 `if None` 更彻底
- 静默回退 = 把单仓又留回来；明确报错让现场立刻暴露，而不是半死不活

**代价**：
- 若真机真存在 `None` 老记录，该 issue 会**打不开**（明确报错）。owner 已裁定默认不存在；**若发现 → 停下报告，不擅自补数据**
- 需要一条显式错误码与它的中文文案（工作包 2.4）

## 决策 3：单仓入口删除，「建逻辑代码库」保持为独立步骤

**背景**：（实测）`web/src/components/lifecycle/AddCodebaseDialog.tsx:31` 的默认 `mode` 是 `"single_repo"`，`:101-119` 渲染「单仓库」/「多仓库逻辑代码库」两个 radio，`:131-140` 选中单仓时渲染「继续添加单仓库」按钮。（实测）`web/src/api/types/lifecycle.ts:345` 与 `work-item-plan.ts:679` 各自定义 `{ kind: "single_repository"; repository_id }` 变体；`WorkItemPlanOptionsDialog.tsx:533-534` 与 `state/workspace-cockpit-inbox-items.ts:54` 消费该变体。

**决定**：
1. 删「单仓库」radio 与「继续添加单仓库」按钮，**保留多仓库选项**；对话框默认态即多仓
2. 创建 issue 必须选一个 LC；`logical_codebase_id: null` 的提交路径删除
3. `single_repository` 类型变体与其消费分支（plan options / cockpit inbox）删除
4. 新建项目流程固定为：**创建项目 → 建逻辑代码库（选聚合根 + 登记成员仓）→ 创建 issue（选该 LC）**；「建逻辑代码库」是**独立步骤**，不塞进「创建项目」

**理由**：
- 单仓选项是两条路径并存的**入口**；入口不删，语义就删不干净
- 建 LC 有多仓专属的登记流程（选聚合根、登记成员、聚合初始化），塞进「创建项目」会让那个流程无处安放（技术方案 §3.1.2）
- 不新造页面：既有对话框去掉单仓分支后就是建 LC 的入口（技术方案 §8.4 的待验证项，实施第一步真机走一遍确认）

**代价**：
- `AddCodebaseDialog.test.tsx:13-32`、`CreateLifecycleIssueDialog.test.tsx:112-128` 等钉住单仓路径的测试需删除或改写
- 类型收窄会让所有 `single_repository` 分支在编译期消失 —— 这是 clean cutover，不保留兼容臂
- 若真机发现既有对话框去掉单仓后不像「建 LC」入口（技术方案 §8.4 未验证项），需要回头补交互，但**不新造页面**

## 决策 4：1 成员 LC 与 N 成员走完全同一套代码路径

**背景**：`member_ids` 允许 1 个成员。1 成员的「多仓」与单仓语义几乎等价 → 给单仓留了后门；且多仓专属机制（仓序自动边、跨仓验收）在 1 成员时的行为此前没有明确定义。

**决定**：
- 代码中 MUST NOT 出现「成员数为 1 时」的专门分支；1 成员只是 `member_ids.len() == 1`
- 多仓专属机制在 1 成员时**自然退化**（不是跳过）：仓序自动边 → 后仓集合为空 → 零条自动边；跨仓验收 → 只有本仓 → 单仓范围；成员/聚合索引 → 索引 1 个成员
- **「至少 2 个成员」类门禁一律移除**（owner 裁定：不禁止 1 成员 LC）

**理由**：
- owner 裁定「不存在多套、不存在 1 成员专属流程」—— 任何 `if members == 1` 都是一条待维护的第二路径
- 自然退化不需要守卫：「零条边」本身就是正确结果，不需要写 `if` 去跳过

**代价**：
- 少数按「至少 2 个成员」写下的检查（若有）会被移除，其原本保护的假设（多成员）不再由代码兜住
- 需要源码级断言（「不存在 `members.len() == 1` 专门分支」+「不存在拒绝 1 成员 LC 的门禁」）防回归；（实测）本 worktree `src/**` 中 `len() == 1` 的命中均为测试夹具断言（如 `src/web/advance_plan.rs:1565` 断言 fixture 恰一成员），未见生产分支 —— 但技术方案 §8.3 列为未验证项，实施第一步仍需完整 grep

## 决策 5：布局引导 = 规则表 + 按错误码给动作（文案单一来源）

**背景**：（实测）六个布局错误码都由登记预检/索引层产出：`aggregate_root_is_git`（`registration_preflight.inc.rs:60`、`:384`）、`member_path_outside_root`（`:90`、`:100`）、`member_symlink_escape`（`:111`）、`nested_worktree`（`:122`，另有 `registration_coordinator.inc.rs:627`）、`aggregate_root_overlap`（`:166`、`:170`）、`aggregate_index_layout_unsupported`（`aggregate_index/exclude.rs:240`、`:465`）、`aggregate_index_member_coverage_failed`（`aggregate_index/operation.rs:671`）。既有契约 `logical-codebase-registration` 的 `REQ-REG-09` 已定义这些**判定规则**本身。

**决定**：
1. 新增能力 `logical-codebase-layout-guidance`：把技术方案 §3.2.1 的**六条规则 → 错误码 → 用户该怎么摆**做成规则表，写入 spec
2. 多仓创建/登记入口与聚合初始化卡片给**目录树示意**（`my-lc-root/` 是普通空目录、`backend/`/`frontend/` 是直接子目录），并说明「只在 `.worktrees/` 下的代码不可被索引」
3. 失败面按错误码直接呈现「原因 + 怎么做」；文案进块2 建立的**单一文案来源**（`failure_copy.rs`，由 `non-interrupt-repository-bootstrap` 的 `REQ-BOOT-08` / `REQ-BOOT-10` / `REQ-BOOT-11` / `REQ-BOOT-12` 拥有），**不另抄一份**
4. 依赖前置（codegraph）：`codegraph_missing` / `codegraph_version_mismatch` 进入口引导清单，同样与 `REQ-BOOT-08` / `REQ-BOOT-10` / `REQ-BOOT-11` / `REQ-BOOT-12` 同源；安装/更新机制只引用块2 `provider-cli-installation` 的 `REQ-PCI-02` / `REQ-PCI-05`，不重定义

**理由**：
- 用户不看错误码，只看「怎么摆」。规则表把「判定」与「动作」绑在一处，避免六处各写一句
- 两处文案各自维护必然漂移（技术方案 §3.4 的落点要求）—— 因此复用块2 的单一来源，而不是新建一个「布局文案模块」

**代价**：
- 布局引导对块2 的 `failure_copy.rs` 有**运行期依赖**：块2 未落地时本能力只能先落规则表与错误码映射，文案接线要等块2（工作包 5.4 标注依赖）
- 规则表会随预检规则演进；改规则必须同时改表（spec 里写死映射，等于承认这份耦合）

## 决策 6：票3 一键核验 = 异步作业 + 4 家并发 + 逐项进度

**背景**：（实测）`src/web/handlers/logical_codebase_capabilities.rs:30-34` 的请求体只有 `provider_type`，`:106-170` 是单家同步核验（文件头注释明写「同步等待——与 aggregate index rebuild 分钟级同步动作同先例」）。（技术方案 §3.3.1）单次核验是分钟级；4 家串行 = 4 倍等待，同步 HTTP 会被代理超时截断。

**决定**：
1. 新增 `POST .../logical-codebases/{lc_id}/capabilities/revalidate-all` → **立即返回 `job_id`**，不阻塞
2. 新增 `GET .../capabilities/revalidate-all/{job_id}` → 轮询进度与逐项结果（`confirmed` / `failed` / `skipped` + `reason`）
3. **4 家全并发**（每家一个 provider 进程）；单家失败**不阻断**其它家
4. **复用既有单家 `revalidate(project_id, lc_id, provider)`**，不改其内部语义 —— 既有幂等短路（全行 Confirmed@当前版本 → `AlreadyConfirmed`）与 CLI 缺失快速失败因此天然继承
5. 作业状态 **durable**（job + 每家状态），页面刷新后按 `job_id` 继续轮询
6. 前端：能力卡头部「一键核验全部」按钮 → 进行中禁用并显示「核验中（N/4）」→ 失败项保留单项「重试」（走既有单 provider 端点）
7. 显式点击才运行；**不因聚合初始化失败自动触发批量核验**（与既有「不点击不运行探针」契约一致）
8. 与块2 `provider-cli-installation` 的联动：`skipped`（`cli_not_installed`）项的引导指向该 provider 的**【安装】**按钮；版本落后项指向**【更新】** —— 安装/更新机制本身**由块2 定义**，本 change 只引用，不重复定义

**理由**：
- owner 2026-11-03 裁定异步（真机能接受）；同步分钟级会被网关截断
- 复用单家 `revalidate` 而不是重写批量探针：否则「已确认秒回」「CLI 缺失快速失败」两件事要做两遍，且必然漂移
- 并发不是优化而是正确性：用户点「全部」期待的是「一次操作」，串行的 4 倍等待会让人以为卡死

**代价**：
- 新增一套作业 durable 记录与轮询端点（作业状态机 + 过期态处理），比同步多一块状态
- 前端从「一个请求」变成「发起 + 轮询」，需要处理页面刷新恢复与终态停止
- 会真实花 provider 额度（每次探针跑 LLM 轮次）→ 按钮文案必须说明「会真实跑一次探测，可能需要几分钟」
- 作业机制与块2 的安装作业**同构**（技术方案 §3.6.6 明确「与票3 同一套作业机制，可复用」）→ 若块2 先落地，本 change 复用其作业存储；若并行落地，以块2 的作业机制为准，不另造第二套

## 明确不改的部分

| 项 | 原因 |
|---|---|
legacy 布局（`logical-codebase/` 目录及其下所有路径、`migrate_legacy`、旧别名仓双路扫描、布局冲突判定） | 它是**多仓数据的物理落点**；删掉 = 删当前所有多仓数据 |
多仓路径的任何行为 | 本 change 只做减法；多仓全流程逐字不变 |
仓序（`change_order`）自动边机制 | 不变；1 成员时自然零边 |
单 plan 不变量 | 不变 |
`logical-codebase-registration` 预检判定规则（`REQ-REG-09` 等） | 不变；本 change 只加「动作文案」层 |
聚合索引与 `logical-codebase-aggregate-index` | 不变 |
provider 能力核验契约（「不点击不运行探针」）与单家 `revalidate` 逻辑 | 不变；票3 只是把多个单次调用并发化，仍是显式点击 |
`aggregate_root_ownership_conflict` 兼容映射臂 | 保留（历史兼容，已无生产者） |
provider / codegraph 安装与更新机制 | 属块2 `provider-cli-installation`；本 change 只引用其 requirement |
`codebase-kinds` 的「单仓代码库条目仍可在统一列表中出现」 | 本 change 删的是 issue 创建/路由入口，不是既有 `RepositoryRecord` 的呈现；列表行为不变（见报告 concerns） |

## 决策带来的连锁要求

| 要求 | 原因 | 落地位置 |
|---|---|---|
`Legacy` 变体不保留任何形式的替代臂 | owner：不保留死代码 | 技术方案 §3.1.1；`REQ-ROUTE-01` |
`issue.logical_codebase_id` 改 `String` 且老记录报错而非静默回退 | 静默回退等于把单仓留回来 | 技术方案 §3.1.1 / §5；`REQ-ROUTE-03` |
必须 grep 排查「成员数 == 1」的分支 | owner：不存在 1 成员专属流程 | 技术方案 §3.1.4；`REQ-REG-17` |
legacy 布局零改动 | 它是多仓数据落点；owner 要求多仓与现在一致 | 技术方案 §3.1.3 / §4；本文件决策 0 |
票3 必须异步 + 并发 + durable | 同步会被截断；串行 4 倍等待；刷新丢进度 | 技术方案 §3.3；`REQ-PCR-BATCH-01` |
入口文案必须与块2 的 `failure_copy.rs` 同源 | 避免两处文案漂移 | 技术方案 §3.4；`REQ-LAYOUT-02` / `REQ-LAYOUT-03` |
安装/更新 requirement 只引用不重定义 | 块2 定义、块3 复用 | 技术方案 §3.3.3 / 块2 §3.6；`REQ-PCR-BATCH-01` |
