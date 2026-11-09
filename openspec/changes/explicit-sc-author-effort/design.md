# Design：Claude SC author 显式 effort 冻结策略

## Context

当前状态（均以 `.worktrees/feat-wig-optimize-11-08/` 为根，行号已实读核对）：

- SC / plan 的流式输入构造器 `src/product/workspace_engine/prompts.rs:464-548`（`build_work_item_plan_streaming_input_with_session`）给 `env_vars: BTreeMap::new()`（`:545` 附近）——**空 env**，子进程经底座继承宿主环境，因此 Claude SC author 的实际 effort 随宿主 settings 漂移。
- 该构造器是**共用**出口，被多条 author 路径复用：SC 初轮（`src/web/workspace_ws_handler/run/single_candidate.rs:1096-1112`）、SC 教学重驱（同文件 `:510`）、SC 人工修订（`src/web/workspace_ws_handler/run/provider_run/sc_manual_revision.rs:356-366`）、legacy outline（`run/provider_run/legacy_outline.rs:295-303,497`）、PlanRevision（`run/followups.rs:331-339,554,1038`）、Draft/Batch（`src/product/workspace_engine/draft_batch/runs.rs:154-162,248-256`）。`src/product/workspace_engine/prompts/baseline_teaching.rs:379-492` 的基线教学用例自 `:337` 起整体位于 `#[cfg(test)]` 模块内，**不是生产 author 出口**（生产面只有该文件的 `baseline_teaching_block` / `reviewer_baseline_teaching_block` / `run_boundary_block` 三个纯文本构造器）。改这个共用 builder 会波及 legacy / draft / plan-revision 家族，这是决策报告 §3.2 点名的风险（"不能把所有 WorkItemSplitter 都改档"）。
- env 注入路径**已可达**，无需改 adapter：`src/web/workspace_ws_handler/run/gateway_start.rs:648-678` 明确「caller `input.env_vars` 原样透传（仅作显式覆盖层）」；`src/product/logical_codebase/provider_gateway.rs:1998-2001` 同口径（保留 caller 显式覆盖）。Claude 的真实 spawn 分别在 `src/cross_cutting/claude_code_provider/direct_start.rs:217-223` 与 `validated_start.rs:188-194` 把 `input.env_vars` 交给 `ProcessManager::spawn`。
- 公共配置面：`ProviderConfigSnapshot`（`src/web/workspace_ws_types/common.rs:113-119`）只有 author / reviewer / review_rounds / permission_modes，**不含 effort**；它被 `StartGeneration.provider_config` 携带（`src/web/workspace_ws_types/in_.rs:26-30`），并在 `src/product/workspace_engine/session_state/timeline.rs:493-499` 由 WIG session 重新构造。`src/product/coding_models/provider_config.rs:90-106` 从它转出 coding 快照。
- 恢复比较：`src/web/workspace_ws_handler/socket.rs:1413-1427` 的 fresh admission **逐字段**比较 author / reviewer / permission_modes（不是整体 `PartialEq`），**不含 effort**；同处亦不含 `review_rounds`（本期不顺手修）。
- durable / runtime 承载：`src/product/models/workspace.rs:843-850` 附近的 SC 字段区、`src/product/workspace_engine/types.rs:84-102`（runtime 会话）与 `:149-168`（从 durable record 重建 runtime）、`src/product/lifecycle_store/workspace.rs:324-335`（创建）与 `:455-470`（auto→interactive 接管的创建调用）、`:485-499`（接管的逐字段复制）。
- 域判别：`WorkItemPlanFlowKind` 只有 `Legacy` 与 `SingleCandidate`（`src/product/work_item_plan_policy/types.rs:243-246`）；生产创建路径恒 `SingleCandidate`——`src/web/handlers/lifecycle/plan_preparation.rs:134` 直接 `let flow_kind = WorkItemPlanFlowKind::SingleCandidate;`，并在 `:210`（`create_workspace_session_with_id` 分支）与 `:235`（`create_workspace_session` 分支）两处写入 `work_item_plan_options.flow_kind`。
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

### 决策 2：作用域判别 —— **显式 SC author 出口**，不从共用 builder 猜

