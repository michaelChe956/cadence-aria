# 提案：去单仓入口 + 统一多仓流程

> 状态：草案 —— owner 决策已全部拍板（见文末）。技术方案见 `cadence/designs/2026-11-02_技术方案_块3_去单仓入口与统一多仓流程_v1.0.md`（owner 2026-11-02 审阅通过，2026-11-03 补决策与范围澄清）。
> 覆盖：票1 去单仓入口（能力下线）/ 票2 多仓入口布局引导 / 票3 provider 能力核验一键化。

## 背景

产品里「单仓」与「多仓」两条路长期并存，且多仓是**按 project opt-in 的实验性路径**，单仓仍是默认路径：

| 环节 | 现状 | 证据 |
|---|---|---|
产品入口 | 「添加代码库」对话框默认选「单仓库」radio，另有「继续添加单仓库」按钮 | `web/src/components/lifecycle/AddCodebaseDialog.tsx:31`、`:101-119`、`:131-140` |
issue 创建 | 创建 issue 时可提交「单仓库代码库」（`logical_codebase_id: null`） | `web/src/api/types/lifecycle.ts:345`、`work-item-plan.ts:679` |
路由 | `RepositoryRouting` 三态：`Legacy`（单仓语义）/ `Logical` / `FailClosed` | `src/product/logical_codebase/repository_routing_types.inc.rs:37-60` |
身份解析 | `resolve_issue_logical_codebase_id` 返回 `Option<String>`：`Some`=LC、`None`=单仓或旧数据 | 同文件 `:63-77` |

`RepositoryRouting::Legacy` 在生产代码有 **30 处**（本 worktree 实测，按 `#[cfg(test)]` 精确切分、真正编进生产二进制的部分，**分布 19 个文件**：**25 个匹配臂消费 + 5 个构造点产生**）；若按文件名排除测试文件计则为 **32 处 / 19 个文件**（**26 个匹配臂消费 + 6 个构造点产生**；差在 `group_validation.rs` 与 `coding_cleanup.inc.rs` 各自的 1 处 `#[cfg(test)]` 内命中；含测试文件则为 34 处）。改写清单用的实测命令**自带排除测试文件的条件**，照抄得 32：`grep -rn "RepositoryRouting::Legacy" src/ web/src/ --include=*.rs --include=*.ts --include=*.tsx | grep -v '_tests' | grep -v 'tests\.inc' | grep -v '/tests/' | grep -v '\.test\.'`。每个匹配臂都要写「如果是单仓怎么办」的分支 —— 每个分支都是「单仓专属流程」的可能藏身处。多仓路径的每一步都要在两套语义之间自证，这才是长期成本。（与技术方案 §1.2 的对照：技术方案 §1.2 **已按实测更正**为「**32 处出现点**（其中 **26 个匹配臂消费、6 个构造点产生**），分布在 **19 个文件**」，并补了**两口径对照段**（口径① 按文件名排除测试文件 = **32 处 / 19 个文件** = 改写清单口径；口径② 按 `#[cfg(test)]` 精确切分 = **30 处 / 19 个文件**）；**本 change 与技术方案口径一致**。）

同时多仓入口**没有布局说明**：用户要自己猜「LC 根目录」与「成员仓 checkout」怎么摆，实测踩 `aggregate_root_is_git` / `member_path_outside_root` / `member_symlink_escape` / `aggregate_root_overlap` / `nested_worktree` / `aggregate_index_member_coverage_failed` 六个码，却拿不到「该怎么摆目录」的动作。

能力核验又只能**一家一家点**：`POST /api/projects/{project_id}/logical-codebases/{lc_id}/capability-revalidate` 的 body 只有 `provider_type`（`src/web/handlers/logical_codebase_capabilities.rs:30-34`、`:106-170`），首次是分钟级同步等待。

## 范围澄清（本提案的第一前提）

「legacy」这个词在两处指的不是同一件事，本提案**只动其中一件**：

| | 名字里的 legacy 指 | 处置 |
|---|---|---|
`RepositoryRouting::Legacy` 变体 | **单仓语义** —— issue 未绑定 LC 时回退物理仓 | **删**（本提案范围） |
`logical-codebase/` 磁盘目录与 `migrate_legacy` | **布局版本** —— v1.2 的 project 级布局，v1.3 之后仍是多仓数据的默认落点 | **一个字不动** |

