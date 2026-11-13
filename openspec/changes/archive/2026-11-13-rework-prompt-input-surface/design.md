# Design: rework-prompt-input-surface

> 权威来源：
> - 主方案 `cadence/designs/2026-11-08_技术方案_WIG生成加速与收敛性加固_v1.0.md` §3.4（一致性裁决 X2）、§6.3（两个平台侧确定性缺陷）、§6.6（G-4 / G-5 工作包行）、§8.0（待办登记 · MODIFIED REQ-WSC-06 无承载 change）；
> - 处置裁决 `cadence/designs-reviews/2026-11-09_裁决_WIG四change审阅问题处置_v1.0.md` §2.1（归档后主规格 500 字符门禁）、§2.2（共享面全局串行顺序，唯一权威）、§2.3（命名域与跨 change 归属的唯一性）、§7.2（X2 归属订正与「不得由 isolate 夹带」定案）；
> - 决策报告 `cadence/designs-reviews/2026-11-08_决策_WIG六项owner裁决_v1.0.md` §7（总落地表 G 行）、§7.1（集成约束 3「E 裁剪先改契约」、4「G 预算不混单位」）；
> - 实测证据 `cadence/analysis-docs/2026-11-08_WIG实测证据基线_v1.0.md`。
> 契约文本见 `specs/work-item-plan-single-candidate/spec.md`（MODIFIED REQ-WSC-06）；工作包见 `tasks.md`。
> 行号均以 `.worktrees/feat-wig-optimize-11-08/` 为根。

## Context

本 change 由两个独立缺口合并而成，共享同一条 requirement（REQ-WSC-06）与同一条主线：**把「本该由平台确定性给出的事实」从模型的自助探索与重复付费里收回来**。

### 缺口一：author 输入面（G-4 / G-5）与返修 prompt 复用

- **G-4（平台侧确定性缺陷，非模型行为）**：多仓目录摘要走 `summarize_repository_structure_logical`（`src/product/work_item_split_engine/context.rs:199-252`），顶层条目先 `top_files.sort()`（`:221`）→ 并入目录 → `top_level.sort()`（`:233`）→ `top_level.truncate(MEMBER_REALITY_MAX_TOP_DIRECTORIES)`（`:234`，上限 12，`src/product/logical_codebase/member_reality.rs:30`）。按名排序后，`.agents` / `.claude` / `.codegraph` / `.codex` / `.gitignore` / `.kimi-code` / `.mcp.json` 等点条目占满 12 个名额，**`busi` / `frontend` / `gateway` / `scripts` / `policy` 全部落榜**。实测 prompt 的 `structure` 段仅 **276 字符**（结构预算 3,200 B **只用了 340 B**），上述 5 个仓名在该段出现次数**均为 0**。这是模型 **9 次**探索的直接诱因。
- **G-5（上下文供给不足）**：`budget_markdown_context`（`src/product/work_item_split_engine/prompts.rs:395-414`）把 `story_context` / `design_context` / `repository_structure` 三段按固定比例（9/20、9/20、1/10 × `WORK_ITEM_PLAN_MARKDOWN_CONTEXT_BUDGET_BYTES` = 32,000）压进同一预算。作者拿不到**已被平台确认的成员仓摘要与依赖图事实**（member targets / change_order 已在 `WorkItemPlanMarkdownAuthorContext` 里装载，`:53-58`），模型只能自行 Read/Glob 补事实：实测首轮 author 在产出 markdown 前做了 **17 次串行工具往返**，探索期 **275.4 秒**。
- **返修 prompt 逐字节复用首轮全文**：`src/product/work_item_split_engine/prompts/sc_revision.rs:27-43` 的 `build_work_item_plan_markdown_revision_prompt` 先 `let base = build_work_item_plan_markdown_prompt(...)`（`:34`）拿到**首轮完整 prompt**，再在输出指令前插入 findings 回灌段。返修轮因此**每轮重付 32KB 静态规范面**（实测 run1 prompt **32,609 字符**；author 侧 `cache_read_tokens` 仅 **128** ⇒ 首轮 prompt 在重驱时完整重发一次）。
- **两侧结论不传递**：reviewer 侧 input **61,051 token**，而**作者的探索结论没有以任何形式传递给 reviewer**（不同 provider、不同会话、无共享上下文），reviewer 重复把仓库探索了一遍。

### 缺口二：X2 的 MODIFIED REQ-WSC-06 无 owner

（**本 change 建立时的状态** —— 本 change 即该缺口的承载方；REQ-WSC-06 由本 change **独占**，任何其它 change 不得再 MODIFIED 该 requirement。）

主规格 `openspec/specs/work-item-plan-single-candidate/spec.md:270` 的 REQ-WSC-06 场景群要求 reviewer 复评前获得的只读契约覆盖投影「**内容 SHALL 至少包括**：逐 WI→contract edge 的 required capabilities、所引契约输出 capabilities 与 compatibility_policy」（`:372`），且含「**节点依赖图事实**（depends_on/边/环/重复边/未知 provider）；handoff 消费闭环…；**跨 work item 写范围冲突事实**（exclusive/forbidden 重叠）」（`:377`）。

主方案 §3.4 的方案 E 裁剪把这三段压成 gaps-only + handoff_consumption + 同仓冲突 + PVR ⇒ **与该条字面义务冲突**。X2 裁定：必须带 MODIFIED REQ-WSC-06 delta 重定义投影最小内容，**不得静默裁**；「只留同仓冲突」须**显式收窄 `:377` 的无仓限定措辞并给理由**（跨仓 forbidden/exclusive 在 multi-repo 场景无意义，实测 **29 条写范围冲突中 27 条为跨仓假阳性**）；且「**信息仍可从 CCC 推导**」**不足以**满足「投影 SHALL 至少包括」的字面义务。

处置裁决 §7.2 定案：承载方**须另立 change**，**不得由 `isolate-plan-criterion-namespace` 夹带**（其 scope 是命名域 + 指纹代际，与 reviewer 投影裁剪不同物）。本 change 即该承载方。

## Goals / Non-Goals

**Goals**

- G-1：让 reviewer 侧的投影呈现只承载判断必需的四类内容，且该收窄**以规格条款生效**（不静默裁），并显式写明跨仓收窄的理由。
- G-2：让 L1（成员仓摘要）与 L2（依赖图事实）成为**平台直供**的 prompt 事实，模型不必自助探索才能拿到。
- G-3：让成员仓摘要不再被点条目挤掉（G-4 排序修正）。
- G-4：让返修轮只带**本轮 delta + 本轮相关既有上下文**，不再逐字节重发首轮完整 prompt，同时**不损返修能力**（findings 回灌、逐条修复指令、`required_action` 可执行对象）。
- G-5：一条 requirement（REQ-WSC-06）承载上述四项，主规格原有场景逐字保全，描述段 ≤500 字符（归档后主规格门禁）。

**Non-Goals**

- 不承载 G-2（MCP 输出截断）/ G-3（Bash 输出截断）/ G-6（缓存 TTL 1h）—— 属 SC author 出口 env 层，见下「不承载什么」。
- 不承载 G-1（effort 档位）—— 裁决 §3 已定案为默认 `high`，由 `explicit-sc-author-effort` 承担。
- 不改 canonical 校验器 fail-closed 语义；不删平台侧 source/IR/机械报告的绑定与同代校验（决策 §7.1 集成约束 3）。
- 不改 legacy/story/design reviewer 路径输入面（主规格既有「投影仅注单候选」场景）。
- 不 bump 任何 prompt 预算常量（决策 §7.1 集成约束 4：23,500 是整 prompt fixture 字节预算，G 包新增内容不得借「动态段」名义逃总预算）。
- 不改 REQ-PLN-17 的分层摘要与单仓逐字不变义务，只修多仓路径的排序口径。
- 不夹带任何未点名的行为变更。

## Decisions

### D1 · reviewer 投影最小内容重定义（X2 的 delta 内容）

投影**呈现内容**收窄为四类：**能力覆盖缺口（gaps-only）**、**handoff 消费闭环**、**同仓写范围冲突**、**Projection Validation Report（PVR）**。

- 缺口呈现只带 `missing_capabilities` 非空的 edge；非缺口 edge 的信息与 canonical 段重复，不在投影里二次呈现。
- handoff 消费闭环保留（`provider` / `contract_ref` / 消费者集合与消费状态），它是 reviewer 判 `unconsumed_required_handoff` 的唯一输入。
- 写范围冲突**按同仓限定**：只保留同一 `target_repository_id` 内的 exclusive/forbidden 重叠。
- PVR 保留：它是平台侧机械校验的唯一来源。