共用 builder `build_work_item_plan_streaming_input*` 被 SC 与 legacy / draft / plan-revision 家族共享，且**共用 builder 无法按 provider 区分**——它对同一族调用点只拿到 author provider 与角色上下文，**没有**「本次是 SC author 还是 legacy outline」的判据；实际的 provider/角色分流发生在 `ProviderRunKind`（`src/web/workspace_ws_handler/run/provider_run.rs:286-293`：`ProviderRunKind::WorkItemPlanLegacyAuthor | WorkItemPlanSingleCandidateAuthor | WorkItemPlanOutlineRevision | WorkItemPlanOutlineRebuild | WorkItemPlanDraft | WorkItemPlanBatch | WorkItemPlanRevision | HumanGateScManualRevision` 归到 `engine.session().author_provider.clone()`）。因此若在共用 builder 内部按「provider 是 Claude」一刀切注入，会把 legacy outline、PlanRevision、Draft/Batch 一并改档 —— 违反「作用域限定 Claude + SingleCandidate author」。

因此采取**出口分流**：

| 出口 | 归属 | effort |
|---|---|---|
| 新增 SC author 专用输入装配（承接 `prompts.rs:464-548` 的 SC 部分） | SC 初轮 / 教学重驱 / SC 人工修订 / resume / fresh | 命中冻结策略时注入 `CLAUDE_CODE_EFFORT_LEVEL=high` |
| 既有共用 builder（legacy outline / PlanRevision / Draft/Batch） | 非 SC 家族 | 保持空 env（现状不变） |
| builder 内基线注入（`prompts/baseline_teaching.rs` 的 `baseline_teaching_block` / `run_boundary_block`） | 纯文本边界块，不承载 env | **不受影响**（基线注入与执行策略变量无耦合） |

**判据（三者同时成立才命中）**：

1. provider == Claude（`ProviderName::ClaudeCode` / `ProviderType::ClaudeCode`）；
2. 调用点是 **SC author 出口**（不是 reviewer、不是 Story/Design、不是 coding）；
3. 该调用所属会话携带**冻结事实**（`Some`）。

`flow_kind == SingleCandidate` 是会话级佐证，但**不单独作为判据** —— draft/batch 与 plan-revision 的调用点与 SC 共用 builder，仅靠 flow_kind 无法把调用点分开，必须由调用点显式选择出口。

**决策 2 的推论（同一原运行一致性）**：SC 人工修订（`sc_manual_revision.rs:356-366`）虽然未在决策报告 §3.4 的枚举列表（初轮 / compile·IR·structure 教学 / native resume / fresh）中单列，但它与初轮是**同一原运行、同一 author 角色**。若排除它，同一原运行会在不同路径上出现不同档位，直接违反「同一原运行使用相同冻结策略」。故按该原则**纳入**同一冻结策略（这是从同运行冻结原则推出的必然结论，不是新增范围）。

### 决策 3：冻结事实的建立、贯穿与不追溯

- **建立**：服务端在**创建 SC 会话**时按服务端事实决定新默认（provider 是 Claude + SC flow ⇒ 写入 `high`）；写入后即成为该原运行的冻结事实。
- **贯穿**：durable record（`models/workspace.rs` SC 字段区）→ runtime 会话（`workspace_engine/types.rs:84-102`）→ 从 durable 重建 runtime（`:149-168`）→ 恢复 / 接管（`lifecycle_store/workspace.rs:324-335`、`:455-470`、`:485-499`）逐字段贯穿，**逐字段**而非整体比较。
- **接管**（auto→interactive）：MUST **原样复制父会话的冻结字段**，父为 `None` 时保持 `None`，MUST NOT 走「创建时写新默认」的路径，MUST NOT 因新 session ID 换档。代码依据：接管先以 `create_workspace_session_with_id` 建子会话（`lifecycle_store/workspace.rs:455-470`），再逐字段复制父会话上下文（`:485-499`）；因此**接管路径 MUST NOT 触发新默认写入**，冻结策略属**上下文**，应随复制保留。
- **读旧 record 不写入**：存量记录读出时冻结字段为缺省值 ⇒ 保持旧继承语义，**不补伪值**，**不写回**（读路径纯读）。
- **恢复**：恢复 / fresh 时使用**服务端原运行的冻结事实**；`socket.rs:1413-1427` 的既有客户端 provider 比较**保持不变**（不扩客户端比较面），另在服务端核验「原运行绑定的策略引用 / 摘要」与「当前冻结配置」一致，摘要不一致 ⇒ typed 协议错误，**不静默按当前默认继续**。

