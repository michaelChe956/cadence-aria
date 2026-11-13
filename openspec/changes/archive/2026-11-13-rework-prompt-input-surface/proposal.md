# Proposal

> ## ⚠️ 处置声明（归档前落档 · 2026-11-13）
>
> **本 change 以 `--skip-specs` 归档**：其 MODIFIED REQ-WSC-06 的 45 个场景**不整体合并进主规格**。
>
> **为什么**：这 45 个场景中**只有 8 个对应的行为已实施并复测**，其余仍指向未实施的工作包。整份合并等于**宣称未实施行为已交付**。
>
> **已交付的 8 个场景**已由拆分件 `deliver-d5-d6-fact-injection`（归档于 `openspec/changes/archive/2026-11-13-deliver-d5-d6-fact-injection/`）**带 spec 同步**落入主规格 `openspec/specs/work-item-plan-single-candidate/spec.md`（REQ-WSC-06 现为 36 场景 = 原有 28 逐字 + 已交付 8；描述段 244 字符）：越界路径回灌最近候选 / 基线树事实直供 / 基线树清单不得占用固定规范面预算 / 注入前轮结构化清单 / 重复判定不靠记忆 / 前轮 finding 状态求交按内容身份 / 前轮 finding 分区注入 / 前轮清单同身份去重。
>
> **未实施部分的落点**（事实 → 现状 → 归谁 → 建议落点）：
>
> | 场景簇 | 现状 | 归谁 | 建议落点 |
> |---|---|---|---|
> | 成员仓摘要不得被点条目挤掉；author 上下文分级供给（L1 与名额分配） | 未实施 | 集成 owner | 由 `repair-wig-fact-projections` 的 **C1/C2** 承接并 MODIFIED REQ-WSC-06 |
> | author 上下文分级供给之 **L2 依赖图事实直供** | 未实施 | 集成 owner | 留待后续新 change |
> | 返修轮不重发首轮全文；返修轮保留本轮修复所需上下文 | 未实施 | 集成 owner | 留待后续新 change（与 `repair-wig-fact-projections` 的 B4 **不重叠**：B4 只做「作者只收有效必关闭条目」，不改返修上下文构造） |
> | 投影最小内容为缺口与消费闭环；写范围冲突事实按同仓限定 | 未实施 | 集成 owner | 留待后续新 change（跨仓 27/29 假阳性的机械命名域） |
> | 写范围越界诊断区分绝对路径与仓内相对写法；写范围教学反例精度 | 未实施 | 集成 owner | `WIG写范围诊断与成员清单注入增补` 的 Task2（诊断小闭环），留待后续 |
> | 逐仓实况事实直供；逐仓实况成员缺席如实说明 | 未实施 | 集成 owner | 同上增补的 Task4/Task5，留待后续 |
> | §8 验收剩余（8.3 归档模拟、8.7 防漏检、8.8 探索次数观测量、8.11 描述段增量义务、8.12 第二轮三臂脚手架拆除） | 未实施 | 集成 owner | 8.7/8.8 依赖 §4 投影裁剪（未做）⇒ 一并留待后续；**8.12 的脚手架拆除**由 `repair-wig-fact-projections` 承接（它本就改 `work_item_plan_compiler/validate.rs`）；8.11 的两项义务已在拆分件描述段压缩中**未被削弱**（基线树事实直供 / 前轮 finding 状态由平台机械复算 逐字在位） |
>
> **本 change 归档不宣称**：未实施义务已交付；「未产出计划」类失败的成因已定位（复测判决其为环境性）；D6 的因果效果已确证（复测判决「重复报归零」缺同窗口阳性对照 ⇒ **待重测**）。
>
> **已交付部分的既有证据**（非本归档件新增）：累积 35 个有效 run、跨 4 个时段窗口 ⇒ 二次评审中**出现重复报** 8/9（89%）→ **0/11（0%）**，p=0.00131；计划产出 4/15→6/20（p=0.179，不显著）；编译通过 1/8→2/8。