**收窄必须显式且带理由**：delta 描述段与新增场景写明 —— 跨仓 forbidden/exclusive 重叠在 multi-repo 场景无意义，实测 29 条写范围冲突中 **27 条为跨仓假阳性**。**不以「信息仍可从 CCC 推导」代替收窄**（X2 已明文指出那不足以满足「投影 SHALL 至少包括」的字面义务）。

*为什么「显式收窄措辞」必须落在 requirement 而非实现注释*：主规格 `:377` 的措辞是**无仓限定**的（「跨 work item 写范围冲突事实」）；若实现侧单方面按同仓过滤，spec 与实现即分叉，而分叉方向恰好是**放宽**（少投影内容）—— 这正是必须由 delta 显式收窄、并附实测理由的原因（29/27 是可复核的集合事实，不是偏好）。

**平台侧校验不受影响**：source/IR/机械报告的绑定校验、同代结构事实与 `canonical_contract_hash` 复算全部保留（决策 §7.1 集成约束 3：「不能因 prompt 不展示就删 source/IR/report 验证」）。delta 以独立场景承载该不变量。

*备选与否决*：
- 只改 prompt 装配不动规格 → 否决，等于静默裁，正是 X2 禁止的。
- 把「CCC 可推导」写成收窄理由 → 否决，X2 明文指出不足以满足字面义务。
- 连「节点依赖图事实」一起删干净 → 否决：环/未知 provider 是 reviewer 判断结构合法性的事实来源，删除会削弱判断力；本 delta 只裁「重复呈现」，不裁「事实来源」。

### D2 · author 上下文分级供给（G-5）

上下文按三级供给：

| 级 | 内容 | 供给方式 |
|---|---|---|
| **L1** | 成员仓摘要（成员仓源码根、目录结构事实） | **平台直供**：由平台按成员仓优先顺序生成后注入 prompt |
| **L2** | 依赖图事实（confirmed member targets / change_order 等已确认拓扑事实） | **平台直供**：由已确认来源装载后注入 prompt |
| **L3** | 其余上下文（story / design 正文等） | **维持既有预算约束**（`WORK_ITEM_PLAN_MARKDOWN_CONTEXT_BUDGET_BYTES` = 32,000 的既有分配口径不变） |

L1/L2 **不再依赖模型自行 Read/Glob 补事实**；模型拿到的 prompt 已含平台确认的事实，探索不再是获取这些事实的前提。

*为什么不把 L3 也变成直供*：L3 是**可再加载的上下文**（story/design 正文可从 durable 重新装载），直供它只是把同一份字节换个搬运路径，不减少探索；L1/L2 则是**模型无法自行判定真伪**的事实（成员仓摘要要排序才知道谁被挤掉、依赖图事实要与编译器同源），必须由平台给。

*为什么 L2 要与编译器/loader 同源*：依赖图事实若由 prompt 侧独立重述，会与 canonical 侧产生第二套口径 —— 这正是主规格既有场景「投影复用共享计算」与 REQ-WSC-06 教学口径一致性所禁止的。L2 的事实来源必须是已确认来源（member targets / change_order 单源装载），不是 prompt 内重算。

### D3 · 成员仓摘要排序修正（G-4）

`context.rs:233-234` 的排序改为**成员仓优先**：成员仓条目先入摘要，点文件/点目录条目在其后填充剩余名额；上限 `MEMBER_REALITY_MAX_TOP_DIRECTORIES`（12）**不变**。

- 目标是消除「三仓全部落榜」这一**平台侧确定性缺陷**，不是扩大摘要容量。
- **单仓路径逐字不变**：`summarize_repository_structure`（单仓分支，`:174-193`）的 `entries.sort()` + `truncate(30)` 与 `(empty repository)` 文案**一个字都不动**，REQ-PLN-17 的「单仓摘要逐字不变」场景继续成立。改动只发生在多仓分层路径（`summarize_repository_structure_logical`）。
- 分层结构（顶层 ≤12 / 每目录一级子项 ≤6 / 总行数 ≤40）与「读取失败显式报错」「空仓显式文本」语义不变（REQ-PLN-17 既有场景）。

*为什么不顺手上调上限*：上限由 `member_reality.rs` 的常量与既有 fixture 钉住；本 change 的问题不是「名额不够」而是「名额被点条目白占」。上抬上限会顶破既有 fixture 与 REQ-PLN-17 的总行数约束，属夹带。

### D4 · 返修 prompt 不复用首轮全文

返修轮 prompt **只携带**：本轮 delta（reviewer findings 回灌 + 逐条硬性修复指令 + `[revision_directives]`）与**与本轮相关的既有上下文**（上一版 source、findings 的 `required_action` 所指向的 section 及其依赖上下文）。

**MUST NOT 逐字节重发首轮完整 prompt**。相应地，返修轮**不得**再重付与本轮无关的静态规范面（grammar / few-shot / 判例等已在首轮产出过的内容）。

**返修能力不得因此退化**（delta 以两个场景承载）：
1. 本轮 findings 回灌与逐条硬性修复指令**保持注入** —— findings 不因 prompt 瘦身而消失。
2. `required_action` 所指向的 section 及其依赖上下文**在返修轮可得** —— 否则「逐字应用补丁」的指令会失去可执行对象，等于把返修变成盲改。

*为什么不是「完全不重发首轮」*：`[revision_directives]` 要求「除该行外，已通过部分逐字节保持不变」，模型需要知道上一版 source 的相关部分才能逐字保持；完全不带上一版会让返修退化为整篇重写 —— 实测 run2 的教学重驱已经证明「几乎整篇重写」会引入新缺陷（`AC-003` 被并入 `AC-002`、`AC-004` 引用丢失）。因此口径是**只带与本轮相关的既有上下文**，不是零上下文。

*备选与否决*：
- 保留逐字节复用、只加一句「请勿探索」 → 否决，探索成本在**工具往返**里，不在措辞里；且 32KB 静态规范面每轮重付与 `cache_read_tokens=128` 的事实不变。
- 只删 few-shot、保留 grammar → 否决，grammar 是「输出必须合法」的硬约束面，删掉会让返修产出不可解析。

### D5 · 基线树事实注入：事前清单 + 事后最近候选

**问题（现场证据）**：作者 prompt 里有**纪律**、没有**事实**。

- 纪律已在场：`work_item_split_engine/prompts.rs:347` 逐字写着「AC 与验证计划引用的仓库内文件路径必须存在于 plan 基线树…缺失 → `acceptance_path_not_in_baseline` Error；修复动作：{repair action}」。
- 事实不在场：首轮 prompt 32,609 字符、重驱 prompt 34,173 字符，**两者都不含**基线树路径清单（`base_tree` / `tree_paths` / `available_paths` / `existing_paths` 全无命中）；prompt 里出现的仓库路径是来自 issue/story/design 的**零散示例**，不是清单。
- 后果（实测）：作者写 `frontend/tests/integration/notice-delete-e2e.mjs` 与 `/tmp/notice-delete-e2e.mjs`，而真实案例 frontend 基线树**只有 11 个路径且完全没有 `tests/` 目录** ⇒ 必然越界 ⇒ preflight Error ⇒ 教学重驱。

**教学单靠不足**：本 change 已有的 Scenario「验收路径基线纪律」（教学义务）与「AC 基线教学生效」**都已在代码里**，实测仍 50% 命中越界（2/4 run）。因此本决策**不是**再加一段教学，而是**把事实给出去**。

**事前：作者轮前装载并注入清单（走上下文面，不动固定面）**

| 项 | 内容 |
|---|---|
| 数据源 | `workspace_engine::plan_preflight::plan_baseline_tree(lifecycle, project_id, issue_id)`（`git ls-tree -r --name-only refs/heads/<base_branch>`，**与校验器同源**，不新造第二套口径） |
| 装载时点 | **作者轮启动前**（`web/workspace_ws_handler/run/single_candidate.rs` 生产构造点）。注意：Evaluate 阶段也调同一函数（`single_candidate.rs:591`），但**Evaluate 在作者轮之后**，故作者轮前需另载一次（同一函数、同一来源，非第二套实现） |
| 注入位置 | `[confirmed_context]` 新增 `baseline:` 段（**上下文面**，非固定规范面） |
| 注入形态 | 按成员仓分组的可用路径清单；成员仓优先排序（与 D3 同序） |
| 容量 | 真实案例 43 路径 / 1,774 B；上下文面预算 32,000 B，真实占用 24,146 B ⇒ **余量 7,854 B ⇒ 装得下，零提额** |