#### 决策 3.1：「原运行」与「真实新生成」的界定（裁决 §3.1(3) / §3.4 / §3.5 的落地定义）

- **原运行** ＝ **同一 session record 承载的同一冻结事实**。其上的一切运行——重臂、resume、native fresh、compile/IR 教学 attempt——**均属同一原运行**，MUST 继续使用**同一冻结策略**；MUST NOT 在重臂时被写入当前部署默认，MUST NOT 因换了一条执行路径或拿到新 session ID 而换档。
- **真实新生成** ＝ 服务端**创建事实**为**新建 session record** 的首次生成。只有它适用**当前部署默认**（本期 `high`）。
- 读旧 record（含旧 record 上的重臂）MUST NOT 写入新默认：**保持 `None`（继承宿主）并披露未知**。

依据：裁决 §3.1(3)「旧已开始运行保原配置；不存在耐久 effort 证据的旧运行不得在 resume 时被补写成『原本 high』」；§3.4「server 创建事实决定新默认，**读旧 record 不写入**」；§3.5「存量继承模式的实际 effort 仍可能受宿主变化影响，不能宣称旧运行的 effective 已被追溯冻结」。

#### 决策 3.2：冻结点与 provider 可变的一致性条款（裁决 §3.4 补强）

`author_provider` 在创建后仍可被 `set_provider` 改写，因此「创建时写入的 provider」与「真实 SC 首次生成准入时的 author_provider」可能不一致。定案：

- 若创建时写入的 provider 与真实 SC 首次生成准入时的 author_provider **不一致**，MUST NOT 静默沿用该档位：MUST 按「**无冻结事实**」处理（**披露未知**、**不注入平台覆盖**），MUST NOT 以创建时值覆盖准入时值。
- 创建时写入本身**不改**（裁决 §3.4「server 创建事实决定新默认」），本条只约束两者不一致时的**处理口径**。
- 该条款已落 `specs/work-item-plan-single-candidate/spec.md` 的「创建时 provider 与准入时 author_provider 不一致按无冻结事实处理」场景。

### 决策 4：requested 与 effective 分离

- 平台保证的是 **requested**（`high`）。观测 / 日志 MUST 记录 requested。
- **effective** 以 provider 支持能力与请求证据为准；无证据 ⇒ 记「未知」，**MUST NOT** 以 `high` 代替未知，**MUST NOT** 把组织 cap 压低后的 effective 冒称 `high`。
- 旧运行（无冻结事实）的 effective 一律「未知」。

### 决策 5：策略版本不识别的失败处理（fail-closed）

冻结事实**存在但 `policy_version` 无法识别**时：MUST NOT 回落到「当前部署默认」或宿主继承去继续启动 —— 那等于用当前默认覆盖旧绑定。处理：**零 provider 启动 + typed 错误 + 保留原记录**（fail-closed）。

推导依据：决策报告 §3.4 要求「不得用当前默认覆盖旧绑定」；不可识别的版本无法证明「按原运行冻结」，因此不得启动。这属实现级失败处理，不改变任何对外契约。

> **口径标注（本条为推导，不是裁决逐字原文）**：裁决 §3.5 明文「未知 policy_version 零启动 + typed 错误 fail-closed」，但 §3.4 未逐字给出该条；本条是**依据裁决 §3.5 推导的安全补充**，不冒充裁决原文。对应场景已挂在 **REQ-WSC-29**（`specs/work-item-plan-single-candidate/spec.md`）的「冻结策略的 policy_version 不可识别时零启动」场景，**不**挂在已超字符预算的 resume MODIFIED 上。

### 决策 6：env 注入点

