# Design：Claude SC author 显式 effort 冻结策略

## Context

当前状态（均以 `.worktrees/feat-wig-optimize-11-08/` 为根，行号已实读核对）：

- SC / plan 的流式输入构造器 `src/product/workspace_engine/prompts.rs:464-548`（`build_work_item_plan_streaming_input_with_session`）给 `env_vars: BTreeMap::new()`（`:545` 附近）——**空 env**，子进程经底座继承宿主环境，因此 Claude SC author 的实际 effort 随宿主 settings 漂移。
- 该构造器是**共用**出口，被多条 author 路径复用：SC 初轮（`src/web/workspace_ws_handler/run/single_candidate.rs:1096-1112`）、SC 教学重驱（同文件 `:510`）、SC 人工修订（`src/web/workspace_ws_handler/run/provider_run/sc_manual_revision.rs:356-366`）、legacy outline（`run/provider_run/legacy_outline.rs:295-303,497`）、PlanRevision（`run/followups.rs:331-339,554,1038`）、Draft/Batch（`src/product/workspace_engine/draft_batch/runs.rs:154-162,248-256`）、基线教学（`prompts/baseline_teaching.rs:379-492`）。**改这个共用 builder 会波及 legacy / draft / plan-revision 家族**，这是决策报告 §3.2 点名的风险（"不能把所有 WorkItemSplitter 都改档"）。
- env 注入路径**已可达**，无需改 adapter：`src/web/workspace_ws_handler/run/gateway_start.rs:648-678` 明确「caller `input.env_vars` 原样透传（仅作显式覆盖层）」；`src/product/logical_codebase/provider_gateway.rs:1998-2001` 同口径。Claude 的真实 spawn 分别在 `src/cross_cutting/claude_code_provider/direct_start.rs:217-223` 与 `validated_start.rs:188-194` 把 `input.env_vars` 交给 `ProcessManager::spawn`。
- 公共配置面：`ProviderConfigSnapshot`（`src/web/workspace_ws_types/common.rs:113-119`）只有 author / reviewer / review_rounds / permission_modes，**不含 effort**；它被 `StartGeneration.provider_config` 携带（`src/web/workspace_ws_types/in_.rs:26-30`），并在 `src/product/workspace_engine/session_state/timeline.rs:493-499` 由 WIG session 重新构造。`src/product/coding_models/provider_config.rs:90-106` 从它转出 coding 快照。
- 恢复比较：`src/web/workspace_ws_handler/socket.rs:1413-1427` 的 fresh admission **逐字段**比较 author / reviewer / permission_modes（不是整体 `PartialEq`），**不含 effort**；同处亦不含 `review_rounds`（本期不顺手修）。
- durable / runtime 承载：`src/product/models/workspace.rs:843-850` 附近的 SC 字段区、`src/product/workspace_engine/types.rs:84-102`（runtime 会话）与 `:149-168`（从 durable record 重建 runtime）、`src/product/lifecycle_store/workspace.rs:324-335`（创建）与 `:481-499`（auto→interactive 接管：复制 history、清空 ledger / reservation）。
- 域判别：`WorkItemPlanFlowKind` 只有 `Legacy` 与 `SingleCandidate`（`src/product/work_item_plan_policy/types.rs:243-246`）；生产创建路径恒 `SingleCandidate`（`src/product/lifecycle_store/inputs.rs:233-238` 注释与 `web/handlers/workspace_session.rs:668`）。
- 官方语义（决策报告 §3.2 已核）：`CLAUDE_CODE_EFFORT_LEVEL` 优先于会话选择、CLI flag 与 settings；支持度依模型且组织 cap 仍有效 ⇒ 平台只能保证 **requested**，不能保证 effective。

## Goals / Non-Goals

**Goals（设计级边界）**