**为什么不放固定规范面**：固定规范面质量预算 `WORK_ITEM_PLAN_MARKDOWN_PROMPT_QUALITY_BUDGET_BYTES` = 23,500 B，实测余量仅 **18~40 B**（红线：任何 SC author 教学变更必须先提额到整百级）。把 1,774 B 塞进固定面**必然要提额**，直接违反本 change 的 Non-Goal「不 bump 任何预算常量」。故清单必须落在**上下文面**。

**预算口径裁决（owner 2026-11-10 · 两个常量均不动）**：作者 prompt 的字节预算**分两面**，归属显式化：

| 面 | 常量 | 度量方式 | 本决策的清单归属 |
|---|---|---|---|
| 固定规范面 | `WORK_ITEM_PLAN_MARKDOWN_PROMPT_QUALITY_BUDGET_BYTES` = 23,500 | 以 fixture 度量（fixture 上下文极小 ⇒ 实测余量 18~40 B） | **不属此面** —— 清单不是固定教学内容 |
| 上下文面 | `WORK_ITEM_PLAN_MARKDOWN_CONTEXT_BUDGET_BYTES` = 32,000 | 以 `[confirmed_context]` 段度量（真实占用 24,146 B ⇒ 余量 7,854 B） | **属此面** —— 清单 1,774 B 装得下 |

⇒ 新增直供内容 MUST 计入**其所属面**，SHALL NOT 以「动态段」名义规避任一面；**两个常量均不上调**。相应地，既有任务 2.4 的措辞（「新增直供内容计入整 prompt 字节预算 23,500」）**已改写**为按面归属的口径 —— 改动的是**验收口径的措辞**，不是常量值、也不是「总预算不逃逸」这条约束本身（该约束仍在：两面各自有断言，均不得超）。

**事后：越界报错附最近候选路径（走既有教学重驱通道）**

- 落点：`work_item_plan_compiler/validate.rs` 生成 `acceptance_path_not_in_baseline` finding 处 —— **该处已在 `baseline_tree` 参数作用域内**，无需新增装载。
- 形态：报错 message 追加「基线树中同前缀最近的现存路径：X、Y、Z」（无同前缀候选时如实说明无可用候选，不臆造）。
- 目的：给重驱一个**具体修改对象**。实测两次重驱的工具调用均为 **0**（26.4s / 152.4s）⇒ 重驱阶段模型不查仓库、只能盲改 ⇒ 「修复动作」是纪律、没有对象。

**边界与失败语义**

- 基线树不可得（`Ok(None)`，无仓 issue）⇒ **不注入**，零回归。
- 基线树装载失败（`Err`，如基线分支不存在）⇒ **不阻断作者轮**，记诊断；既有 fail-closed 点仍在 Evaluate 阶段（`plan_baseline_tree` 的 `Err` 在 Evaluate 原样上抛）⇒ **不新增阻断点**，失败位置与现状一致。

*备选与否决*：
- 只做事前清单、不做事后候选 → 否决：重驱阶段仍无具体修改对象（实测重驱 0 工具调用），盲改命中率不变。
- 只做事后候选、不做事前清单 → 否决：事后救不防首发越界，且首轮作者多付 6~11 分钟探索代价。
- 把清单做成作者可调用的平台工具 → 否决：实测作者探索期已 9~17 次工具往返、275.4 秒，再加工具调用只会加剧往返；且失败时仍无候选路径。
- 把清单做成固定面教学 → 否决：必然触发提额，违反本 change Non-Goal（见上）。

### D6 · 前轮 finding 状态：内容身份求交 + 去重 + 分区注入

**问题（现场证据）**：前轮清单只给「身份」，不给「本轮是否仍存在」⇒ reviewer 被上一轮锚定，对**已修好**的产物重复报错 ⇒ 升级 `human_required`。

实测（`xhigh-4324`）：

1. 重驱**确实修好了**：`intent_target_codebase` 从 `lc_aggregate_root_view_7cb8c0b4-…` 改为 `7cb8c0b4-…`（授权逻辑代码库 ID）；两个 source revision 哈希不同（`aa45c70f` → `21c01e3a`）。
2. **机械侧已清零**：第二版机械报告绑定 `source-21c01e3a`，findings 只剩 1 条 warning，`intent_unexecutable` **消失**。
3. **评审却重复报 4 条同码 must_fix**：第二轮评审 prompt 引用的是**新** revision、机械报告里也没有该码，但输出 4 条 `plan_preflight.intent_unexecutable`。
4. 原因：prompt 的 `### Previous Review Round Findings` 把上一轮 finding 原样列为待办，**不含**「本轮是否仍存在」⇒ 评审只能靠自由文本判断「是否已解决」⇒ 锚定。

**求交键必须是内容身份，不能是 message 派生指纹**

`FindingFingerprint::identity_for_finding(category, class, &message, contract_field)` **含 message**。而机械报告的 message 与评审输出的 message **不同**（评审会加 `[code]` 前缀）⇒ 按完整指纹求交**恒不相交**，机制必然失效。求交键取 **`contract_field` 与机械报告 `code`**（机械报告 code 形如 `intent_unexecutable`，评审 finding 的 `contract_field` 形如 `plan_preflight.intent_unexecutable` ⇒ 平台侧按 `plan_preflight.` 前缀构造同一身份）。**fingerprint 仍保留**，但只用于「同一 identity 重现」的重复标注，不再承担求交职责。

**同一码对多 WI 重复登记 ⇒ 清单自带噪声**：实测前轮 4 条 finding 的 fingerprint **完全相同**（同一 preflight 码对 4 个 WI 各登记一次）⇒ 清单里 4 条一模一样的待办本身即「重复报」的放大器。

**做法**

1. **求交**：加载当前候选的机械报告（`session.mechanical_report_ref`，Evaluate 阶段落盘）→ 取其 findings 的内容身份 → 与前轮非 advisory findings 的内容身份求交。
2. **去重**：同一内容身份只保留一条（保留首条），重复条目折叠为计数。
3. **分区注入**：
   - `unresolved_findings`（**待办**）：本轮机械报告**仍存在**同内容身份 ⇒ 附 `required_action` 与 `contract_field`。
   - `resolved_findings`（**已关闭**）：本轮机械报告**已无**该内容身份 ⇒ 附 `resolved_by`（本轮 revision 身份）事实，**不得**再要求作者修改。
4. 评审 prompt 的重复判定纪律段改写为：重复标注以**平台给出的分区事实**为准（`resolved_findings` 不得再作为待办重复上报），模型不靠自由文本记忆判断。

**数据可达性**：当前轮机械报告 ref 在 `session.mechanical_report_ref`（Evaluate 阶段 `compare_and_save_single_candidate_evaluation` 落盘）⇒ 评审 prompt 组装时（`prompts/review_context.rs`）可直接装载。

*备选与否决*：
- 只标 `resolved_in_current_revision`、不去重不分区 → 否决：前轮清单里 4 条同 fingerprint 的重复噪声保留，评审看到的是「4 条一模一样的待办」。
- 已解决项完全不进 prompt → 否决：与本 change 既有 Scenario「注入前轮结构化清单」的字面义务冲突（该场景要求注入前轮非 advisory findings 的 canonical identity）。
- 让评审自己判断是否已解决 → 否决：**这正是现状**，且已被实测证伪为失败路径。

### D7 · 写范围越界的诊断与教学精度（包③-b）

**问题（源码直读复核）**

`/tmp/notice-delete-frontend-check/**` 被 plan 期写范围归属核对拒收。**被拒是正确的** —— 本决策**不放宽判定**，只提诊断与教学的精度。

```text
work_item_plan_compiler/write_scope_bound.rs::item_finding      :52    取 WI 被授权仓前缀，逐 exclusive scope 提取固定锚点
cross_cutting/worktree.rs::scope_fixed_path_anchors               :386   scope → 固定锚点（首个 `*` 之前的固定段）
cross_cutting/worktree.rs::normalize_scope_text                   :466   归一化（反斜杠 / `./` 前缀 / 重复斜杠 / 大小写）——**不剥离前导 `/`**
work_item_plan_compiler/write_scope_bound.rs::anchor_violation    :110   `..` 段 fail-closed(:119-122)；`path_within_prefix`(:150) 只放行被授权成员前缀内；不命中任何成员 checkout ⇒ `None` 分支(:141-144)
work_item_plan_compiler/write_scope_bound.rs::violation           :154   稳定码 `write_scope_outside_bound_repository`(:36-37)，severity = Error
```

⇒ **精确表述**：该 scope 是**语法可承载、但没有交付写权限的绝对路径**（归一化保留前导 `/` ⇒ 锚点 `/tmp/notice-delete-frontend-check` 不命中 `busi` / `frontend` 等成员前缀）；它在操作系统意义上**也不是聚合根相对路径**。当前报错把它归入「未命中成员」大类，**没有真正定位到某个聚合根内部目录**。