- **注入点 = SC author 出口模块**：新增 `src/product/workspace_engine/prompts/sc_author_input.rs`（承接 `prompts.rs:464-548` 的 SC author 段）与 `src/product/workspace_engine/prompts/sc_launch_policy.rs`（把冻结策略转成 caller env，即 `CLAUDE_CODE_EFFORT_LEVEL=high`），由 SC author 出口在装配 `StreamingProviderInput.env_vars` 时写入。
- **注入点不是** `src/cross_cutting/claude_code_provider/mod.rs`（adapter 硬编码路径，裁决 §3.3 明文否决，会静默影响所有 Claude 角色/使用者）；**注入点也不是** `src/product/workspace_engine/prompts.rs:545`（那是**共用 builder 的空 env 现状位置**，是本 change 要**被替换/分离**的对象，不是注入实现位置）。
- 注入**只发生在 SC author 出口**；其它出口 env 保持空。
- 不改 adapter：`gateway_start.rs:648-678` / `provider_gateway.rs:1998-2001` 已保证 caller env 原样透传为显式覆盖层；Claude direct / validated spawn 已消费 `input.env_vars`。
- 不在每次启动时重读宿主或平台新默认：注入值**只**来自该原运行的冻结事实。
- **本期不改的既有出口**（防止扩大 G 范围）：`prompts.rs:419`（其它 author 出口）、`prompts/review_parts/single_candidate_input.inc.rs:315`（reviewer 出口）、`coding_models/provider_config.rs:90-106`（coding 转换），以及**新增** `src/product/workspace_engine/prompts/revision.rs:96`（Story/Design 修订出口，同样是 `env_vars: BTreeMap::new()`）。
- **retry 继承原出口**：`src/product/workspace_engine/provider_drive/artifact_retry.rs:248-263` 的 `build_artifact_retry_input` 以 `base_input.clone()` 为基底改写 prompt，**不重建 env**，因此 retry 轮继承原出口已装配的 env（含冻结策略变量），MUST NOT 另行注入或清空。

### 决策 7：对主规格 REQ-ENV-HOST-01 的**显式修订**（走 OpenSpec delta，不静默违反）

主规格 `openspec/specs/session-policy-envelope/spec.md` 的 **REQ-ENV-HOST-01**（`### Requirement: 统一宿主执行与受信环境（REQ-ENV-HOST-01）`）正文写明「平台不构造、不覆盖子进程环境（调用方显式 env map 仅作覆盖层）」，其「运行环境与清理边界」场景写明「平台不注入」。本 change 要在 SC author 出口设定 `CLAUDE_CODE_EFFORT_LEVEL`，与该字面义务**冲突**。

**处置：走 OpenSpec 显式裁决，不静默违反。**

- 本 change **新增** delta `specs/session-policy-envelope/spec.md`，以 `## MODIFIED Requirements` 修改 REQ-ENV-HOST-01，**追加**一句显式例外（**主规格原文与既有 3 个场景逐字保留**）：

  > 受信执行策略变量（provider 官方声明的单一档位变量，如 `CLAUDE_CODE_EFFORT_LEVEL`）MAY 由平台在**调用方显式 env map 覆盖层**内设定为该 provider 的受控值；该例外 MUST NOT 构造或覆盖其余子进程环境、MUST NOT 涉及凭据、隔离或网络命名空间、且 MUST NOT 依模型标签或宿主设置重算。原「平台不构造、不覆盖子进程环境」的义务对本变量以外的环境保持不变。

- 同步新增一条场景「受信执行策略变量只在覆盖层内设定」，承载「平台设定受信执行策略变量不得影响其余环境继承」。
- **可达性依据**（裁决 §3.1(2) / §3.2）：caller 注入**有可达路径，不用改全局 adapter 默认** —— `run/gateway_start.rs:648-678` 原样透传 caller env；`provider_gateway.rs:1998-2001` 保留 caller 显式覆盖；Claude direct / validated 的真实 spawn 消费 `input.env_vars`。因此本例外落在**既有 caller 显式覆盖层**语义内，而非新增平台 env 构造能力。
- **禁止**直接改 `openspec/specs/**`（主规格由归档时的 delta 自动同步）；**禁止**把该例外做成「仅本 change 的实现细节」而不进规格。

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

## 回退（裁决 §3.5）

