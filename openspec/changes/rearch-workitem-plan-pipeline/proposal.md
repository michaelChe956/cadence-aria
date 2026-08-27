## Why

workitem 段（issue→story→design→**workitem**→coding→交付的第四段）在 2026-08-25 首次全流程实测中 4/4 未能到达 Confirmed，暴露的是架构性不收敛而非单点 bug：

- **rep1**：draft review 判 `needs_human` 后，自动化路径无人类入口，driver 与服务器形成 accept→重审死循环，11 轮空转 25 分钟直至硬超时
- **rep2**：review 门禁要求「可执行规格」级契约完备度，语料每补一层就暴露更深一层缺口，永不收敛
- **rep3**：仅 `review_decision` 一个节点就有两套选项语法（pass+optional 二选一 / revise 三选一），自动化驱动需逐形状试错
- **rep4**：弱模型产出的 draft 自相矛盾（non_goals 禁测试却要求写测试），只能靠人工裁决，而人工裁决通道在自动化下不存在

根因两条：① **reviewer 的判决被当成状态机命令**（`review/routing.rs` 直接把 Pass/Revise/NeedsHuman 映射为阶段跳转）；② **执行策略（batch/serial）被建模成产品决策**，要求用户/WS 应答。

外部调研（Kiro、OpenAI Codex、Claude Code、spec-kit、OpenSpec、Aider、DeepSeek Harness）显示业界已收敛到同一形态：**LLM 写文档、确定性工具驱动状态、人当关键门**。OpenSpec 官方即因「刚性阶段不收敛」放弃锁相状态机，其内部架构（markdown 源文件 → 结构化解析 → ArtifactGraph）正是本提案采用的编译器模型的现成先例。

本变更同时服务三个目标：架构半重构消除过度编排、提升速度效率、提升弱模型成功率。

## What Changes

### 编排层：多阶段对话工作流 → 单候选计划事务

- 对外流程压缩为 `prepare → generate → evaluate → approval → completed` 五类可见状态（另加吸收态 `failed`）
- outline/draft/batch 的逐段确认、逐段 review、模式选择弹窗全部取消；这些步骤降级为 `generate` 内部的实现细节（第一版可保留内部多次 provider 调用）
- batch/serial 由运行时按模型能力与候选规模自选，不再暴露为 WS/UI 决策
- reviewer 降级为证据提供者：findings 完整记录进产物，**不再直接驱动状态跳转**
- 新增中央策略层：所有产出归入四类 typed outcome（`valid / repairable / human_required / fatal`），不允许模型 severity 直接决定跳转
- 防死循环铁律：finding 稳定指纹，相同指纹重现立即终态；整次运行有 repair budget 与 transition budget
- `must_fix` 类问题最多触发 **1 次**聚合自动返修，未解决进入唯一人工门
- 本阶段复用阶段 1 的唯一人工门与持久化快照；对话流式交互（复杂反馈走输入框、批准/终止用行内选项）归阶段 3，本 change 不实现
- 运行策略在 session 创建时固定并持久化：交互模式手动最终批准；campaign 模式显式 `auto_if_valid`

### 产物层：markdown 编译器模型（C′）

- 人和 LLM 只接触 markdown/EARS 源文件（`work-item-plan.md`）；弱模型不再直接产出私有深层 JSON
- 确定性编译器单向编译 markdown → 顶层 `PlanCandidateIr { source_revision_hash, compiler_version, items: Vec<PlanCandidateItemIr> }`；逐 work item 的 `PlanCandidateItemIr { target_repository_id, contract: CanonicalWorkItemContract, verification_plan: WorkItemDraftVerificationPlan, trusted_commands }`。依赖关系由 `contract.depends_on` 携带并沿用既有派生，不新造 DependencyGraph 类型
- 字段四来源矩阵：markdown 明写（作者语义）/ session 与已确认上下文 / 编译器确定性派生 / compile 事务运行时生成——每个 canonical 字段有且仅有一个来源
- IR 携带 `source_revision_hash` + `compiler_version`；**freshness 信任边界固定在 publish**：发布前校验 hash/version + typed validator 全绿，写入不可变 publication provenance；coding 只消费已发布的 immutable runtime binding，执行期间永不解析 markdown（coding 零改动）