**要修的具体点（误导性「改写例」）**：`anchor_violation` 的 `None` 分支（`:141-144`）对所有未命中锚点统一输出改写示例 `如 {bound_prefix}/{anchor}`。对仓内相对写法（`src/lib.rs` → `busi/src/lib.rs`）该示例正确；对绝对路径（`/tmp/...`）它产生 `busi//tmp/notice-delete-frontend-check` —— 一个**语法上不可能成立**的路径，把作者引向错误修复方向。

**纪律教学已经存在且不弱**：`work_item_split_engine/prompts.rs:282-285` 调用 `prompts/sc_author_teaching.rs::work_item_plan_confirmed_member_order_teaching`（`:238`），其「写范围路径基准」段在 `:281`（实测 **563 B** UTF-8，含两个尾随换行），已明示「`<成员目录>/<仓内路径>`」「裸仓内相对写法（如 `src/**`）仅被执行期兼容、plan 期归属校验会按越界拒收」「聚合根本地文件区……任何 Work Item 都不可写」。⇒ 要修的是**反例精度与诊断可执行性**，不是再堆一段纪律。

**选定方案：不改权限判定，只提精度**

1. **替换现有多仓教学段，不追加**：把 `:281` 的「写范围路径基准」段**整段替换**为候选文本 —— **保留**既有三条纪律的语义要点（成员目录前缀的聚合根相对形式 / 裸仓内相对写法 plan 期拒收 / 聚合根本地文件区不可写），并**增加**：① `/tmp/check/**` 一类**绝对路径反例**；② 明示「**不能直接加成员目录前缀冒充仓内路径**」；③ exclusive_scopes **必须属于本 Work Item 的授权成员**；④ 明示「被授权成员内新建路径」与「AC 基线存在性」是**两项独立规则**（前者由写范围归属核对管，后者由 `acceptance_path_not_in_baseline` 管）。
2. **`anchor_violation` 对明显绝对路径给专门说明**：锚点以 `/` 开头 ⇒ 走专门分支：**仍返回同码 Error**，消息保留原始路径并明确「绝对路径 / 不属于授权成员」；**不再生成 `{bound_prefix}/{anchor}` 改写例**；**不自动删除 scope、不去掉前导 `/`、不迁移文件**。仓内相对写法的改写指导**保留不变**（既有断言 `repo_relative_scope_is_blocked_with_rewrite_guidance_not_misclassified` 依赖它）。

*被否决*：

| 备选 | 否决理由 |
|---|---|
| 给 `/tmp/**` 开例外 | 扩权：把「无交付写权限的绝对路径」变成合法写面，违反 fail-closed 与写范围归属核对的既有语义；且不修根因（作者仍不知道仓内路径该怎么写） |
| 由平台自动把 `/tmp/...` 搬到成员目录 | 改变用户交付语义：平台替作者决定交付位置；正确做法是让诊断可执行（给出正确写法），而不是代改 |

**预算**

- 当前「写范围路径基准」段实测 **563 B**（UTF-8，含两个尾随换行）；主方案给出的候选替换段 **516 B** ⇒ **净减 47 B**。
- ⚠️ **数字纪律**：563 B 是**实测基线对照值**；516 B / 净减 47 B 是主方案的**候选对照值**（候选文本未逐字给出）⇒ **实施后必须实测复测**，不得写成已达成结论。
- 该段在 `member_targets=None`（legacy 单仓）时**整段缺席**（`sc_author_teaching.rs:242-247`）⇒ 可保持 legacy 固定 fixture（对照基线：实测 **23,448 / 余 52**；预算常量 `WORK_ITEM_PLAN_MARKDOWN_PROMPT_QUALITY_BUDGET_BYTES` = 23,500，`work_item_split_engine/prompts.rs:143`）**逐字不变**。
- **不得新增「动态教学不计预算」的例外**：替换段仍在固定规范面内计量。

**验收落点**

- `src/product/work_item_plan_compiler/tests/write_scope_bound.rs`（实测 **424 行**）：加入实际 `/tmp/notice-delete-frontend-check/**` 样本 ⇒ 恰一条指向目标 WI 的同码 Error；消息保留原路径、明确「绝对路径 / 不属于授权成员」；**不出现 `busi//tmp` 等自动前缀建议**；`busi/src/new_feature/**` 仍放行（即使不存在）；`src/lib.rs` 仍给「仓内相对写法需改为成员前缀」的指导；单仓 / Manual、映射缺失、`..` 与跨成员现有断言保留。
- 多仓夹具断言放 `src/product/work_item_split_engine/tests/prompt_contract_sc_revision.rs`（实测 **703 行**）；**不得继续向已 1,630 行的 `tests/prompt_contract.rs` 堆测试**。

### D8 · 成员清单 / 逐仓实况注入（包⑤）

**先澄清与本 change 既有范围的关系（不重复立项）**

| 既有项 | 本 change 既有内容 | 本决策的关系 |
|---|---|---|
| **G-4**（成员仓摘要排序修正，`work_item_split_engine/context.rs:233-234` 的 `top_level.sort()` + `truncate(MEMBER_REALITY_MAX_TOP_DIRECTORIES = 12)`） | 已规划，且已排为迁移计划**第 2 步**，带「单仓逐字不变」义务（`REQ-PLN-17`） | **不重复立项、不取代它**。G-4 修「摘要里的**名额分配**」 |
| **G-5 / D2 L1 直供**（成员仓摘要 + 依赖图事实平台直供） | 已规划（`tasks.md` §2） | 本决策是 **L1 的供给源接线**，落在同一条 L1 直供工作包内 |
| **D5 事前**（基线树清单注入 `[confirmed_context]`） | 已落地（上下文面，额度 32,000/4） | 另一段上下文面内容，**与本段分别计量**（见下） |

**新增的缺口**：G-4 只解决「摘要里有没有成员仓名」（实测 `structure:` 段 **276 B、成员名 0 次** —— `.agents` / `.claude` / `.codegraph` / `.codex` / `.gitignore` 等**点条目**按名排序后占满 12 个名额）。**逐仓的「事实」（职责 / 关键目录 / 入口 / 构建测试命令 / 基线分支）在 SC author prompt 侧根本不存在**：

```text
render_compact_inventory | MEMBER_REALITY_SECTION_MARKER | 逐仓实况
  src/product/work_item_split_engine/                ⇒ 零命中
  src/product/workspace_engine/prompts.rs             ⇒ 命中的是**聚合材料块渲染器**
        aggregate_scope_material_prompt(:779) / 成员清单小节 + 逐仓实况小节(:789-795)
        ⇒ 服务 workspace 会话（Story / Design / WorkItem outline）路径，不是 SC markdown author 路径
```

**产品内已有的渲染器与消费链（源码直读）**