- **策略回退**：新生成策略可经受控版本（`policy_version`）改回原档位或改回旧继承语义；**新旧运行各按原冻结策略结束** —— 已被冻结的原运行 MUST NOT 被回退动作改档，回退只影响此后新建的 session record。
- **不热切**：回退 MUST NOT 热切在途 provider（不 kill / 不重启正在运行的 provider 会话），在途运行按其已冻结策略自然结束。
- **不清预算**：回退 MUST NOT 清理或重置任何原运行的授权、预算、预留与授权层 attempt 事实。
- **旧二进制不得直接回滚**：新增的冻结字段落在 `WorkspaceSessionRecord`（**无 `deny_unknown_fields`**）上，旧二进制读该记录时会**静默忽略**未知字段 —— 回滚到旧二进制会让冻结事实在持久层被「读出但不可见」，进而使「按原运行冻结」在回滚后失效。因此**旧二进制不得直接回滚**；如需回退二进制，须先受控地收敛冻结字段的可见性（裁决 §5.6 同口径）。

## 共享面与排序（跨 change）

> 本 change 与其它 change 的写面交集在此登记；**共享文件的接线由集成 owner 串行执行**，任何一方都不得并发改同一文件。登记为**双边**：本表列出本 change 与哪些包共写同一文件、相对次序如何。

### 与 `bound-sc-generation-teaching` 的共享面（次序：**bound → effort**）

| # | 共享面 | 本 change 的写入 | 相对次序 |
|---|---|---|---|
| 1 | `src/product/models/mod.rs`（新模块注册） | 注册 `sc_launch_policy` | bound 先注册 `sc_generation_attempt`，effort 后注册；**先注册者先落**，不并发编辑 |
| 2 | `src/product/models/workspace.rs`（SC 字段区 `:843-850` 附近） | 冻结字段（`ProviderEffortConfig`） | **bound → effort**：bound 落窗口字段，effort 落 SC 冻结字段，避免两次 serde 迁移 |
| 3 | `src/product/lifecycle_store/workspace.rs`（创建 `:324-335` / 接管 `:455-470` 创建调用 + `:485-499` 复制） | 冻结策略写入与逐字段继承 | **bound → effort → isolate**：先合 bound 的窗口建立 / 接管只引用原窗口，再合 effort 的冻结继承，再合 isolate 的 `run_history` 创建点与代际 |
| 4 | `src/product/workspace_engine/types.rs`（`:84-102` runtime / `:149-168` 重建） | 冻结字段贯穿 runtime↔durable | **bound → effort**：窗口 / ledger 同步先落，冻结字段后落 |
| 5 | `src/web/workspace_ws_handler/run/single_candidate.rs`（初轮 `:1096-1112`、教学 `:510`） | SC author 出口的 env 装配 | **决策六拆分 → mechanicalize → bound → effort → isolate** |
| 6 | `src/product/workspace_engine/prompts.rs`（共用 builder `:464-548`、`:419` 其它 author 出口） | SC author 段抽离 | **决策六拆分 → effort → isolate**：effort 抽 SC author 出口，isolate 后落 spec/plan 域教学 |
| 7 | `src/web/workspace_ws_handler/socket.rs`（fresh admission `:1413-1427`） | 服务端侧冻结策略引用 / 摘要核验 | 本 change 只**新增服务端侧**核验，**不改**既有客户端 provider 比较条件，也不顺带加 `review_rounds` |
| 8 | 通道 C 的**冻结输入摘要** | effort 策略摘要（provider + policy_version + author_effort + digest）**填值** | **bound → effort**：字段由 bound 提供、effort 只负责填值 |

> **第 8 项的字段归属（依赖只登记在单侧的问题已消解）**：冻结输入摘要**字段本身**由 `bound-sc-generation-teaching` 的 **REQ-WSC-28B / D1** 提供（次序 **bound → effort**），本 change **只负责填值**，MUST NOT 在本 change 内定义 effort 语义，也 MUST NOT 修改 bound 目录的文件。

### 与 `isolate-plan-criterion-namespace` 的共享面（次序：**effort → isolate**）

- 共享文件：`src/product/lifecycle_store/workspace.rs` 的**创建 / 接管段**（创建 `:324-335`、接管 `:455-470` / `:485-499`）。
- 次序：**effort 先落**（冻结策略的写入与逐字段继承），**isolate 后落**（`run_history` 创建点 `run_history: Default::default()`（`:325`）与 `run_history.clone()`（`:485`）的代际字段）。
- 隔离要点：isolate 只改 `run_history` 相关代际字段，MUST NOT 触碰冻结字段；effort MUST NOT 改 isolate 的代际语义。