> 权威来源：
> - 主方案 `cadence/designs/2026-11-08_技术方案_WIG生成加速与收敛性加固_v1.0.md` §3.4（一致性裁决 X2）、§6.3（两个平台侧确定性缺陷）、§6.6（工作包 G-4 / G-5）、§8.0（待办登记）；
> - 处置裁决 `cadence/designs-reviews/2026-11-09_裁决_WIG四change审阅问题处置_v1.0.md` §2.2（共享面全局串行顺序）、§2.3（命名域与跨 change 归属的唯一性）、§7.2（X2 的 MODIFIED REQ-WSC-06 无 owner → 另立 change 承载）；
> - 实测证据 `cadence/analysis-docs/2026-11-08_WIG实测证据基线_v1.0.md` §2 / §3 / §5 / §6 / §17。
> 本 proposal 只记录 Why / 范围 / 非目标 / 受影响 capability；架构边界在 `design.md`，验收条款在 `specs/`，工作包在 `tasks.md`。

## Why

本 change 承接两个**尚无 change 承载**的遗留缺口。

**缺口一：author 输入面成本（G-4 / G-5）尚无 owner。**（**本 change 建立时的状态** —— 该缺口由本 change 承接，下文「What Changes」逐项落地。）主方案 §6.6 的 G-4 / G-5 在 §8.0 映射表里**没有对应 change**；这些缺陷都在**平台侧**、不在模型侧：

- G-4：聚合根顶层条目按名排序后截断到上限 12（`context.rs:233-234`），点条目占满名额，**三个业务成员仓全部落榜** —— 实测 `structure` 段仅 276 字符、5 个仓名出现 **0 次**，是模型 9 次探索的**直接诱因**。
- G-5：`budget_markdown_context`（`prompts.rs:395`）把上下文按固定比例压进同一预算，作者拿不到平台已确认的成员仓摘要与依赖图事实，只能自行 Read/Glob 补事实 —— 实测首轮 author 产出 markdown 前 **17 次串行工具往返**、探索期 **275.4 秒**。
- 返修轮更贵：`sc_revision.rs:27` 的返修 prompt **逐字节复用首轮完整 prompt**，每轮重付 **32KB 静态规范面**（run1 prompt 实测 **32,609 字符**；author 侧 `cache_read_tokens` 仅 **128**）。
- 另一侧同样有账：reviewer 侧 input **61,051 token**，而**作者的探索结论没有任何形式传递给 reviewer**。

**缺口二：MODIFIED REQ-WSC-06 无 owner。**（**本 change 建立时的状态** —— 本 change 即该缺口的承载方，见下文「编号与归属说明」。）一致性裁决 X2（主方案 §3.4）已裁定：方案 E 的 reviewer 投影裁剪**与主规格 REQ-WSC-06 字面冲突**，必须带 MODIFIED delta，**不得静默裁**；「只留同仓冲突」必须显式收窄主规格 `:377` 的**无仓限定措辞**并给理由（跨仓 forbidden/exclusive 在 multi-repo 场景无意义，实测 **29 条写范围冲突中 27 条为跨仓假阳性**）。处置裁决 §7.2 定案：**承载方须另立 change，不得由 isolate 夹带**。

本 change 即承载方。

### 缺口三：作者拿不到 plan 基线树的事实（纪律已在，事实不在）

**产品全流程实测**（4 run，同一真实案例、串行交替、独立工作区与端口；证据见 `cadence/analysis-docs/2026-11-09_分析报告_WIG产品流程实测与根因锁定_v1.0.md`）：

- 作者 prompt **有纪律、无事实**：首轮 32,609 字符 / 重驱 34,173 字符，**两者都不含**基线树路径清单（`base_tree` / `tree_paths` / `available_paths` 全无命中）；prompt 里的仓库路径只是来自 issue/story/design 的零散示例。
- 真实案例 frontend 基线树**只有 11 个路径且完全没有 `tests/` 目录**，作者却写了 `frontend/tests/integration/notice-delete-e2e.mjs` 与 `/tmp/notice-delete-e2e.mjs` ⇒ **必然**越界。
- 越界被 preflight 机械校验拦下（`acceptance_path_not_in_baseline`）⇒ 教学重驱；实测 **2/4 run** 命中，其中 **1/4 最终升级 `human_required`**（自动预算耗尽，等人工）。
- 教学重驱本身**没有修改对象**：实测两次重驱的工具调用均为 **0**（26.4s / 152.4s）⇒ 模型不查仓库、只能盲改。

⇒ 既有 Scenario「验收路径基线纪律」（教学义务）**已在代码里**（`work_item_split_engine/prompts.rs:347`），**教学单靠不足** ⇒ 需要**把事实给出去**，而不是再加一段教学。

### 缺口四：前轮 findings 清单不含「本轮是否仍存在」，重复报错升级人工

实测（`xhigh-4324`）：