1. 让「新的 Claude SC author 原运行用 `high`」成为**服务端内部、耐久、按原运行冻结**的执行策略，而不是一次性的 env 注入。
2. 让同一原运行的**所有 author 调用路径**（初轮 / 教学重驱 / 人工修订 / native resume / 明确 fresh）复用**同一冻结策略**。
3. 让**作用域可判定且可测试**：Claude + SingleCandidate author 命中；非 Claude、legacy outline、PlanRevision、Draft/Batch、reviewer、Story/Design、coding 一律不变。
4. 让**存量运行**（无冻结事实）保持旧继承语义并**披露未知**，不伪造原档位。

**Non-Goals（设计级）**

- 不做可编辑的 effort/model 配置面（公共快照 / TS / durable / resume 全链另立设计）。
- 不改 provider adapter 的默认档位，不做全局硬编码。
- 不承诺任何提速幅度、token 降幅或质量结论（无实测）。
- 不顺手修恢复比较中 `review_rounds` 等其它遗漏项。
- 不做任何真实 provider 调用或 Case A/B 对照（**当前未授权**）。

## Decisions

### 决策 1：承载面 —— 服务端内部独立冻结配置，**不扩公共 DTO**

新增服务端内部类型（建议落 `src/product/models/sc_launch_policy.rs`）：

- 结构：`ProviderEffortConfig { policy_version, provider, author_effort }`
- `policy_version`：策略版本串（如 `sc-author-effort/v1`），使「按原运行冻结」在策略演进后仍可解释；
- `provider`：冻结时使用的 provider（作用域判定的耐久依据，不依赖后续会话状态反推）；
- `author_effort`：冻结的 requested 档位（本期唯一取值 `high`）。

**边界 1 —— 不扩公共 `ProviderConfigSnapshot`（硬约束）。**

- `ProviderConfigSnapshot` **不增加任何 effort 字段**，Rust wire、两份 TS 快照、`coding_models/provider_config.rs:90-106` 的转换链、`session_state/timeline.rs:493-499` 的快照重建**全部零改动**。
- 冻结事实**不**从 `StartGeneration.provider_config` 读入，**不**由客户端提供；服务端不新增任何客户端可写字段。
- 理由（决策报告 §3.3）：扩公共快照会波及 Rust wire、两份 TS 快照、coding 转换、session 持久与恢复比较；本期只需「平台 SC author 策略」，无用户配置 UI 要求。引入公共配置面会把一次内部策略变成长期公共契约。
- 代价（接受）：effort 暂不可被用户编辑；若将来需要可编辑，必须走一次完整的公共面设计（公共快照 + TS + durable + resume 比较），不在本 change 内暗开。

**边界 1 的推论（跨 change 共享）**：`bound-sc-generation-teaching` 的 `mechanicalize` 侧 MODIFIED REQ-WSC-09 归属在**那个 change 目录内**，本 change **不修改** `openspec/changes/mechanicalize-plan-compile-repair/**` 与 `openspec/changes/fix-author-rewrite-quota-pool/**` 的任何文件；本 change 的 delta 只写**不与二者重复**的部分。

### 决策 2：作用域判别 —— **显式 SC author 出口**，不从共用 builder 猜

共用 builder `build_work_item_plan_streaming_input*` 被 SC 与 legacy / draft / plan-revision 家族共享。若在该 builder 内部按「provider 是 Claude」一刀切注入，会把 legacy outline、PlanRevision、Draft/Batch 一并改档 —— 违反「作用域限定 Claude + SingleCandidate author」。

因此采取**出口分流**：

| 出口 | 归属 | effort |
|---|---|---|
| 新增 SC author 专用输入装配（承接 `prompts.rs:464-548` 的 SC 部分） | SC 初轮 / 教学重驱 / SC 人工修订 / resume / fresh | 命中冻结策略时注入 `CLAUDE_CODE_EFFORT_LEVEL=high` |
| 既有共用 builder（legacy outline / PlanRevision / Draft/Batch / 基线教学） | 非 SC 家族 | 保持空 env（现状不变） |