### 与 `close-contract-findings-before-pass` 的共享面

- **无直接共享写面。** close 的写面是 `work_item_plan_policy/classify.rs`（isolate → close 串行共写）、`review/routing*.rs` / `review/closure_policy.rs`（mechanicalize → close）与 REQ-WSC-17 / REQ-TOP-07 的 delta，均与本 change 的写面不重叠。
- 本 change 与 close 的唯一间接关系：close 消费 isolate 的版本化 scope，而 isolate 的 `lifecycle_store/workspace.rs` 写面在本 change 之后落（见上节次序）。

### 与 `mechanicalize-plan-compile-repair` / `fix-author-rewrite-quota-pool` 的关系

- 二者的 MODIFIED REQ-WSC-09 / REQ-WSC-21 归属**在各自 change 目录内**；本 change **不修改** `openspec/changes/mechanicalize-plan-compile-repair/**` 与 `openspec/changes/fix-author-rewrite-quota-pool/**` 的任何文件，本 change 的 delta 只写**不与二者重复**的部分。
- 代码面上二者与本 change **经决策六拆分后的模块**才分离（mechanicalize 的代码面含 `run/single_candidate.rs`），故**决策六的纯移动拆分必须先落**。

### 需集成 owner 执行的事项（跨 change，非本 change 文件）

1. `bound-sc-generation-teaching` 的 SC generation window **冻结输入摘要**中追加 effort 策略摘要（provider + policy_version + author_effort + digest），使教学重驱按原运行的冻结策略执行，而非当前默认。**该字段由 bound 的 REQ-WSC-28B / D1 提供（次序 bound → effort），本 change 只负责填值**，不修改 bound 目录的文件。
2. `mechanicalize-plan-compile-repair` 的 MODIFIED REQ-WSC-09（加入 typed IR 与 structure）**归该 change 的 owner**，在本 change 目录内**不做**任何改动；本 change 的 delta 与其无重叠条款。
3. 本 change 归档时，各 change 的 `work-item-plan-single-candidate` delta 需各归各 requirement（REQ-WSC-29 / REQ-WSC-28 等）互不覆盖；本 change 新增的 `session-policy-envelope` delta 归 REQ-ENV-HOST-01，与主规格原文逐字并存。

## Risks / Trade-offs

| 风险 / 取舍 | 处置 |
|---|---|
| `high` 的质量与耗时收益**均未实测** | 本期只冻结口径；不承诺时长 / token 降幅；真实对照需另行授权后实测（边界 3） |
| 组织 cap 可能把 effective 压到 `high` 之下 | requested / effective 分离；effective 无证据即未知，MUST NOT 冒称 `high` |
| 存量运行无法证明原 effective | 保留旧继承 + 披露未知（边界 2）；MUST NOT 追溯冻结 |
| 出口分流需要改动共用 builder 的调用面，可能误伤 legacy 家族 | 出口分流 + 作用域矩阵验收：legacy outline / PlanRevision / Draft/Batch 的 env **必须仍为空**；builder 内基线注入不受影响 |
| 冻结字段进入 durable record 触发 serde 迁移 | 一律可缺省 + 跳过序列化；旧记录反序列化不变；读旧 record **不写回** |
| `WorkspaceSessionRecord` 无 `deny_unknown_fields`，旧二进制回滚会静默忽略冻结字段 | **旧二进制不得直接回滚**（回退段）；回退须经受控版本收敛 |
| 策略版本演进后老记录不可读 | 未知 `policy_version` ⇒ fail-closed（决策 5），不静默按当前默认启动 |
| 与主规格 REQ-ENV-HOST-01「平台不注入」字面冲突 | 走 OpenSpec 显式裁决：新增 `session-policy-envelope` delta 追加受信执行策略变量例外（决策 7），**不静默违反** |
| 不扩公共 DTO 意味着 effort 暂不可编辑 | 接受；可编辑化另立完整公共面设计，不在本 change 暗开 |
| 与 `bound-sc-generation-teaching` 的窗口摘要接线跨 change | 登记在「共享面与排序」，由集成 owner 统一执行（字段归 bound 的 REQ-WSC-28B / D1） |
| 真实 provider 调用未授权 | 全部真机项写成「获授权后执行」，本期只跑离线矩阵（边界 3） |
