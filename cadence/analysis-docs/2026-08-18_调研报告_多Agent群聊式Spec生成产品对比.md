# 调研简报：多 AI Agent「群聊式」协作生成软件规格/设计文档

> 场景：本地开发工具中，用户与固定角色 agent（author / reviewer / researcher / 前端设计 / 后端设计）在同一聊天室，通过讨论产出 Issue 澄清、Story Spec、Design Spec；机制参考 yetone/cumora（triage 路由、seen-cursor 新鲜度门控、原子 claim、循环熔断、agent 互谈）。

## 摘要

业界已有大量「多角色 agent 协作产出软件文档」的实践：MetaGPT/ChatDev 用 SOP 流水线（非自由群聊）从一句话需求生成 PRD/设计/代码；AutoGen/AG2 GroupChat、CrewAI hierarchical、LangGraph supervisor/swarm、OpenAI Agents SDK handoff 提供了成熟的「谁发言/谁接管」仲裁模式；cumora 则是把 agent 作为聊天室一等公民、带认领与防碰撞机制的代表。学术与实践证据一致表明：**同等 token 预算下多 agent 讨论常常不优于强提示单 agent**，且存在发散、附和（sycophancy）、成本放大等已知失败模式——解法是强结构（SOP/路由/仲裁）、轮次与发言预算、及早收敛判定，而非放任自由讨论。

---

## 1. 产品/项目盘点与关键机制对比

### 对比表