1. 重驱**确实修好了**（`intent_target_codebase` 改为授权逻辑代码库 ID，两个 source revision 哈希不同）；
2. **机械侧已清零**（第二版机械报告绑定新 revision，`intent_unexecutable` 消失）；
3. 评审却**重复报 4 条同码 must_fix** ⇒ 平台自带的「不要无限第 N 次 revise」纪律被动触发 ⇒ 改判 `human_required` ⇒ session 终态 `stopped_needs_human`（**等人工介入**）。

原因：`### Previous Review Round Findings` 把上一轮 finding 原样列为待办，**不含**「本轮是否仍存在」的机械事实 ⇒ 评审只能靠自由文本判断「是否已解决」⇒ 被上一轮锚定。既有 Scenario「重复判定不靠记忆」要求 reviewer **依据注入清单**判重，但清单里**没有**可判的事实。

⇒ 两个缺口的共同形状：**平台已持有事实（基线树 / 当前机械报告），但没把它交给需要它的那一环**。这与本 change 既有主题（"prompt 承载什么、由谁供给"）**同构**，故一并由本 change 承载，不另立。

## What Changes

- **reviewer 投影最小内容重定义（X2）**：只读契约覆盖投影的呈现内容收窄为四类 —— 能力覆盖缺口（gaps-only）、handoff 消费闭环、**同仓**写范围冲突、Projection Validation Report；显式收窄主规格 `:377` 的无仓限定措辞并写明收窄理由，不以「信息仍可从 CCC 推导」代替收窄。
- **author 上下文分级供给（G-5）**：L1 成员仓摘要与 L2 依赖图事实由**平台直接注入 prompt**，不再依赖模型自行 Read/Glob 补事实；L3 维持既有预算约束。
- **成员仓摘要排序修正（G-4）**：`work_item_split_engine/context.rs:233-234` 改为成员仓优先，成员仓不得再被点文件/点目录条目挤掉；单仓摘要口径维持 REQ-PLN-17 的逐字不变义务。
- **返修 prompt 不复用首轮全文**：返修轮只带**本轮 delta + 与本轮相关的既有上下文**，MUST NOT 逐字节重发首轮完整 prompt；本轮 findings 回灌与逐条硬性修复指令保持注入，findings 的 `required_action` 所指向的 section 与其依赖上下文在返修轮仍须可得。
- **规格落点**：以上四项写成一个 **MODIFIED REQ-WSC-06**（`work-item-plan-single-candidate`）：主规格该 requirement 的**原有场景标题与正文逐字保留**，只追加场景 + 改写描述段。
- **【新增 · 缺口三】基线树事实直供（事前）+ 越界报错回灌最近候选（事后）**：
  - **事前**：作者轮启动前由平台从**校验器同源**的 plan 基线树装载可用路径清单，作为 **L1 的扩展**注入 `[confirmed_context]`（落在**上下文面**，不动固定规范面预算常量）；无仓 issue 不注入；装载失败**不阻断**作者轮（既有 fail-closed 点仍在 Evaluate 阶段）；返修轮携带**同一份**清单。
  - **事后**：`acceptance_path_not_in_baseline` 的 Error finding message 追加**基线树中同前缀最近的现存路径**（无候选时如实说明），给教学重驱一个具体修改对象；`ACCEPTANCE_PATH_NOT_IN_BASELINE_REPAIR_ACTION` 常量**逐字不动**。
- **【新增 · 缺口四】前轮 findings 清单附本轮机械复算状态并分区**：平台以**内容身份**（`contract_field` vs 机械报告 `code`）对当前候选机械报告求交 → 同内容身份去重 → 清单分「待办 / 已关闭」两区注入；**「是否已解决」的判定主体从模型移到平台**（模型不得依据清单文本自行推断）；fingerprint 仍保留但只承担重复标注职责。

## Non-Goals（本 change 明确不做）