**判据（三者同时成立才命中）**：

1. provider == Claude（`ProviderName::ClaudeCode` / `ProviderType::ClaudeCode`）；
2. 调用点是 **SC author 出口**（不是 reviewer、不是 Story/Design、不是 coding）；
3. 该调用所属会话携带**冻结事实**（`Some`）。

`flow_kind == SingleCandidate` 是会话级佐证，但**不单独作为判据** —— draft/batch 与 plan-revision 的调用点与 SC 共用 builder，仅靠 flow_kind 无法把调用点分开，必须由调用点显式选择出口。

**决策 2 的推论（同一原运行一致性）**：SC 人工修订（`sc_manual_revision.rs:356-366`）虽然未在决策报告 §3.4 的枚举列表（初轮 / compile·IR·structure 教学 / native resume / fresh）中单列，但它与初轮是**同一原运行、同一 author 角色**。若排除它，同一原运行会在不同路径上出现不同档位，直接违反「同一原运行使用相同冻结策略」。故按该原则**纳入**同一冻结策略（这是从同运行冻结原则推出的必然结论，不是新增范围）。

### 决策 3：冻结事实的建立、贯穿与不追溯

- **建立**：服务端在**创建 SC 会话**时按服务端事实决定新默认（provider 是 Claude + SC flow ⇒ 写入 `high`）；写入后即成为该原运行的冻结事实。
- **贯穿**：durable record（`models/workspace.rs` SC 字段区）→ runtime 会话（`workspace_engine/types.rs:84-102`）→ 从 durable 重建 runtime（`:149-168`）→ 恢复 / 接管（`lifecycle_store/workspace.rs:324-335`、`:481-499`）逐字段贯穿，**逐字段**而非整体比较。
- **接管**（auto→interactive，`lifecycle_store/workspace.rs:481-499`）：**继承原策略**，MUST NOT 因新 session ID 换档。当前该路径复制 `run_history` / `human_gate_snapshot` / `policy_diagnostics` 等上下文并清空 ledger / reservation；冻结策略属**上下文**，应随复制保留。
- **读旧 record 不写入**：存量记录读出时冻结字段为缺省值 ⇒ 保持旧继承语义，**不补伪值**，**不写回**（读路径纯读）。
- **恢复**：恢复 / fresh 时使用**服务端原运行的冻结事实**；`socket.rs:1413-1427` 的既有客户端 provider 比较**保持不变**（不扩客户端比较面），另在服务端核验「原运行绑定的策略引用 / 摘要」与「当前冻结配置」一致，摘要不一致 ⇒ typed 协议错误，**不静默按当前默认继续**。

### 决策 4：requested 与 effective 分离

- 平台保证的是 **requested**（`high`）。观测 / 日志 MUST 记录 requested。
- **effective** 以 provider 支持能力与请求证据为准；无证据 ⇒ 记「未知」，**MUST NOT** 以 `high` 代替未知，**MUST NOT** 把组织 cap 压低后的 effective 冒称 `high`。
- 旧运行（无冻结事实）的 effective 一律「未知」。

### 决策 5：策略版本不识别的失败处理（fail-closed）

冻结事实**存在但 `policy_version` 无法识别**时：MUST NOT 回落到「当前部署默认」或宿主继承去继续启动 —— 那等于用当前默认覆盖旧绑定。处理：**零 provider 启动 + typed 错误 + 保留原记录**（fail-closed）。

推导依据：决策报告 §3.4 要求「不得用当前默认覆盖旧绑定」；不可识别的版本无法证明「按原运行冻结」，因此不得启动。这属实现级失败处理，不改变任何对外契约。

### 决策 6：env 注入点