```text
logical_codebase/planning_context_set.rs::render_compact_inventory   :195  成员清单（id|alias|path|role|profile）+ 逐仓实况段
logical_codebase/planning_context_set.rs::render_member_reality_section :317 段首 `## 逐仓实况`（member_reality.rs:21），每仓 5 行（member_reality.rs:99 渲染）
logical_codebase/planning_context_resolver.rs::render_compact_inventory 调用点 :473 ⇒ PlanningContextSnapshot.inventory_injection(:52)
web/workspace_context/builder.rs:201 ⇒ aggregate_scope_material_prompt(&resolved.inventory_injection.rendered)
web/workspace_ws_handler/run/provider_run/legacy_outline.rs:242-247 ⇒ aggregate_work_item_target_scope_prompt(&rebuilt.inventory_injection.rendered, …)
```

⇒ **legacy outline 路径有，SC markdown author 路径没有**。

**两者互补，不是替代**：G-4 修「摘要的名额分配」，本决策供「摘要里根本没有的逐仓事实」。

**实施约束（源码直读核验）**

| 约束 | 事实 | 处置 |
|---|---|---|
| **全量不裁剪** | `render_compact_inventory` 渲染**全量成员**（`planning_context_set.rs:182-183` 注释、`:238-250` 清单渲染），MUST NOT 按 soft/hard 预算裁剪成员，MUST NOT 输出 `omitted_member_count`（`REQ-PLN-12`） | 字节随**成员数**线性增长 ⇒ **不受** D5 基线清单的 32,000/4 上限保护，**两段分别计量** |
| **会返回 `Err`** | `rendered.len() > PLANNING_MATERIAL_MAX_BYTES`（`planning_context_set.rs:168` = **500 KiB**）⇒ `Err(ProductStoreError::InvalidRecord { kind: "planning_material_over_budget" })`（`:227-235`） | 接入侧**必须**处理失败路径（降级为「逐仓实况缺席」），不得 panic、不得整轮失败 |
| **渲染顺序** | 目标成员在前（保持 `target_member_ids` 顺序），其余按 alias 序（`planning_context_set.rs:201-212`，注释 `:181`） | 与基线树 / 成员顺序口径**保持一致**；目标成员取本计划已授权成员（与 `member_targets` 同源） |
| **缺 checkout 的成员** | ⚠️ **源码核验更正（与主方案表述有出入，以源码为准）**：该成员**仍然出现**在清单与逐仓实况段 —— `resolution.set` 含全部有效成员（`planning_context_set.rs:132-151`），`render_member_reality_section` 遍历**全部**成员（`:324-330`）。其「关键目录 / 入口 / 构建测试命令」显「未识别」、基线显「基线未识别(该成员无 checkout 记录)」（`member_reality.rs:80-83`）；且其清单行 `path` 列退化为**成员 alias**（`planning_context_set.rs:143-144` 的 `unwrap_or_else(\|\| member.alias.clone())`）—— **不是**真实聚合根相对路径 | 接入侧 MUST NOT 把该行 `path` 当作真实路径解释；MUST NOT 静默丢弃该成员；「逐仓实况成员集合」与「已授权有效成员」的口径差 MUST **如实解释**（不得静默） |
| **空集合** | `render_member_reality_section` 在 `ordered.is_empty()` 时返回空串（`planning_context_set.rs:321-323`） | 无仓 issue 不注入（与 D5 既有口径一致） |

**注入形态（接入决策）**

- **事实来源唯一**：复用 `PlanningContextSetResolver::resolve`（`planning_context_set.rs:52`）+ `render_compact_inventory`（`:195`）—— **不新造第二套成员发现器**、不改渲染器语义（REQ-PLN-12 / REQ-PLN-14 归 `logical-codebase-aggregate-planning` 域）。
- **装配点**：作者轮生产构造点（`web/workspace_ws_handler/run/single_candidate.rs`，与 D5 事前的 `single_candidate_baseline_tree` 同处、同模式），作者轮启动前装载。
- **上下文承载**：`WorkItemPlanMarkdownAuthorContext` 新增**可选**字段（成员清单 + 逐仓实况的渲染文本），注入 `[confirmed_context]` 的新增段 —— **上下文面**，不进固定规范面；`None`（无仓 / 装载失败 / 预算不足）⇒ 该段**整段缺席**，legacy 单仓 fixture 逐字不变（与 D5「`None` ⇒ 整段缺席、不留空标记」同一口径）。
- **预算口径**：上下文面 32,000 B **不抬**；两段**分别计量** —— 先算本段字节，再从上下文面剩余额度中扣减；额度不足 ⇒ **整段缺席 + 装载侧记诊断**（MUST NOT 裁剪成员以「塞得下」，MUST NOT 留半截成员块；MUST NOT 输出 `omitted_member_count` 字样）。
- **成员块粒度**：要么整段在场，要么整段缺席；不得出现「部分成员在场」而未说明的形态。

**边界**

概览给的是「有哪些仓、关键目录在哪」，**不含文件清单** ⇒ **不能替代多成员基线树**（跨仓误拒不因此消除）。多成员基线树属**另一 change `plan-baseline-multi-member`**（REQ-WSC-02 域，序 7 之后）。**两件都要，分属两个 change。**

**成本**

- 字节随成员数增长（主方案给出 3 成员量级 ~1–2 KB 的**自造估算**）。⚠️ **实施时必须实测**并计入上下文面；**不得引用自造估算值**。
- **成本守恒**：固定规范面 23,500 B **零新增**（本段落在上下文面）；上下文面 32,000 B **不抬**；D7 的教学段**净减 47 B**（对照值）；**无新增模型调用**；**无新增 env 开关**。

## 共享面与排序

按全局串行表（处置裁决 §2.2，唯一权威），本 change 排在**序 7（最后）**。**这些 change 目录本 change 一律不改**，仅在此登记共写文件与相对次序。

| 共享文件 | 对方 change | 相对次序 | 本 change 的动作 |
|---|---|---|---|
| `src/product/work_item_split_engine/prompts.rs` | `isolate-plan-criterion-namespace`（序 5：`:185,:306` 的 WIC ID 纪律加入） | **isolate → 本 change** | 本 change 改 `:395` `budget_markdown_context` 的签名与调用点（L1/L2 直供）；**不改** isolate 的 `:185` / `:306` 两处 ID 纪律措辞，也不改其语义 |
| `src/product/work_item_split_engine/prompts/sc_author_teaching.rs` | `mechanicalize-plan-compile-repair`（序 1：波次 7「prompt 分层瘦身」≤8K 字符 / ≤25 硬措辞） | **mechanicalize → isolate → 本 change** | 本 change 在其瘦身之后改写作者教学面（若 L1/L2 直供需要新增教学措辞）；顺序相反则瘦身的度量基线被本 change 改动，验收失真（双边登记：mechanicalize 侧同样登记本 change 后续接手） |
| `src/product/work_item_split_engine/tests/prompt_contract*.rs`（`prompt_contract.rs`、`prompt_contract_weak_model.rs`、`prompt_contract_sc_revision.rs`） | `mechanicalize-plan-compile-repair`（按新契约更新既有断言，不许删测试） | **mechanicalize → isolate → 本 change** | 本 change 更新作者 prompt 与返修 prompt 的契约断言（直供段存在性、返修轮不带首轮全文）；两个 change 的断言合并在同一提交内落地，避免同一断言被两处改写 |
| `src/product/workspace_engine/prompts/review.rs` | `close-contract-findings-before-pass`（序 6：单候选 reviewer prompt 装配与呈现口径） | **close → 本 change** | 本 change 改单候选 reviewer 的投影**呈现段**（`single_candidate_input.inc.rs`）；**不改** close 的有效必关闭判定与呈现口径，二者是不同物 |
| `src/product/work_item_split_engine/prompts/sc_revision.rs` | `close-contract-findings-before-pass`（序 6：`:147` 的返修轮呈现口径 —— 重开后仍为 `suggestion` 的契约缺口按有效必关闭呈现） | **close → 本 change** | 本 change 改 `:27` 的**prompt 构造口径**（停止逐字节复用首轮全文）；**不改** close 在 `:147` 的 severity/呈现口径 |
| `src/product/work_item_plan_compiler/validate.rs`、`types.rs` | `mechanicalize-plan-compile-repair`（序 1：REQ-WSC-09 唯一持有者，`types.rs:61-80` 加 `lowering_fault` 字段；`types.rs:121` 的 `ACCEPTANCE_PATH_NOT_IN_BASELINE_REPAIR_ACTION` 常量在其改动面内） | **mechanicalize → 本 change** | 本 change（D5 事后）只在 `acceptance_path_not_in_baseline` 的 finding message 上**追加**「基线树中同前缀最近路径」；**不改**四类 typed IR 诊断的资格清单、不改 `REPAIR_ACTION` 常量逐字文本、不改机械化判定口径 |
| `src/product/workspace_engine/prompts/review_context.rs` | `close-contract-findings-before-pass`（序 6：`:557`「评审上下文构建入口的作用域」已在其改动面内） | **close → 本 change** | 本 change（D6）只在前轮 findings 清单内**增补**内容身份求交状态、去重与分区；**不改** close 的有效必关闭判定、severity 呈现与 owner 排除口径 |
| `src/web/workspace_ws_handler/run/single_candidate.rs` | `mechanicalize`（序 1）/ `bound`（序 4）/ `isolate`（序 5）/ `effort` / `fix-usage-and-timing-metrics` 多方共写 | **本 change 序 7 在其后** | 本 change（D5 事前）只在该生产构造点**装载基线树并向 prompt context 传参**；**不改**教学资格判据、effort 出口、usage/timing 埋点与任何 provider 调度 |
| `src/product/work_item_plan_compiler/write_scope_bound.rs` | 无（写范围归属核对面为其独有；`bound-sc-generation-teaching` 不改该文件） | 本 change（序 7）自有面 | 本 change（D7）只改**绝对路径锚点的诊断文案分支**（同码 Error、判定不放宽）；**不改** `item_finding` 的归属判定、`path_within_prefix` 前缀口径、`..` 段 fail-closed 与稳定码 |
| `src/product/work_item_split_engine/prompts/sc_author_teaching.rs` | `mechanicalize-plan-compile-repair`（序 1：波次 7「prompt 分层瘦身」≤8K 字符 / ≤25 硬措辞） | **mechanicalize → 本 change** | 本 change（D7）**替换**「写范围路径基准」段（`:281`）：保留既有纪律语义要点 + 增补反例；**不改**该文件其余教学段与其瘦身口径 |
| `src/product/work_item_split_engine/tests/prompt_contract_sc_revision.rs` | `mechanicalize-plan-compile-repair`（按新契约更新既有断言，不许删测试） | **mechanicalize → 本 change** | 本 change（D7 多仓夹具断言 + D8 逐仓实况断言）追加于该文件；**不删改**既有断言 |
| `src/product/logical_codebase/planning_context_set.rs` / `member_reality.rs` | `plan-baseline-multi-member`（序 7 之后）、`logical-codebase-aggregate-planning` 主规格域 | 本 change 在前 | 本 change（D8）**只读消费** `render_compact_inventory`（`:195`）与 `render_member_reality_section`（`:317`）；**不改**渲染器语义、不改 `PLANNING_MATERIAL_MAX_BYTES`、不违反 REQ-PLN-12 / REQ-PLN-14 |
| `src/web/workspace_ws_handler/run/single_candidate.rs`（D8 成员实况装载点） | 同上（D5 事前已登记） | **本 change 序 7 → `plan-baseline-multi-member` 序 7 之后** | 本 change（D8）只在该生产构造点**装载成员清单 + 逐仓实况并向 prompt context 传参**；不改 provider 调度、教学资格、effort / usage / timing 埋点 |

**排序结论**：决策六纯拆分 → `mechanicalize`（prompt 瘦身与其 fixture/断言）→ `isolate`（WIC ID 纪律）→ `bound` → `effort` → `close`（返修轮呈现口径）→ **本 change**（序 7，最后）。本 change 落在最后，是因为它的三处写入面（`prompts.rs:395`、`sc_revision.rs:27`、reviewer 投影呈现）都已被前序 change 改过 —— 在前序语义未稳定时并发改同一文件，会让「哪条语义变化导致了验收结果变化」无法归因。

**D5 / D6 不改变全局次序**：二者是本 change 内部的增补决策，其新增写入面（`work_item_plan_compiler/validate.rs`、`prompts/review_context.rs`、`web/.../run/single_candidate.rs`）分别已被 `mechanicalize`（序 1）与 `close`（序 6）改过，而本 change 恒在序 7 ⇒ **D5 事后排在 mechanicalize 之后、D6 排在 close 之后**这一相对次序**由既有全局序天然满足**，无需新立 change、无需改串行表。D5 事前只**读** `plan_baseline_tree`（不改其判定语义），与 `single_candidate.rs:591` 的 Evaluate 期调用**同函数同来源**，不构成第二套基线口径。

**D7 / D8 不改变全局次序**：二者是本 change 内部的增补决策，其新增写入面（`work_item_plan_compiler/write_scope_bound.rs`、`prompts/sc_author_teaching.rs`、`tests/prompt_contract_sc_revision.rs`）与 D5 事前共用的作者轮生产构造点（`web/.../run/single_candidate.rs`）均已被前序 change 改过，而本 change 恒在序 7 ⇒ **D7 排在 mechanicalize 之后、D8 与 D5 事前同处（同一构造点、串行落地）**这一相对次序**由既有全局序天然满足**，无需新立 change、无需改串行表。D8 只**读** `logical_codebase` 的 `render_compact_inventory` / `render_member_reality_section`（不改其判定语义、不改 `PLANNING_MATERIAL_MAX_BYTES`），与 `plan-baseline-multi-member`（序 7 之后）**无共写**⇒ 不构成次序冲突。

**无双 MODIFIED 声明**：本 change **只 MODIFIED REQ-WSC-06 一条 requirement**。与其它 active change **无同一 requirement 被双 MODIFIED**：

| requirement | 本 change | 其它 active change | 结论 |
|---|---|---|---|
| `work-item-plan-single-candidate` / `REQ-WSC-06` | **MODIFIED**（唯一 owner） | 无（`isolate-plan-criterion-namespace` 全目录零命中 REQ-WSC-06；`mechanicalize` / `close` / `bound` / `effort` 亦零命中） | 独占，无双改 |
| `work-item-plan-single-candidate` / `REQ-WSC-09` | 不改 | `mechanicalize-plan-compile-repair` MODIFIED | 无交集 |
| `work-item-plan-single-candidate` / `REQ-WSC-17` | 不改 | `close-contract-findings-before-pass` MODIFIED | 无交集 |
| `work-item-plan-single-candidate` / `REQ-WSC-21` | 不改 | `fix-author-rewrite-quota-pool` MODIFIED | 无交集 |
| `work-item-plan-single-candidate` / `REQ-WSC-25` / `REQ-WSC-28` / `REQ-WSC-29` / `REQ-WSC-30` / `REQ-WSC-31` | 不改 | `bound` / `effort` / `isolate` | 无交集 |
| `workspace-structural-governance` / `REQ-SG-05`、`work-item-typed-outcome-policy` / `REQ-TOP-04` / `REQ-TOP-05` / `REQ-TOP-07` | 不改 | `bound` / `isolate` / `close` | 无交集 |
| `logical-codebase-aggregate-planning` / `REQ-PLN-17` | **不改**（只修多仓路径排序口径，单仓逐字不变义务继续成立） | 无 | 无交集 |

**REQ-WSC-06 的独占声明**：本 change 是 **REQ-WSC-06 的唯一 owner**（此前无 owner）。X2 的投影裁剪、G-4 的成员仓摘要排序、G-5 的上下文分级供给、返修 prompt 的复用去除，**全部落在这一条 requirement 之下** —— 它们同属一件事：**「prompt 承载什么、由谁供给」**。拆成多条 requirement 会让「投影最小内容」「上下文供给分级」「返修轮内容」三条义务各自独立演进，正是主规格 §8.0 待办登记所指的失序形态。

## 不承载什么

| 项 | 归属 | 本 change 的动作 |
|---|---|---|
| **G-2** MCP 输出截断（`MAX_MCP_OUTPUT_TOKENS`） | SC author 出口 env 层 | **只登记，不夹带** |
| **G-3** Bash 输出截断（`BASH_MAX_OUTPUT_LENGTH`） | SC author 出口 env 层 | **只登记，不夹带** |
| **G-6** 缓存 TTL 1h（`CLAUDE_CODE_PROMPT_CACHE_TTL`） | SC author 出口 env 层 | **只登记，不夹带** |
| G-1 effort 档位（默认 `high`） | `explicit-sc-author-effort` | 不涉及 |
| **方案 A（引用域统一 / cross-WI）** | `isolate-plan-criterion-namespace`（REQ-WSC-06 之外的命名域隔离 owner，其 delta 已有「`done_when_refs` 引用另一 WI 的 criterion ⇒ 拒绝」场景） | **不承载**；只登记相邻关系与本次实测事实（见下） |
| 探索预算硬约束（`[exploration_budget]` 段，方案 §6.7） | 主方案标为「次级项」，非本 change 契约 | 不引入 |
| **多成员基线树判定修复（跨仓误拒）** | `plan-baseline-multi-member`（REQ-WSC-02 域，序 7 之后） | **不承载**；D8 的逐仓实况只给「有哪些仓 / 关键目录在哪」，**不含文件清单** ⇒ 不能替代多成员基线树，跨仓误拒不因此消除（两件都要，分属两个 change） |
| **`/tmp/**` 一类绝对路径的写权限例外** | 无承载方（本 change 明确否决） | **不承载**；写范围判定 fail-closed 不变（D7 只提诊断与教学精度，不放宽判定） |

**相邻关系登记（方案 A / cross-WI 实测校准）**：本次产品全流程实测（4 run，同一案例、同一份 32,609 字符 author prompt）产出 cross-WI 越界 **0/79**，而历史 run1 同一份 prompt 产出 **15 条**（`unknown_done_when_ref` 15/15 全为 cross-WI）⇒ 该失败模式在同输入下**偶发**，方案 A 的「必然触发」因果链**未被支持**。方案 A 的代码侧（结构硬检把偶发变不可能）**仍值得做**，但其优先级**不应高于**本 change 的 D5 / D6。本 change **不 MODIFIED** 方案 A 的任何 requirement，只在此登记该事实；因果链与优先级的校准落在**方案文档**（`cadence/designs/2026-11-08_技术方案_WIG生成加速与收敛性加固_v1.0.md`）与实测分析报告（`cadence/analysis-docs/2026-11-09_分析报告_WIG产品流程实测与根因锁定_v1.0.md` §5.0）。

**相邻关系登记（env 层）**：G-2 / G-3 / G-6 的注入点是 **SC author 出口的 env 构造**（`explicit-sc-author-effort` 负责该出口的 env 装配机制，注入点为 `workspace_engine/prompts/sc_author_input.rs` 与 `sc_launch_policy.rs`）。本 change 改的是 **prompt 内容面**（`work_item_split_engine/prompts.rs` 的上下文供给、`sc_revision.rs` 的返修构造、reviewer 投影呈现），与 env 层**不同物**。两者唯一的耦合点是「本 change 让 prompt 变小 ⇒ 同样的探索预算下更少触发压缩」，这属**收益方向**，不构成实现耦合，也不得被写成已测结论。

**必须同时声明的红线**：G-6 的 TTL 1h 收益**未实测**（决策 §3.2、方案 §6.5），不得写成确定事实；G-2 / G-3 是否纳入本期由裁决 §3.1(1) 的范围限定裁定（本期只控制 Claude SC author 生成/教学链）。本 change **不引用**这三项的任何收益数字作为验收依据。

## Risks / Trade-offs

| 风险 | 缓解 |
|---|---|
| **投影裁剪削弱 reviewer 判断力**（信息变少 ⇒ 漏检） | 裁剪只删**与 canonical 段重复**的呈现，不删事实来源：缺口（`missing_capabilities` 非空的 edge）、handoff 消费闭环、同仓真实冲突（实测 2 条是真实信号）、PVR 全部保留；且 gap edge 的 `evidence` 要求仍在 prompt 中逐条列出 |
| **同仓限定误删真实跨仓约束** | 收窄只针对**写范围（exclusive/forbidden）**；跨仓依赖、契约供给/需求、环与未知 provider 等事实不在收窄范围内。跨仓写范围在 multi-repo 场景本就无意义（不同成员仓写不同物理目录），29/27 的实测比例登记在 spec 里可复核 |
| **L1/L2 直供把 prompt 顶破 23,500 B 质量预算** | 直供内容来自**已存在的**平台事实（成员仓摘要、member targets / change_order），不是新增教学内容；决策 §7.1 集成约束 4 明确「不得利用动态段名义逃总预算」。验收项含「作者 prompt 字节仍在既有质量预算内」 |
| **返修轮瘦身后模型无法逐字保持未涉及 section** | delta 的第二个新增场景把「`required_action` 所指向的 section 及其依赖上下文在返修轮可得」写成**义务**，并在 `tasks.md` 列为独立验收项；`[revision_directives]` 的「逐字节保持」要求不被削弱 |
| **返修轮不再重发首轮全文 ⇒ 模型忘记输出语法** | 返修轮仍须输出**完整 markdown source**，因此**输出语法面不可省**。口径是「只带与本轮相关的既有上下文」而非「零上下文」；实现上以「与本轮相关」为准绳保留必要格式约束，验收项须证明返修轮产出仍可被 parser 解析 |
| **G-4 排序修正动到单仓路径** | 改动只在多仓分层分支；单仓分支 `entries.sort()` + `truncate(30)` 逐字不动，REQ-PLN-17 的「单仓摘要逐字不变」场景作为既有门禁持续生效 |
| **与 close 的返修轮呈现口径互相覆盖** | 两者不同物：close 改 `:147` 的**severity 呈现**（哪些 finding 必须修），本 change 改 `:27` 的**prompt 构造**（带哪些内容）。串行次序 close → 本 change，且本 change 不改 `:147` 的语义 |
| **D5 事前清单把上下文面顶破 32,000 B** | 清单**计入上下文面同一配额**（owner 2026-11-10 裁决）：先算 `story:`/`design:`/`structure:`/`baseline:`/`story_spec_ids:`/`design_spec_ids:` 的**固定开销**，再把**剩余**额度给清单（上限 32,000/4）；额度为 0 ⇒ 整段缺席（不留空标记）。三段占满时**带清单 ≡ 不带清单**（逐字节同长），由 `markdown_author_prompt_context_face_stays_within_budget_when_all_faces_are_full` 钉住 |
| **上下文面 32,000 的既有溢出（**非本 change 引入**，登记不修）** | `budget_markdown_context` 的三段配额之和恰为 32,000（14,400 + 14,400 + 3,200），但 `story:`/`design:`/`structure:`/`story_spec_ids:` 等**固定开销未被计入** ⇒ 三段占满时上下文面**既有地**超出 32,000 数十字节（实测 32,060 B）。该溢出**先于**本 change 存在，本 change **不擅自修正**（不扩范围），但 MUST NOT 让清单**新增**溢出 —— 断言即「带清单 ≡ 不带清单」。**登记待后续独立 change 处理**（把固定开销从三段配额中预留） |
| **D5 事后追加的候选路径可能是错的** | 候选路径来自**同一棵基线树**（校验器同源），按同前缀最近匹配给出、并列时全部列出、无匹配时如实说明无可用候选；**不得**由平台臆造路径（臆造即制造新的越界引用） |
| **D6 求交键选错 ⇒ 恒不相交** | 指纹含 `message`，机械报告 message 与评审 message 不同 ⇒ **必须**按 `contract_field`/code 求交；delta 以独立场景把「求交键为内容身份而非 message 派生指纹」写成义务，防止实现踩坑 |
| **D6 误把已解决项判为待办 ⇒ 仍会重复报** | 求交的数据源是**当前候选**的机械报告（`session.mechanical_report_ref`，Evaluate 阶段落盘）⇒ 与评审输入同代；同代校验既有 `ir_adapter.rs` 的 `mechanical_report_ref` 一致性检查兜底 |
| **【新发现 · 既有产品缺陷】多仓场景下 plan 基线树只覆盖 issue 的**一个**仓** | `plan_baseline_tree` → `plan_baseline_repository_path` 按 issue 的 `repo_id` 解析**单一**物理仓（`git ls-tree` 该仓基线分支），返回**成员内相对路径**（实测案例：busi 的 `.gitignore`/`pom.xml`/`src/main/java/…`，**不含** frontend / gateway）。**校验器用的就是这同一棵树** ⇒ 多仓计划里 **frontend/gateway 的 AC 路径必然被判「不在基线树内」** —— 这正是基线 run3（`acceptance_path_not_in_baseline` 命中 WI-004=frontend）的直接原因。**该缺陷先于本 change 存在**，D5 事前注入的是**同一棵树**，故**只能让作者看见事实、不能修复多仓判定**。⇒ 本 change **登记**该缺陷并把「基线树覆盖全部成员仓 + 按成员仓分组比较」列为**后续独立 change**（属 REQ-WSC-02 域，需 owner 裁决其验收语义）；本 change 内的渲染**如实按树自身的顶层段分组**（不再产出误导性的 `(未归属)` 归并） |
| **本 change 的实现曾把 4 个文件推过「单文件行数」红线** | 仓库有 `tests/it_core/large_file_guard.rs` 的 `product_source_and_test_files_stay_under_line_limit`（默认 ≤1200 行，个别文件在守卫登记表里有更高的**登记上限**）。D5/D6 实现落地后该守卫报 4 个文件超限（`prompts.rs` 1378 / `prompt_contract.rs` 2021 / `review_prompt.rs` 1283 / `run/single_candidate.rs` 1693）。⚠️ **该守卫在集成测试里（`--test it_core`），只跑 `--lib` 看不到** ⇒ 验证必须跑**全量** `cargo test --locked`。处置：**拆分**而非抬登记上限（新增 `prompts/sc_baseline_tree.rs`、`prompts/sc_few_shot.rs`、`run/single_candidate_baseline_tree.rs`、`run/single_candidate_delivery.rs` 与两个测试文件），拆后四个文件**均低于**其原登记上限 |
| **D5/D6 的收益未实测** | 现场证据是**现状基线**（n=2/档，preflight 失败 2/4、需人工介入 2/4），不是方案效果。所有收益数字一律标注为**待复测**，不得写成达成承诺（与「Open Questions」第 4 条同口径） |
| **D7 的诊断分支被误读成「放宽了判定」** | D7 只改**文案分支**：绝对路径仍返回同码 `write_scope_outside_bound_repository`（Error），判定口径、`path_within_prefix` 前缀语义、`..` 段 fail-closed 与稳定码逐字不变；验收断言「绝对路径样本仍被拒且仍为该码」直接钉住这一点 |
| **D7 删掉了「仓内相对写法」的改写指导 ⇒ 既有断言红** | 绝对路径分支与仓内相对分支**分开**：只有绝对路径不再生成 `{bound_prefix}/{anchor}` 改写例；仓内相对写法（`src/lib.rs`）的改写指导保留（既有断言 `repo_relative_scope_is_blocked_with_rewrite_guidance_not_misclassified` 依赖它） |
| **D7 的教学段替换把固定面顶破 23,500** | 替换段是**净减**（对照：563 B → 候选 516 B）；该段在 `member_targets=None` 时整段缺席 ⇒ legacy 固定 fixture（对照：23,448 / 余 52）逐字不变。⚠️ 候选文本未逐字给出 ⇒ 字节数**实施后必须实测复测**；且**不得**新增「动态教学不计预算」的例外 |
| **D8 的逐仓实况把上下文面顶破 32,000** | 本段与 D5 基线清单段**分别计量**（本段不占用 D5 的 32,000/4 额度，但两者之和 MUST ≤ 32,000）；额度不足 ⇒ **整段缺席 + 装载侧记诊断**（不裁剪成员、不留半截成员块、不输出 `omitted_member_count` 字样）。固定规范面 23,500 B **零新增** |
| **D8 的逐仓实况被误当作基线树替代** | 逐仓实况只给「有哪些仓 / 关键目录在哪」，**不含文件清单**；跨仓误拒不因此消除 —— 基线树修复属 `plan-baseline-multi-member`（REQ-WSC-02 域），本 change 只登记边界 |
| **D8 的「缺 checkout 成员」口径被写错** | 以源码为准：该成员**仍在**清单与逐仓实况段（关键目录 / 入口 / 构建测试命令显「未识别」、基线显「基线未识别(该成员无 checkout 记录)」），其清单行 `path` 列退化为**成员 alias**（非真实聚合根相对路径）⇒ 接入侧 MUST NOT 把该行当真实路径解释、MUST NOT 静默丢成员 |
| **D8 的字节成本靠自造估算** | 主方案的「3 成员量级 ~1–2 KB」是**自造估算值**；实施时**必须实测**成员数与字节并计入上下文面，**不得**引用自造估算（与「Open Questions」第 4 条同口径） |

## Migration Plan

1. **锚定**：在**未改实现**的状态下记录基线 —— 作者 prompt 实测字节与 margin、返修轮 prompt 实测字节、`structure` 段实测长度与成员仓出现次数、reviewer 投影各段实测字节。基线数字只作**对照**，不作为「已达成」结论。
2. **G-4 先落**（纯纠错、零契约变更面）：多仓摘要排序改为成员仓优先；单仓路径断言逐字不变。
3. **X2 投影裁剪**：先按 delta 改 reviewer 侧**呈现段**（`single_candidate_input.inc.rs`），保留平台侧绑定与同代校验；对照基线记录各段字节与判断必需内容是否齐备。
4. **G-5 直供**：改 `budget_markdown_context` 签名与调用点，把 L1（成员仓摘要）与 L2（依赖图事实）作为平台直供段注入；L3 分配口径不变。
5. **返修 prompt 瘦身**：改 `sc_revision.rs:27` 的构造口径（带本轮 delta + 本轮相关既有上下文，不再逐字节重发首轮全文）；同步更新 `prompt_contract_sc_revision.rs` 断言。
6. **教学与断言合并提交**：`sc_author_teaching.rs` 与 `prompt_contract*.rs` 的改动与前序 change 的合并在同一提交内落地（见「共享面与排序」）。
7. **D5 事后（基线树最近候选回灌）**：`work_item_plan_compiler/validate.rs` 的 `acceptance_path_not_in_baseline` finding message 追加最近候选路径；`types.rs:121` 的 `REPAIR_ACTION` 常量**逐字不动**；断言覆盖「有同前缀候选 / 无候选」两分支。
8. **D5 事前（基线树清单直供）**：作者轮生产构造点装载基线树 → `WorkItemPlanMarkdownAuthorContext` 新增字段 → `[confirmed_context]` 新增 `baseline:` 段；**上下文面**字节仍在 32,000 B 内、**固定规范面**字节仍在 23,500 B 内（后者不应因本项变动）；无仓 issue（`Ok(None)`）不注入。
9. **D6（前轮 finding 状态）**：`prompts/review_context.rs` 的前轮清单改为「内容身份求交 → 去重 → 分区注入」；断言覆盖「本轮仍存在 ⇒ 进待办」「本轮已消失 ⇒ 进已关闭且不得要求修改」「同身份多条 ⇒ 折叠为一条」「求交键为内容身份而非 message 指纹」。
10. **复测**：按同一装置（串行交替、独立工作区与端口、`prepare`→等 terminated 落盘→`Enable`）复测 n≥4/臂，指标为 preflight 失败率、需人工介入率、重写轮次、墙钟、usage；**基线数字取本次实测**（preflight 失败 2/4、需人工介入 2/4、墙钟中位数 520.8s / 788.8s）。
11. **D7（写范围诊断与教学精度）**：先固化绝对路径样本的红测（`src/product/work_item_plan_compiler/tests/write_scope_bound.rs`）⇒ 改 `write_scope_bound.rs` 的绝对路径文案分支（同码 Error、不生成 `{bound_prefix}/{anchor}` 改写例）⇒ **替换** `sc_author_teaching.rs:281` 的「写范围路径基准」段（保留既有纪律语义要点 + 增补绝对路径反例与两项独立规则）⇒ 多仓夹具断言追加到 `src/product/work_item_split_engine/tests/prompt_contract_sc_revision.rs`（**不向已 1,630 行的 `tests/prompt_contract.rs` 堆测试**）⇒ 复测固定面字节（对照基线 23,448 / 余 52）与教学段字节（对照基线 563 B）。**判定不放宽**：绝对路径样本仍被拒、仍为同码。
12. **D8（成员清单 / 逐仓实况注入）**：先固化「SC author prompt 侧出现 `## 逐仓实况` 段且逐项在场」「渲染失败 / 预算不足 ⇒ 作者轮不失败、降级缺席」的红测 ⇒ 在作者轮生产构造点装载（复用 `PlanningContextSetResolver::resolve` + `render_compact_inventory`，**不新造成员发现器**）⇒ `WorkItemPlanMarkdownAuthorContext` 新增**可选**字段并注入 `[confirmed_context]` 新段（上下文面）⇒ **分别计量**两段字节、断言上下文面 ≤ 32,000 且固定面 23,500 零新增 ⇒ **实测**成员数与字节（不得引用主方案的自造估算）。**边界**：不替代多成员基线树（属 `plan-baseline-multi-member`）。

**回退**：可逐项回退（G-4 排序、G-5 直供、返修 prompt 构造、投影裁剪、**D5 事前 / D5 事后 / D6** 互相独立）。回退只回到「作者 prompt 更大 / reviewer 投影更全」的既有形态，**不得**回退到「投影静默裁剪且无 delta」或「返修轮逐字节重发且无对账」的状态 —— 那两个形态本 change 的存在理由正是要消除它们。D5 / D6 的回退**各自独立**：D5 事后回退不牵连 D5 事前（一个是报错 message，一个是 prompt 内容），D6 回退不影响前轮清单的注入义务（只回到「不给状态、不去重」的现状）。

**D7 / D8 的回退（各自独立，且不得回退到「诊断误导 / 事实缺席且无 delta」的状态）**：D7 回退 ⇒ 写范围判定与稳定码**不变**（判定面本就不在 D7 改动面内），只是绝对路径回到「与仓内相对写法共用同一改写例」的文案；D8 回退 ⇒ 逐仓实况段整段缺席（回到「SC author 侧没有逐仓事实」的现状），**不得**回退成「部分成员在场且未说明」。二者回退都**不得**触碰「两个预算常量不上调」「固定面 23,500 零新增」「上下文面 32,000 不破」这三条约束。

## Open Questions

以下事项**不改变**本 change 的 specs、架构选型与工作包拆分，可在实施期或后续立项处理：

1. **返修轮「与本轮相关」的判定规则**（按 findings 的 `target_work_item_id` 收敛，还是按 section 引用闭包收敛）—— 属实现细节，须在实施期由集成 owner 选定为**唯一**判定口径，避免出现第二套「相关」解释。
2. **L2 依赖图事实的具体载荷形状**（成员仓对、仓序、边）—— 载荷必须与已确认来源同源；具体序列化形态由实施期定，**不得**在 prompt 侧独立重述口径。
3. **`[exploration_budget]` 段是否纳入** —— 主方案 §6.7 标为次级项且未绑定 requirement；本 change **不引入**，若后续立项须另立 delta。
4. **收益数字（探索期秒数、token 数）** —— 一律待实施后**复测**，本 change 的 design 与 tasks 中所有数字均为**基线对照值**，不是达成承诺。
5. **D8 的成员实况段与 D5 基线清单段在上下文面内的额度分配** —— D8 已给出**唯一**口径：「先算成员实况段字节 → 从上下文面剩余额度扣减 → 两段之和 MUST ≤ 32,000；额度不足 ⇒ 成员实况段整段缺席 + 装载侧记诊断」。若实施期出现两段互相挤占，须由集成 owner 在该口径内选定**唯一**分配方式，**不得**出现第二套「额度怎么分」的解释，也**不得**以裁剪成员的方式腾额度。