- 不做 G-2（MCP 输出截断）/ G-3（Bash 输出截断）/ G-6（缓存 TTL 1h）—— 这三项是 **SC author 出口的 env 层**，由 `explicit-sc-author-effort` 承载该出口的 env 构造机制；本 change 只在 `design.md` **登记**相邻关系，**不夹带**。
- 不改 effort 档位（G-1）—— 裁决 §3 已定案为默认 `high`，由 `explicit-sc-author-effort` 承担。
- 不改 canonical 校验器的 fail-closed 语义、不删平台侧 source/IR/机械报告的绑定与同代校验 —— 投影裁剪只裁**呈现面**。
- 不改 legacy/story/design reviewer 路径的输入面。
- 不 bump 任何预算常量（author prompt 质量预算 23,500 B、`WORK_ITEM_PLAN_MARKDOWN_PROMPT_MAX_BYTES` 等）；新增直供内容须在既有预算内成立。
- 不触碰 `isolate-plan-criterion-namespace` 的命名域与指纹代际面，不触碰 `close-contract-findings-before-pass` 的有效必关闭判定面。
- 本轮**只出规划产物**，不改 `src/**`。
- **不做方案 A（引用域统一 / cross-WI）** —— 该缺口由 `isolate-plan-criterion-namespace` 承载；本次实测（同输入下 cross-WI **0/79**，而历史 run1 同一份 prompt 产出 15 条）说明其「必然触发」因果链**未被支持**，属**优先级校准**，不属本 change 范围（事实登记见 `design.md`「不承载什么」与实测分析报告 §5.0）。
- **不改 effort 档位判定** —— 实测 n=2/档未观测到收敛性或产物质量差异（收敛性各项计数差值为 0），故 effort 档**不作为本轮优化杠杆**；该事实只登记，不改 `explicit-sc-author-effort` 的任何条款。

## Capabilities

### New Capabilities

- 无。本 change 不引入新能力路径，全部落在既有 capability 的 delta 上。

### Modified Capabilities

- `work-item-plan-single-candidate`：**MODIFIED** `REQ-WSC-06`（prompt 分层职责）—— 承载 reviewer 投影最小内容重定义（X2）、author 上下文分级供给（G-5）、成员仓摘要排序修正（G-4）与返修 prompt 不复用首轮全文。

> 编号与归属说明：本 change 是 **REQ-WSC-06 的唯一 owner**（此前无 owner，见处置裁决 §2.3 与 §7.2）。本 change **只 MODIFIED REQ-WSC-06 一条 requirement**，与其它 active change 无同一 requirement 被双 MODIFIED。

## Impact

- **author 输入面**：`work_item_split_engine/context.rs`（成员仓摘要排序）、`work_item_split_engine/prompts.rs`（`budget_markdown_context` 签名与调用点）、`work_item_split_engine/prompts/sc_revision.rs`（返修 prompt 构造）。
- **reviewer 输入面**：`workspace_engine/prompts/review_parts/single_candidate_input.inc.rs`（投影与供给/需求段的呈现面）、`workspace_engine/prompts/review.rs`（单候选 reviewer prompt 装配）。
- **规格面**：`openspec/specs/work-item-plan-single-candidate/spec.md` 的 REQ-WSC-06（归档后生效）；`REQ-PLN-17` 的单仓逐字不变义务**保持不变**、不被本 change 修改。
- **【新增写入面】**（D5 / D6，见 `design.md`「共享面与排序」的补登记行）：`src/product/work_item_plan_compiler/validate.rs`（D5 事后）、`src/product/workspace_engine/prompts/review_context.rs`（D6）、`src/web/workspace_ws_handler/run/single_candidate.rs`（D5 事前装载点）、`src/product/work_item_split_engine/prompts.rs` 的 `WorkItemPlanMarkdownAuthorContext`（D5 事前新增可选字段）。四者分别排在 `mechanicalize`（序 1）与 `close`（序 6）之后 —— 本 change 恒在序 7 ⇒ 相对次序由既有全局序天然满足。
- **共享文件与串行次序**：见 `design.md`「共享面与排序」—— 本 change 排在全局序 **7（最后）**；与 `isolate-plan-criterion-namespace` 共享 `work_item_split_engine/prompts.rs`（isolate `:185,:306` 的 WIC 命名纪律 ↔ 本 change `:395` 的上下文供给），次序 **isolate → 本 change**；与 `mechanicalize-plan-compile-repair` 共享 `sc_author_teaching.rs` 与 `prompt_contract*.rs` fixtures，次序 **mechanicalize → isolate → 本 change**；与 `close-contract-findings-before-pass` 共享 `prompts/review.rs` 与 `sc_revision.rs`（close `:147` 的呈现口径 ↔ 本 change `:27` 的 prompt 复用），次序 **close → 本 change**。
- **不在本 change 范围内**：G-2 / G-3 / G-6（SC author 出口 env 层）、G-1（effort 档位）、教学窗口与机械处置面（`bound-sc-generation-teaching` / `mechanicalize-plan-compile-repair`）、契约类 finding 免阻断闭环（`close-contract-findings-before-pass`）。