- 由**小型独立模块**把冻结策略转成 caller env（`CLAUDE_CODE_EFFORT_LEVEL=high`），SC author 出口在装配 `StreamingProviderInput.env_vars` 时写入。
- 注入**只发生在 SC author 出口**；其它出口 env 保持空。
- 不改 adapter：`gateway_start.rs:648-678` / `provider_gateway.rs:1998-2001` 已保证 caller env 原样透传为显式覆盖层；Claude direct / validated spawn 已消费 `input.env_vars`。
- 不在每次启动时重读宿主或平台新默认：注入值**只**来自该原运行的冻结事实。

## 边界 2：存量继承模式的 **effective 未被追溯冻结**（兼容例外，必须随契约披露）

这是本 change 最容易被误述的一点，必须在 delta 与验收里同时出现：

- 无冻结事实的存量运行**保留旧继承模式**（不注入平台覆盖、不补造历史快照）；
- 但它的**实际 effective 档位仍可能随宿主 settings 变化** —— 平台既没有冻结它，也无法证明它原来的值；
- 因此 **MUST NOT** 宣称「旧运行的 effective 已被追溯冻结」；对外披露必须是「未知 / 旧继承模式」；
- 「旧 None 不追溯改档」**不等于**「旧运行的档位从此固定」：只有**下一次真实新生成**（新的合法生成授权、新建原运行）才使用届时的新默认策略。

推论：验收只能证明「旧 None 继续不注入平台覆盖、不补造历史快照」，**不能**证明「旧运行的 effective 恒定」。

## 边界 3：**真实 provider 调用需另行授权**（当前未授权）

- 本 change 的产出是**规划制品**（proposal / specs / design / tasks）。**任何**真实 provider 调用、effort 实验、Case A/B 对照、真实时长 / token / 质量测量**均未获授权**，本期一律不执行。
- `tasks.md` 中相关条目一律写成「**获授权后执行**」，不得写成已执行、不得预填测量结论。
- 质量验收口径（决策报告 §3.5）：获授权后按项目 prompt 验证规则，涉及 Draft / Canonical 输出约束时执行获授权 Case A/B 各 10 次首次输出 10/10，不计自动修复；先 F 可用再对照时长、input/cache、IR 首次通过、真实语义缺陷、越权回归；**不以「输出完成」替代计划正确**。
- 授权到位前的可执行验证只有**离线矩阵**（env 装配、durable roundtrip、恢复 / 接管、作用域矩阵），不产生任何 provider 副作用。

## 共享面与排序（跨 change）

> 本 change 与其它 change 的写面交集在此登记；**共享文件的接线由集成 owner 串行执行**，任何一方都不得并发改同一文件。