核实依据：`src/product/app_paths.rs:75-178` 有十几个生产路径基于 `logical_codebase_root`（`aggregate_indexes_root` / `aggregate_policy_artifact_path` / `aggregate_initializations_root` / `aggregate_recipe_receipts_root` / `provider_trust_root` / `registration_preflights_root` / `registration_batches_root` / 各 lock 路径），`coding_attempt_store/paths.rs:130-149` 的 pointer-publications 也在其下；`logical_codebase/store_purge.inc.rs:183-188` 还记录了「旧根还在而墓碑已删时下一次 `migrate_legacy` 会把 LC 复活」的自愈机制。**删掉该目录 = 删掉当前所有多仓数据。**

## 目标

| # | 目标 |
|---|---|
1 | **只有一条路**：所有 issue 都归属一个逻辑代码库；`RepositoryRouting` 只剩 `Logical` / `FailClosed` 两态，`Legacy` 变体整个移除，不留永不出现的分支 |
2 | **单仓入口消失**：「添加代码库」「创建 issue」不再有单仓选项；新建项目界面**直接就是多仓**，且「建逻辑代码库」作为独立步骤（选聚合根 + 登记成员 + 聚合初始化） |
3 | **1 成员与 N 成员同一套路径**：成员数是参数不是分支；仓序自动边在 1 成员时自然为零条；「至少 2 个成员」类门禁一律移除 |
4 | **布局可引导**：多仓入口给目录树示意；六个布局错误码各自配「你该怎么摆目录」的动作 |
5 | **能力核验一键化**：一个「一键核验全部」按钮 → 异步作业 + 逐项进度 + 失败项单独重试；单家失败不阻断其它家 |
6 | **多仓全流程逐字不变**：本提案只做减法，`legacy` 布局、仓序机制、单 plan 不变量、单家核验语义、聚合索引全部不动 |

## 非目标

- **不删任何 legacy 布局**：`logical-codebase/` 目录、`migrate_legacy`、旧别名仓双路扫描（`dangling_reference/patrol/lc_partition.rs`）、布局冲突判定（`repository_routing_resolver.inc.rs`）零改动
- 不改仓序（`change_order`）自动边机制、不改单 plan 不变量、不改聚合索引与登记预检主体
- 不改单家能力核验的 `revalidate` 逻辑（票3 的批量作业**复用**它），不改「不点击不运行探针」契约
- 不实现 provider / codegraph 的安装与更新机制（属块2 `provider-cli-installation`；本提案只**引用**）
- 不做存量单仓数据迁移脚本（owner 裁定：默认没有存量数据）
- 不做「1 成员跳过某步骤」这类分支（1 成员与 N 成员走完全同一套代码路径）
- 不新增页面（「建逻辑代码库」复用既有「添加代码库」对话框改造后的形态）
- 不删 `aggregate_root_ownership_conflict` 兼容映射臂

## 受影响能力

| 能力 | 变更 |
|---|---|
`web-runtime-repository-routing` | MODIFIED `REQ-ROUTE-01`（三态→两态）、`REQ-ROUTE-02`（入口对齐中的单仓断言改写）、`REQ-ROUTE-03`（RENAMED：单仓回退语义 → 无归属不回退物理仓）、`REQ-ROUTE-C5-TARGET`（RENAMED + MODIFIED：载体判定只剩 LC 载体） |
`logical-codebase-registration` | ADDED：统一多仓入口（新建项目即多仓、建 LC 为独立步骤、提交 issue 必须选 LC）；1 成员与 N 成员同一套登记/流程路径 |
`logical-codebase-layout-guidance` | **ADDED（新能力）**：布局规则表（规则 → 错误码 → 用户该怎么摆）、失败面按错误码给动作、入口依赖前置提示（codegraph） |
`provider-capability-revalidation` | ADDED（增量，不重复定义单家语义）：一键批量核验的异步作业 + 轮询 + 逐项进度 + 单项重试；复用块2 `provider-cli-installation` 的安装/更新机制 |
`logical-codebase-issue-entry` | MODIFIED `REQ-MRE-01`（issue 必须选一个 LC）、REMOVED `REQ-MRE-02`（单仓零回归 —— 与「删掉单仓路径」直接冲突） |