| 产品/项目 | 角色划分 | 仲裁机制（谁发言/防抢话/收敛） | 产物定稿方式 | 出处 |
|---|---|---|---|---|
| **cumora** | agent 是聊天室一等成员，有人格、私有记忆/workspace，可主动发言、认领工作（claim 机制防碰撞） | 定时 turn + 事件触发；agent 间协调「claim 工作、互不冲突」；triage 路由 + seen-cursor 新鲜度门控 + 循环熔断 | 聊天内产物 + 看板（Kanban）任务；agent 在自己的 workspace 产出文件 | [GitHub](https://github.com/yetone/cumora)、[官网](https://cumora.ai/) |
| **MetaGPT** | 产品经理/架构师/项目经理/工程师，模拟软件公司 SOP | **不做自由群聊**：SOP 编排的角色流水线，结构化中间产物（"Code = SOP(Specs)"），以人为工作流抑制级联幻觉 | 每个角色输出结构化文档（PRD、竞品分析、数据结构、API、设计文档），下游角色消费上游产物 | [GitHub](https://github.com/foundationagents/metagpt)、[论文](https://arxiv.org/html/2308.00352) |
| **ChatDev** | CEO/CTO/程序员/评审等，组成虚拟软件公司，按瀑布阶段结对 chat | **ChatChain**：阶段化任务分解，每阶段由两个 agent 的双 agent 对话完成（如 coding-review），有 "instructor/assistant" 角色，防多 agent 抢话的本质是永远只有两人在谈 | 每阶段产出文档/代码，写入版本化软件产物 | [论文 ACL 2024](https://aclanthology.org/2024.acl-long.810/)、[GitHub](https://github.com/OpenBMB/ChatDev/) |
| **CAMEL** | task-specify agent + task-planner + role-playing 双 agent（user/assistant 角色）+ 可选 critic-in-the-loop | **Inception prompting** 强制双 agent 轮替持续对话；critic 按给定 criteria 评审收敛 | 对话轨迹本身即产物；可导出为数据集/任务轨迹 | [论文](https://arxiv.org/abs/2303.17760)、[societies/role_playing.py](https://github.com/camel-ai/camel/blob/8a75a567/camel/societies/role_playing.py) |
| **AutoGen / AG2 GroupChat** | 任意多 agent + 特殊 `GroupChatManager` | Manager 在每条消息后统一选择下一发言者：`auto`（LLM 选择）/`round_robin`/`manual`/自定义 `speaker_selection_func`（可返回 None 结束）；`max_round` 熔断、`is_termination_msg` 终止条件 | 广播式消息历史；产物由指定 agent 写文件，无内建定稿仪式 | [AG2 GroupChat](https://docs.ag2.ai/latest/docs/user-guide/advanced-concepts/groupchat/groupchat/)、[自定义选择](https://microsoft.github.io/autogen/0.2/docs/notebooks/agentchat_groupchat_customized/)、[v0.2 源码](https://github.com/microsoft/autogen/blob/v0.2.16/autogen/agentchat/groupchat.py) |
| **CrewAI** | Crew = agents + tasks；hierarchical 模式有 manager agent | **hierarchical process**：manager agent 负责委派、校验、去重；普通 agent 可 `allow_delegation`；sequential 模式按任务顺序执行，天然无抢话 | Task 输出（`expected_output` 定义），最后任务输出即最终产物 | [Hierarchical Process](https://docs.crewai.com/en/learn/hierarchical-process) |
| **LangGraph multi-agent** | supervisor（把 subagent 当工具调用）或 swarm（agent 间 handoff） | **Supervisor 模式**：主 agent 路由+汇总，subagent 无状态、上下文隔离；**Swarm 模式**：handoff 转移控制权。LangChain 实测 supervisor 改进后 Tau-bench 提升约 50%；supervisor 比 swarm 多 1 次 LLM 调用（路由+专家）但控制更强 | supervisor 汇总各 subagent 结果形成最终输出；checkpointing 支持暂停/恢复 | [架构选择](https://www.langchain.com/blog/choosing-the-right-multi-agent-architecture)、[基准](https://www.langchain.com/blog/benchmarking-multi-agent-architectures)、[Supervisor vs Swarm 实测](https://dev.to/focused_dot_io/multi-agent-orchestration-in-langgraph-supervisor-vs-swarm-tradeoffs-and-architecture-1b7e) |
| **OpenAI Swarm / Agents SDK** | 通用 agent + handoff；三种模式：handoffs / agents-as-tools / manager-as-orchestrator | **Handoff 即工具调用**（`transfer_to_xxx_agent`），控制权显式转移、共享消息上下文；或 manager 保持控制权把 specialist 当工具调 | 拥有最终用户可见回复权的 agent 定稿（"who owns the final user-facing answer"） | [Orchestration 指南](https://developers.openai.com/api/docs/guides/agents/orchestration)、[Agents SDK handoffs](https://openai.github.io/openai-agents-python/handoffs/)、[AutoGen 对 Swarm 的实现](https://microsoft.github.io/autogen/stable/user-guide/agentchat-user-guide/swarm.html) |
| **Claude Code subagents / 波次编排** | 主会话 = 纯 orchestrator，背景 subagent 做勘察/实现/审查（superpowers 的 subagent-driven development 即此模式） | 主 agent 计划并委派，subagent 各自独立 context 只回传摘要（上下文隔离）；依赖图分波（waves）并行执行；质量门禁（gate）后勾选工作包 | 主 agent 校验后合并；每个 subagent 返回摘要，最终由 orchestrator 定稿 | [官方 subagents 文档](https://code.claude.com/docs/en/sub-agents)、[波次编排社区实现](https://github.com/am-will/swarms)、[dynamic workflows](https://claude.com/blog/introducing-dynamic-workflows-in-claude-code) |
| **GitHub Spec Kit** | 单 agent + 结构化 slash 命令（specify/clarify/plan/tasks/analyze），非多 agent | `/speckit.clarify` 由 agent 向用户提澄清问题并写入决策；Workflow 引擎支持条件/循环/fan-out/人审 checkpoint，可暂停恢复 | Constitution + spec.md / plan.md / tasks.md 三件套定稿，用户显式接受 | [Spec Kit 文档](https://github.github.io/spec-kit/)、[Quickstart](https://github.github.io/spec-kit/quickstart.html)、[Workflows](https://github.github.io/spec-kit/reference/workflows.html) |
| **OpenSpec (OPSX)** | 单 agent + 变更目录产物（proposal/spec/tasks）+ 用户自定义模板 | 无多 agent；用目录结构 + 依赖关系约束产物生成，AI 逐产物测试 | change 目录（proposal/spec/design/tasks）经人审后 archive | [OPSX Workflow](https://openspec.dev/docs/opsx)、[Spec Kit vs OpenSpec](https://intent-driven.dev/knowledge/spec-kit-vs-openspec/) |

### 要点解读

1. **两种范式**：「自由群聊 + 仲裁」（AutoGen GroupChat、cumora）vs「结构化流水线/SOP」（MetaGPT、ChatDev、Spec Kit、波次编排）。研究与实践共识是后者更可控、更省钱；前者人机体验更好但需要额外收敛机制。
2. AutoGen GroupChat 的四种 speaker selection（auto/round_robin/manual/自定义函数，可返回 None 提前结束）+ `max_round` 是最成熟的「防抢话 + 熔断」参考实现，几乎可以映射到 cumora 式 triage。
3. OpenAI Agents SDK 明确三选一决策框架：handoff（专家接管）适合强角色分工；agents-as-tools（manager 保持控制）适合产物定稿责任集中在主 agent 的场景。
4. ChatDev 的「双 agent 结对对话完成每个阶段」是控制发散的低成本技巧：任何时刻房间里只有两人在谈。
5. Claude Code subagents 的「subagent 独立上下文、只回传摘要」是 token 成本控制的关键模式（避免全量历史广播给所有角色）。

---

## 2. 多 agent 讨论 vs 单 agent 多轮：效果与成本结论

1. **同等预算下单 agent 常可打平或胜出**：ACL 2024 论文《Rethinking the Bounds of LLM Reasoning》系统实验表明，强提示的单 agent 可在广泛推理任务上达到与最佳多 agent 讨论方法几乎相同的性能（[ACL](https://aclanthology.org/2024.acl-long.331/)）。另有信息论视角（数据处理不等式）的研究：固定推理 token 预算下，单 agent 在多跳推理上优于多 agent 系统（[arXiv 2604.02460](https://arxiv.org/abs/2604.02460)）。
2. **多 agent 的增益常被「更多测试时算力」混淆**：统一评测显示，把计算量归一化后 MAS 增益大幅缩水（[arXiv 2601.13243](https://arxiv.org/html/2601.13243)）。
3. **已知失败模式——发散不收敛**：
   - 同质多 agent 辩论中「Talk Isn't Always Cheap」记录了辩论随轮次**准确率下降**、错误传播等失败动力学（[arXiv 2509.05396](https://arxiv.org/html/2509.05396)）。
   - 同质 agent 无引导辩论不如「隔离的自我纠错」——附和（sycophancy）与偏见强化是主因；CONSENSAGENT 通过缓解附和提升共识效率（[arXiv 2605.00914](https://arxiv.org/html/2605.00914)、[ACL Findings 2025](https://aclanthology.org/2025.findings-acl.1141.pdf)）。
4. **收敛解法**：
   - 少数服从多数/加权投票、judge 聚合，但共享偏见时不可靠（[NeurIPS 2025](https://proceedings.neurips.cc/paper_files/paper/2025/file/42475c537936b2394b5015e871765056-Paper-Conference.pdf)）。
   - **自适应停止**：Wald-SPRT 序贯检验做「compute governor」，每轮由 judge 打共识分、累积达阈值即停，而非固定轮数（[arXiv 2605.19193](https://arxiv.org/html/2605.19193)）。
   - 协议设计：Within-Round（只看当前轮）vs Cross-Round（看全历史）对质量有显著影响，上下文可见性是收敛杠杆（[arXiv 2603.28813](https://arxiv.org/html/2603.28813)）。
   - 结构化 SOP（MetaGPT）本质上是把「收敛」问题转化为「产物依赖图」问题，从根上绕开发散。
5. **实践侧佐证**：CrewAI hierarchical 被指出增加一层推理、更多状态交接与 token 消耗，并引入「委派循环、责任不清」的新失败类；建议先用少量 agent、边界验证后再拆分（[activewizards](https://activewizards.com/blog/hierarchical-ai-agents-a-guide-to-crewai-delegation/)、[FRENXT 生产经验](https://www.frenxt.com/research/building-production-multi-agent-systems)）。

---

## 3. 「角色自主认领 + 智能路由」的现有实现

| 框架 | 机制 | 可借鉴点 |
|---|---|---|
| AutoGen/AG2 GroupChatManager | 每条消息后由 manager（LLM 或规则函数）选下一发言者；`speaker_selection_func(last_speaker, groupchat)` 可返回 agent / None（结束）/ "auto"（继续自动），加 `max_round` | 自定义选择函数 = triage 路由的标准挂点；返回 None 即「无人需要发言→收尾定稿」 |
| AutoGen SelectorGroupChat / Swarm | Swarm：handoff 作为特殊工具调用、agent 本地决策转移控制权，无中心 orchestrator | 「认领」可建模为 handoff 工具（`transfer_to_backend_designer`），无需中心裁判也能路由 |
| LangGraph Supervisor | supervisor 节点输出 `Command(goto=...)` 路由到 worker 节点，worker 完成回到 supervisor；StateGraph + checkpoint | 显式状态机路由 + 可暂停恢复；supervisor 只做路由可用小模型 |
| OpenAI Agents SDK | handoffs = 面向 LLM 的工具（`transfer_to_refund_agent`），agents-as-tools 由主 agent 调用 specialist 并保留控制权 | 「谁拥有最终产物」由架构显式决定，避免多人同时改文档 |
| CrewAI hierarchical | manager agent 委派任务、校验结果、去重；`allow_delegation` 控制谁能转交 | manager 默认用强模型、worker 用弱模型的分工是成本模式的现成模板（[docs](https://docs.crewai.com/en/learn/hierarchical-process)） |
| cumora | agent 主动 claim 工作（原子认领防碰撞）+ 定时主动发言 | 与任务队列结合的认领语义，最贴近你们的设计 |

成熟结论：**「路由」业界普遍做成显式节点/工具调用，而不是让所有 agent 抢答**；「认领」最常见的形式是 handoff 工具或 manager 委派，cumora 的原子 claim 是少数把认领做进聊天语义的实现。

---

## 4. 单机、token 成本敏感场景的降本模式

1. **上下文隔离 + 摘要回传**：Claude Code subagents 各自独立 context，只回传摘要给主会话，避免把全量群聊历史广播给每个 agent（[官方文档](https://code.claude.com/docs/en/sub-agents)）。这是群聊最直接的成本杠杆——cumora 式「各自订阅房间」应改为「按需注入 + 摘要」。
2. **分级模型**：路由/仲裁用小模型（甚至规则函数），只有产物撰写用强模型。CrewAI 官方即推荐 manager 与 worker 可用不同模型；LangGraph 生产实践建议「无 LLM 需求的节点用状态机 + 分层 prompt 缓存」（[FRENXT](https://www.frenxt.com/research/building-production-multi-agent-systems)）。
3. **轮次上限与发言预算**：AutoGen `max_round`、终止消息条件是内建熔断；研究侧的 SPRT 自适应停止（共识分够了就停）是更精细的「发言预算」（[arXiv 2605.19193](https://arxiv.org/html/2605.19193)）。
4. **可见性控制**：Within-Round 协议（agent 只见当前轮贡献而非全历史）能显著降低 prompt 长度并影响收敛（[arXiv 2603.28813](https://arxiv.org/html/2603.28813)）。
5. **流程化替代讨论**：MetaGPT/ChatDev/Spec Kit 的流水线/结对对话把 N×M 的全连接对话降为 O(N) 的顺序交接；LangChain 实测 supervisor 每次交互只多 1 次路由 LLM 调用（[DEV 实测](https://dev.to/focused_dot_io/multi-agent-orchestration-in-langgraph-supervisor-vs-swarm-tradeoffs-and-architecture-1b7e)）。
6. **「先两个 agent，验证边界后再拆」**：多条生产经验一致建议从 2–3 个 agent 起步（[FRENXT](https://www.frenxt.com/research/building-production-multi-agent-systems)、[activewizards](https://activewizards.com/blog/hierarchical-ai-agents-a-guide-to-crewai-delegation/)）。

---

## 对我们设计的可借鉴 / 应避免清单

### 可借鉴
- **仲裁显式化**：仿 AutoGen `speaker_selection_func`——triage 是一个可插拔函数，输入（last_speaker、房间状态、产物状态），输出「下一发言者 / 无人发言（进入定稿）」；支持 round_robin/规则兜底，路由用小模型或纯规则。
- **认领 = handoff/claim 语义**：cumora 原子 claim + OpenAI handoff-as-tool 是最省 token 的认领实现；seen-cursor 新鲜度门控可防止 agent 基于过期上下文重复发言。
- **产物定稿责任唯一**：借鉴 Agents SDK「谁拥有最终 user-facing 输出」的显式决定——author 角色独占写 Spec 文件的权限，reviewer/researcher 只产出评论/引用，从权限上消除「多人同时改稿」的冲突面。
- **结构化产物依赖（Spec Kit/OpenSpec）**：Issue 澄清 → Story Spec → Design Spec 做成带依赖与验收条件的产物流水线，讨论服务于产物，产物状态机决定何时进入下一阶段，天然防发散。
- **熔断与预算**：max_round、每 agent 发言预算（如每轮每角色 ≤1 条）、无实质新信息即沉默的指令约束；可加轻量「共识分」判定提前收束（SPRT 思路）。
- **上下文隔离**：各角色不共享全量历史，接收的是「与己相关的摘要 + 自己未读的关键消息」， reviewer 只需 diff 视角。

### 应避免
- **放任自由群聊 + 同质角色**：同质 agent 无引导辩论有实证的准确率下降与附和放大（arXiv 2509.05396、2605.00914）；角色提示词要有实质差异与唯一职责。
- **全量历史广播给所有 agent**：成本随轮数×人数平方级膨胀，是最常见的成本失控点。
- **无终止条件的开放式讨论**：必须内建「无人认领/无新信息 → 自动收敛定稿或升级人类」路径。
- **一上来做重层级编排**：CrewAI hierarchical 式额外 manager 层在单机场景增加 token 与委派循环风险；优先 supervisor-as-router（小模型/规则）+ 少量专家。
- **用多 agent 讨论换不回质量就退回单 agent**：ACL 2024 等证据表明强提示单 agent 常可打平——群聊的价值应定位在「人类可介入/可观察的协作体验」与「角色专业化的产物质量」，而非默认更高准确率。

## Sources

保留（核心证据）：
- yetone/cumora — https://github.com/yetone/cumora ；https://cumora.ai/
- MetaGPT — https://github.com/foundationagents/metagpt ；论文 https://arxiv.org/html/2308.00352
- ChatDev — https://aclanthology.org/2024.acl-long.810/ ；https://github.com/OpenBMB/ChatDev/
- CAMEL — https://arxiv.org/abs/2303.17760 ；role_playing 源码 https://github.com/camel-ai/camel/blob/8a75a567/camel/societies/role_playing.py
- AG2/AutoGen GroupChat — https://docs.ag2.ai/latest/docs/user-guide/advanced-concepts/groupchat/groupchat/ ；自定义发言者 https://microsoft.github.io/autogen/0.2/docs/notebooks/agentchat_groupchat_customized/ ；Swarm https://microsoft.github.io/autogen/stable/user-guide/agentchat-user-guide/swarm.html
- CrewAI hierarchical — https://docs.crewai.com/en/learn/hierarchical-process
- LangChain 多 agent 架构与基准 — https://www.langchain.com/blog/choosing-the-right-multi-agent-architecture ；https://www.langchain.com/blog/benchmarking-multi-agent-architectures
- OpenAI Agents SDK — https://developers.openai.com/api/docs/guides/agents/orchestration ；https://openai.github.io/openai-agents-python/handoffs/
- Claude Code subagents — https://code.claude.com/docs/en/sub-agents ；dynamic workflows https://claude.com/blog/introducing-dynamic-workflows-in-claude-code
- GitHub Spec Kit — https://github.github.io/spec-kit/ ；Workflows https://github.github.io/spec-kit/reference/workflows.html
- OpenSpec OPSX — https://openspec.dev/docs/opsx
- 单 vs 多 agent 证据 — https://aclanthology.org/2024.acl-long.331/ ；https://arxiv.org/abs/2604.02460 ；https://arxiv.org/html/2601.13243
- 失败模式与收敛 — https://arxiv.org/html/2509.05396 ；https://arxiv.org/html/2605.00914 ；https://arxiv.org/html/2605.19193 ；https://arxiv.org/html/2603.28813 ；https://aclanthology.org/2025.findings-acl.1141.pdf
- 生产经验 — https://www.frenxt.com/research/building-production-multi-agent-systems ；https://activewizards.com/blog/hierarchical-ai-agents-a-guide-to-crewai-delegation/

剔除：ABABNews 与 cephalochromoscope（cumora 的二手转述，信息量低于官方 README）、Cursor 论坛 feature request（无实现）、arXiv 2607.26212 MAD survey（仅摘要可及、与其他综述重叠）、NeurIPS 2025 自适应稳定性检测（细节未核对，仅用于投票聚合局限性结论）。

## Gaps
- cumora 内部 seen-cursor/熔断的具体实现细节（README 概述层面可确认机制存在，源码级参数未逐一核验）。
- 「多 agent 群聊生成 spec 文档」尚无针对该任务本身的公开定量基准（现有论文多为数学/推理基准外推）。
- 各框架在本地小模型上的路由准确率无统一数据；建议自建 A/B（单 agent vs 群聊）以同 token 预算对比产物质量。
