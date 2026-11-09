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

## 共享面与排序

按全局串行表（处置裁决 §2.2，唯一权威），本 change 排在**序 7（最后）**。**这些 change 目录本 change 一律不改**，仅在此登记共写文件与相对次序。

| 共享文件 | 对方 change | 相对次序 | 本 change 的动作 |
|---|---|---|---|
| `src/product/work_item_split_engine/prompts.rs` | `isolate-plan-criterion-namespace`（序 5：`:185,:306` 的 WIC ID 纪律加入） | **isolate → 本 change** | 本 change 改 `:395` `budget_markdown_context` 的签名与调用点（L1/L2 直供）；**不改** isolate 的 `:185` / `:306` 两处 ID 纪律措辞，也不改其语义 |
| `src/product/work_item_split_engine/prompts/sc_author_teaching.rs` | `mechanicalize-plan-compile-repair`（序 1：波次 7「prompt 分层瘦身」≤8K 字符 / ≤25 硬措辞） | **mechanicalize → isolate → 本 change** | 本 change 在其瘦身之后改写作者教学面（若 L1/L2 直供需要新增教学措辞）；顺序相反则瘦身的度量基线被本 change 改动，验收失真（双边登记：mechanicalize 侧同样登记本 change 后续接手） |
| `src/product/work_item_split_engine/tests/prompt_contract*.rs`（`prompt_contract.rs`、`prompt_contract_weak_model.rs`、`prompt_contract_sc_revision.rs`） | `mechanicalize-plan-compile-repair`（按新契约更新既有断言，不许删测试） | **mechanicalize → isolate → 本 change** | 本 change 更新作者 prompt 与返修 prompt 的契约断言（直供段存在性、返修轮不带首轮全文）；两个 change 的断言合并在同一提交内落地，避免同一断言被两处改写 |
| `src/product/workspace_engine/prompts/review.rs` | `close-contract-findings-before-pass`（序 6：单候选 reviewer prompt 装配与呈现口径） | **close → 本 change** | 本 change 改单候选 reviewer 的投影**呈现段**（`single_candidate_input.inc.rs`）；**不改** close 的有效必关闭判定与呈现口径，二者是不同物 |
| `src/product/work_item_split_engine/prompts/sc_revision.rs` | `close-contract-findings-before-pass`（序 6：`:147` 的返修轮呈现口径 —— 重开后仍为 `suggestion` 的契约缺口按有效必关闭呈现） | **close → 本 change** | 本 change 改 `:27` 的**prompt 构造口径**（停止逐字节复用首轮全文）；**不改** close 在 `:147` 的 severity/呈现口径 |

**排序结论**：决策六纯拆分 → `mechanicalize`（prompt 瘦身与其 fixture/断言）→ `isolate`（WIC ID 纪律）→ `bound` → `effort` → `close`（返修轮呈现口径）→ **本 change**（序 7，最后）。本 change 落在最后，是因为它的三处写入面（`prompts.rs:395`、`sc_revision.rs:27`、reviewer 投影呈现）都已被前序 change 改过 —— 在前序语义未稳定时并发改同一文件，会让「哪条语义变化导致了验收结果变化」无法归因。

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
| 探索预算硬约束（`[exploration_budget]` 段，方案 §6.7） | 主方案标为「次级项」，非本 change 契约 | 不引入 |

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

## Migration Plan

1. **锚定**：在**未改实现**的状态下记录基线 —— 作者 prompt 实测字节与 margin、返修轮 prompt 实测字节、`structure` 段实测长度与成员仓出现次数、reviewer 投影各段实测字节。基线数字只作**对照**，不作为「已达成」结论。
2. **G-4 先落**（纯纠错、零契约变更面）：多仓摘要排序改为成员仓优先；单仓路径断言逐字不变。
3. **X2 投影裁剪**：先按 delta 改 reviewer 侧**呈现段**（`single_candidate_input.inc.rs`），保留平台侧绑定与同代校验；对照基线记录各段字节与判断必需内容是否齐备。
4. **G-5 直供**：改 `budget_markdown_context` 签名与调用点，把 L1（成员仓摘要）与 L2（依赖图事实）作为平台直供段注入；L3 分配口径不变。
5. **返修 prompt 瘦身**：改 `sc_revision.rs:27` 的构造口径（带本轮 delta + 本轮相关既有上下文，不再逐字节重发首轮全文）；同步更新 `prompt_contract_sc_revision.rs` 断言。
6. **教学与断言合并提交**：`sc_author_teaching.rs` 与 `prompt_contract*.rs` 的改动与前序 change 的合并在同一提交内落地（见「共享面与排序」）。

**回退**：可逐项回退（G-4 排序、G-5 直供、返修 prompt 构造、投影裁剪互相独立）。回退只回到「作者 prompt 更大 / reviewer 投影更全」的既有形态，**不得**回退到「投影静默裁剪且无 delta」或「返修轮逐字节重发且无对账」的状态 —— 那两个形态本 change 的存在理由正是要消除它们。

## Open Questions

以下事项**不改变**本 change 的 specs、架构选型与工作包拆分，可在实施期或后续立项处理：

1. **返修轮「与本轮相关」的判定规则**（按 findings 的 `target_work_item_id` 收敛，还是按 section 引用闭包收敛）—— 属实现细节，须在实施期由集成 owner 选定为**唯一**判定口径，避免出现第二套「相关」解释。
2. **L2 依赖图事实的具体载荷形状**（成员仓对、仓序、边）—— 载荷必须与已确认来源同源；具体序列化形态由实施期定，**不得**在 prompt 侧独立重述口径。
3. **`[exploration_budget]` 段是否纳入** —— 主方案 §6.7 标为次级项且未绑定 requirement；本 change **不引入**，若后续立项须另立 delta。
4. **收益数字（探索期秒数、token 数）** —— 一律待实施后**复测**，本 change 的 design 与 tasks 中所有数字均为**基线对照值**，不是达成承诺。
