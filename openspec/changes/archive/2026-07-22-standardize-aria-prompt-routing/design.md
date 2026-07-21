## Context

Aria 同时存在 Workspace author/reviewer、WorkItemPlan、Coding Workspace、Runtime Unit 等多条 Provider prompt 路径。它们的输出协议和权限边界不同：有些只能产出候选 artifact，有些只能只读审查，有些可在受限 worktree 中编码。现有 WorkItemPlan 和 Coding prompt 已经拥有 `[openspec_contract]`、`[superpowers_contract]`、材料驱动执行协议和结构化输出契约；`prompt_template_registry` 还拥有候选产物与 daemon writeback 边界。

Cadence-skills 的 `agent-routing-kernel.md` 和 `openspec-superpowers-workflow.md` 是流程权威。Aria 需要让每个实际 agent 入口直接遵守它们，但不能把整份规则文本反复塞入 prompt，也不能以新的内部状态机、规则副本或 metadata 模型替代它们。重复的路由提示还会污染同会话格式修复的 JSON/sentinel 输出。

## Goals / Non-Goals

**Goals:**

- 所有新建或恢复的 Aria agent 入口直接引用并遵守两份 Cadence 原始规则，按实际阶段执行原生 Skill 路由、OpenSpec/Plan 前置条件和人工 gate。
- 保留现有 OpenSpec/Superpowers 约束、候选产物写入权、traceability、结构化输出、Provider resume 与审查语义。
- 仅在需要启动或重新进入工作流的 prompt 中注入最小、明确的规则指令，避免对模型造成重复或冲突的流程要求。
- 为每一类 prompt 生命周期建立回归测试，防止后续修改丢失规则、改变已有契约或把路由误注入格式修复。

**Non-Goals:**

- 不新建 Aria 专有的路由体系、规则 DSL、状态字段、数据库表或 API。
- 不复制、改写或外置 Cadence 原始规则文本；不以 prompt 声明伪造已完成的 Skill 调用。
- 不更改 Aria daemon 的 canonical OpenSpec writeback、审批 gate、Provider 协议、artifact schema 或前端状态机。
- 不把 `cadence-workflow`、Hook、插件或阅读状态机引入执行路径。

## Decisions

### 1. 直接引用原始规则，而不是建立替代规则层

每个需要路由的既有 builder 使用一个最小的共享文本插入工具。该工具只输出两份原始规则的路径、当前任务阶段、必须遵守的原始路由行和禁止依赖 `cadence-workflow` 的提醒；它不保存规则副本、不解释规则、不持久化状态，也不判断实施完成。

完整规则文本不嵌入 prompt。这样 agent 必须实际读取权威文件，同时避免为每个 Provider 请求重复数千字规则导致上下文稀释。

曾考虑将场景、Skill、gate 和状态完整建模为 Aria 的新注册表。该方案会与既有 Cadence 规则形成第二权威来源，且扩大状态机和迁移范围，因此不采用。

### 2. 按 prompt 生命周期而非文件名决定注入

路由提示只注入以下请求：新建 Story/Design/Work Item/WorkItemPlan 会话、新建 Coding/Tester/Reviewer/Group PR Review 请求、Runtime Unit 的独立 Provider 节点，以及明确的 Provider resume 或重新进入实施请求。

同一 Provider 会话中仅为修复 JSON、nonce sentinel、artifact fence 或结构化输出解析失败而发送的 follow-up 不注入路由。它们必须只修复已有输出，保留原来的输出 schema 和解析边界。若 follow-up 实质改变任务、范围、架构或验收，则将其作为规则规定的阶段转换或 resume，而非格式修复。

### 3. 保留既有契约，并定向补强缺口

`work_item_plan_runtime_contract` 的 confirmed Story/Design、traceability、`writing-plans`、最少拆分、TDD 和验证计划约束保持原样；`provider_runtime_contract` 及 Coder/Reviewer/GroupFinalReview 材料协议保持原样；N04/N05/N07/N11 的候选 artifact、daemon writeback 和结构化输出约束保持原样。

规则插入位于这些既有角色和权限契约之前，或以不改变原字段和输出顺序的相邻段落插入。Story/Design author 与 reviewer、通用 revision、Coding full/delta、CodeReviewer、GroupFinalReview、Tester、Runtime N04–N27 等缺少直接路由说明的入口才新增该段落。已有 `workflow_discipline` 不会被删除；与 Aria 的非交互候选职责并列时，明确由既有 Aria human-confirmation gate 承接规则要求的确认，Provider 不得越权写 canonical artifact。

### 4. 阶段选择以原规则表为准

Story/Design 的创作与方案讨论使用 `using-superpowers → brainstorming`；设计确认后由 Aria 现有 gate 和 daemon 持久化 OpenSpec proposal/design/specs/tasks；已获批的 OpenSpec 契约进入 WorkItem/Plan 时使用 `writing-plans`；编码和 bounded rework 在确认 Plan 后使用执行与 TDD 规则；测试、Code Review、Group PR Review、集成验证、最终总结分别遵守验证、审查、sync/archive 和分支收尾规则。

每个 prompt 只声明当前阶段与紧邻的前置 gate，不能在同一请求内罗列整个生命周期。这样不会要求 reviewer 重新 brainstorming，也不会要求格式修复重新做 verification。

### 5. 测试以文本契约和负向隔离为主

现有 prompt 测试扩展为三组：新建/恢复入口含原规则路径、正确阶段和必调 Skill；既有 `[openspec_contract]`、`[superpowers_contract]`、材料协议及输出 schema 仍在；格式修复和 artifact retry 不含新路由回执要求。测试同时断言 prompt 不包含 `cadence-workflow`，并覆盖 Story、Design、WorkItem、Coding、Code Review 与组级 PR Review 的代表入口。

## Risks / Trade-offs

- [规则路径在 Provider 工作目录不可读] → 使用用户已确认的 Cadence-skills 前置环境；若原始规则不可用，prompt 要求停止并报告，不以摘要替代。
- [每个节点重复完整规则造成上下文稀释] → 仅插入路径、阶段和原规则行；不内嵌规则正文。
- [新路由覆盖现有 output contract] → 维持既有角色、权限和 schema 段落，新增测试锁定其内容与顺序。
- [续跑误判为新任务] → 以调用原因区分 resume/阶段转换与纯格式修复，后者不插入路由。
- [非交互候选节点与用户确认规则冲突] → 保留 Provider 只产出候选、Aria gate 承接确认、daemon 写回 canonical artifact 的既有职责边界。

## Migration Plan

1. 先为规则插入文本建立单元测试和最小共享工具，确保其仅引用权威文件且不定义新流程。
2. 分组接入 Workspace、WorkItemPlan、Coding/Review 和 Runtime Unit builder，并在每组后运行定向 prompt 测试。
3. 运行全量 Rust 格式、检查与测试，检查 Story、Design、Work Item 三类 Workspace 共享链路未受回归影响。
4. 若某个 Provider 因新增路由提示破坏结构化输出，回滚该 builder 的插入点；保留既有合同并把该情形作为需重新设计的生命周期分类，而非在 prompt 中增加更多重复规则。

## Open Questions

- 无。用户已确认：Cadence 原始规则为唯一流程权威，Aria 现有 gate 承接人工确认，且本次不建立替代路由体系。