### prompt 层：分层职责收敛

- 删除行为教学层（B 层）：目标仓库经 Cadence-skills 注入 Superpowers/OpenSpec/项目规则，「怎么工作」由仓库侧单一来源承载，aria prompt 不再重复
- 保留并强化产物规格层（C 层）：任务上下文 + 输出语法 + 边界约束 + **真实判例 few-shot**（以 rep2/rep3/rep4 的 9 个真实 findings 与 rep1 round-1 的 2 条 Advisory suggestion 为阶段 1 14 条 golden 素材）
- reviewer prompt 重校准：must_fix 只给机械漏网硬错误与明确自相矛盾；完备度类意见一律降 advisory；每条 finding 附带归类建议供策略层消费

### 迁移策略

- 新旧路径经持久化 `flow_kind` 并存（serde default=legacy），rollout flag 只在 session 创建时读取一次
- **compile 事务接入**：先从 `run_work_item_plan_compile` 提取 `InitialPlanCompileInput`（legacy 从 store 组装、新路径从顶层 PlanCandidateIr 的 items 组装，事务语义不变）。`logical_targets` 对齐现状为 `Option<BTreeMap<LogicalRepositoryId, String>>`；`CompileStores`/`PreparedInitialPlanCompile` 的字段以实施时 `compile.rs` 现状为准，计划不锁定。提取前先补 legacy 行为对照（parity）测试
- 多仓 Issue 的回落仅允许发生在新路径**任何副作用之前**的确定性 preflight：尚未创建/持久化新路径记录、尚未写 markdown/IR/run history、且未启动 provider 时，才可选择 legacy。preflight 通过后，一旦持久化任何新路径状态或启动 provider，失败必须写入该新路径的 durable fatal/recoverable 终态；不得静默切换 `flow_kind` 或回落 legacy（单仓 MVP 边界）
- codex+pi campaign 实测达标 + 恢复测试通过 + 阶段 1 的 14 条 classifier golden（11 原始 + 3 标注变体）全部分类正确后，才在阶段 4 删除旧 WS 协议与中间状态结构；仅 grammar/lowering finding 另有 compiler diagnostic golden，其余仅为 prompt few-shot 素材

## Capabilities

### New Capabilities
- `work-item-plan-single-candidate`: 单候选计划事务——markdown 编译器模型（C′）、字段四来源矩阵、publish 边界 freshness、唯一人工门、运行策略持久化（策略层/终态矩阵在阶段 1 `workitem-typed-outcome-policy` 定义，本 change 复用）

### Modified Capabilities
（无——本变更以新能力承载新流程；旧能力的退役在第四刀单独评估，届时再出 delta）

## Impact

- **编排**：`src/product/workspace_engine/`（plan_outline/draft_batch/decisions/review）、`src/web/workspace_ws_handler/`（decisions、protocol、mapping）、`src/web/workspace_ws_types/`
- **新增**：markdown grammar 定义、markdown→IR 确定性编译器、中央 policy/outcome 层、fingerprint 计算
- **复用不动**：`work_item_split_validator`（校验规则不改，仅输入来源换成编译产物）、`workspace_engine/compile.rs`（事务语义不改，仅提取注入式输入）、lifecycle store、coding engine/WS
- **前端**：本 change 不改 ChatWorkspace 对话流人工门、长文本输入弹窗或 `WorkItemPlanOptionsDialog`；这些 UI/最小 WS 协议属于阶段 3。阶段 2 仅保证后端复用阶段 1 的 gate snapshot/终态矩阵
- **prompt**：`work_item_split_engine/prompts.rs` 大瘦身（删 B 层行为教学与 JSON 格式对抗内容）
- **验证**：复用 `cadence/reports/workitem-coding-campaign/` 两个 campaign driver（需适配新协议）+ 阶段 1 的 14 条 classifier golden；compiler diagnostic golden 仅覆盖明确的 grammar/lowering 例
- **外部依赖**：无。prompt 分层依赖 Cadence-skills 既有注入内容即可（现状已覆盖），不要求 Cadence-skills 变更