| # | 共享面 | 涉及 change | 归属与顺序 |
|---|---|---|---|
| 1 | `src/product/models/mod.rs`（新模块注册） | 本 change（`sc_launch_policy`）、`bound-sc-generation-teaching`（`sc_generation_attempt`） | **集成 owner 串行**：两个新模块的注册在同一文件，按「先注册者先落」合并，不并发编辑 |
| 2 | `src/product/models/workspace.rs`（SC 字段区 `:843-850` 附近） | 本 change（冻结字段）、`bound-sc-generation-teaching`（SC generation window 字段） | **同文件串行**：字段追加互不覆盖，但落盘顺序由集成 owner 统一，避免两次 serde 迁移 |
| 3 | `src/product/lifecycle_store/workspace.rs`（创建 `:324-335` / 接管 `:481-499`） | 本 change（冻结策略写入与继承）、`bound-sc-generation-teaching`（窗口建立 / 接管只引用原窗口） | **同文件串行**：接管的「继承 vs 清空」语义两方都要动同一段 ⇒ 先合本 change 的冻结继承，再合窗口只引用 |
| 4 | `src/product/workspace_engine/types.rs`（`:84-102` runtime / `:149-168` 重建） | 本 change（冻结字段贯穿）、`bound-sc-generation-teaching`（窗口 / ledger 同步） | **同文件串行**（runtime→durable→runtime 镜像的 CAS 合流问题在窗口侧更重，建议窗口侧后落） |
| 5 | `src/web/workspace_ws_handler/run/single_candidate.rs`（初轮 `:1096-1112`、教学 `:510`） | 本 change（SC author 出口的 env 装配）、`bound-sc-generation-teaching`（统一教学编排）、决策六（handler 拆分） | **决策六的拆分先落**（纯移动、行为不变），再由集成 owner 依次接 SC 出口与教学编排；**不得并发插 handler** |
| 6 | `src/product/workspace_engine/prompts.rs`（共用 builder `:464-548`、`:419` 其它 author 出口） | 本 change（SC author 出口抽离）、`bound-sc-generation-teaching`（compile 教学块迁出）、决策六（prompts 拆分） | **决策六的拆分先落**；本 change 只抽离 SC author 段，**`:419`（其它 author）与 `review_parts/single_candidate_input.inc.rs:315`（reviewer）本期不改** |
| 7 | `src/web/workspace_ws_handler/socket.rs`（fresh admission `:1413-1427`） | 本 change（服务端核验冻结策略引用 / 摘要）、`mechanicalize` / 恢复相关改动 | **同文件串行**；本 change 只**新增服务端侧**核验，**不改**既有客户端 provider 比较条件，也不顺带加 `review_rounds` |
| 8 | 通道 C 的**冻结输入摘要** | `bound-sc-generation-teaching`（窗口冻结输入）、本 change（effort 策略摘要） | **窗口的冻结输入摘要 MUST 纳入 effort 策略摘要** ⇒ 教学重驱不得用当前默认覆盖旧绑定。**该摘要的构造在 `bound-sc-generation-teaching` 的目录内**，本 change **不修改**其文件；由集成 owner 在该 change 落地时统一接上（见下「需集成 owner 执行的事项」） |

### 需集成 owner 执行的事项（跨 change，非本 change 文件）

1. `bound-sc-generation-teaching` 的 SC generation window **冻结输入摘要**中追加 effort 策略摘要（provider + policy_version + author_effort + digest），使教学重驱按原运行的冻结策略执行，而非当前默认。
2. `mechanicalize-plan-compile-repair` 的 MODIFIED REQ-WSC-09（加入 typed IR 与 structure）**归该 change 的 owner**，在本 change 目录内**不做**任何改动；本 change 的 delta 与其无重叠条款。
3. 本 change 归档时，两个 change 的 `work-item-plan-single-candidate` delta 需各归各 requirement（REQ-WSC-29 / REQ-WSC-28 等）互不覆盖。

## Risks / Trade-offs

| 风险 / 取舍 | 处置 |
|---|---|
| `high` 的质量与耗时收益**均未实测** | 本期只冻结口径；不承诺时长 / token 降幅；真实对照需另行授权后实测（边界 3） |
| 组织 cap 可能把 effective 压到 `high` 之下 | requested / effective 分离；effective 无证据即未知，MUST NOT 冒称 `high` |
| 存量运行无法证明原 effective | 保留旧继承 + 披露未知（边界 2）；MUST NOT 追溯冻结 |
| 出口分流需要改动共用 builder 的调用面，可能误伤 legacy 家族 | 出口分流 + 作用域矩阵验收：legacy outline / PlanRevision / Draft/Batch / 基线教学的 env **必须仍为空** |
| 冻结字段进入 durable record 触发 serde 迁移 | 一律可缺省 + 跳过序列化；旧记录反序列化不变；读旧 record **不写回** |
| 策略版本演进后老记录不可读 | 未知 `policy_version` ⇒ fail-closed（决策 5），不静默按当前默认启动 |
| 不扩公共 DTO 意味着 effort 暂不可编辑 | 接受；可编辑化另立完整公共面设计，不在本 change 暗开 |
| 与 `bound-sc-generation-teaching` 的窗口摘要接线跨 change | 登记在「共享面与排序」第 8 项，由集成 owner 统一执行 |
| 真实 provider 调用未授权 | 全部真机项写成「获授权后执行」，本期只跑离线矩阵（边界 3） |