> `logical-codebase-issue-entry` 不在任务书列的 4 份 delta 里，但 `REQ-MRE-02`「既有单仓 issue 创建与单仓 plan 链路 SHALL 保持行为零变化」是本提案**直接推翻**的条款（技术方案 §3.1.2 第 3-4 条要求删除单仓选项）。留着它会让实施者收到互相矛盾的两条 MUST，因此纳入本 change；范围仍限定在删除单仓创建路径，不改多仓成员多选语义。

## 风险与取舍

| 风险 | 处置 |
|---|---|
「legacy」相关代码里混着单仓语义与布局版本两种东西 | **逐处核实，不按名字猜**：只有属于单仓语义的才在 `Legacy` 消费点改写清单里处理；布局类（`migrate_legacy`、别名仓双路扫描、冲突判定）零改动 |
`issue.logical_codebase_id` 从 `Option<String>` 改 `String` 是 durable 格式变化 | 老记录反序列化必须给**明确错误**（`issue_not_bound_to_logical_codebase`），不得静默回退单仓。owner 裁定默认无存量数据 → 真机若发现 `None` 老记录，**停下报告**，不擅自补数据 |
| 32 处出现（19 个文件：26 个匹配臂消费 + 6 个构造点产生；按 `#[cfg(test)]` 精确切分进生产二进制的是 30 处 / 19 个文件）逐处改写可能改出新回归 | 改写原则统一：`Legacy` 臂删除，原逻辑**要么合并进 `Logical` 臂、要么改为 `FailClosed`**；由分模块回归锁住多仓既有行为。清单以 `tasks.md` 3.1 的命令为准（**命令自带排除测试文件的条件，照抄得 32 处 / 19 文件**；含测试则 34 处）。清单里的 2 处 `#[cfg(test)]` 命中**必须一并处置**（改写或删除该测试/夹具），处置方式由实施者决定 |
1 成员 LC 可能被某处「至少 2 个成员」门禁误拒 | 实施第一步先 grep 成员数判断（`members.len() == 1` / `member_count` / `require_multiple_members`）；找到即按 owner 裁定移除。本 worktree 实测 `src/**` 中 `len() == 1` 的命中均为测试夹具断言（如 `src/web/advance_plan.rs:1565` 是 fixture 形态断言），非生产分支 —— 实施时仍需复核 |
布局引导若写成「按错误码各写一份文案」会与块2 的 `failure_copy.rs` 漂移 | 布局动作文案进**同一份文案单一来源**，与块2 的失败面同源；codegraph 依赖提示亦同源引用，不另抄一份 |
一键核验会被代理超时截断 | owner 裁定异步：`POST .../revalidate-all` 立即返回 `job_id`，`GET .../revalidate-all/{job_id}` 轮询；4 家**全并发**（串行 = 4 倍等待） |
批量核验可能被误当成「自动核验」 | 仍是**显式点击**触发；作业不对任何 provider 自动起探针，与 `provider-capability-revalidation` 既有「不点击不运行探针」契约一致 |
批量作业状态若只在内存 → 刷新丢进度 | 作业状态 durable（job + 每家状态），页面刷新后按 `job_id` 继续轮询 |

## owner 决策记录（2026-11-02 / 2026-11-03 已全部拍板）

| # | 决策 | 结论 |
|---|---|---|
1 | 存量单仓数据 | **默认没有任何存量数据**，不用管、不保留；不做迁移脚本 |
2 | 下线节奏 | **不存在下不下线**（因为没存量数据） |
3 | 1 成员 LC | **不禁止**；1 成员与 N 成员走**完全同一套代码路径**，不存在多套、不存在 1 成员专属流程 |
4 | 去掉范围 | 单仓相关的**能去掉都去掉，不保留死代码**；但要保证 **LC 仓的全流程不被影响**（2026-11-03 追加：「保证多仓流程是没问题和现在保持一致的」） |
5 | 新建项目界面 | **直接就是多仓** |
6 | 「至少 2 个成员」类门禁 | **一律移除** |
7 | legacy 布局 vs 单仓语义（2026-11-03 核实） | `logical-codebase/` 目录是**布局版本**不是单仓语义 → **保留，一个字不动**。只删 `RepositoryRouting::Legacy` 变体、`resolve_issue_logical_codebase_id` 的 `None` 分支、单仓入口 |
8 | 票3 同步 vs 异步（2026-11-03） | **异步作业 + 轮询**（真机能接受异步） |
