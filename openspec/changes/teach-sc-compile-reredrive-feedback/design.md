# 设计：教学重驱反馈完备化（SC compile 结构失败的载荷增强）

## Context

**动机见 `proposal.md` 的 Why**，本节只列约束设计的事实。

- **诊断对象面**：`CompilerDiagnostic`（`src/product/work_item_plan_compiler/types.rs:63-80`）字段为 `code` / `line` / `field` / `message` / `repair_example` / `kind`。`field` 的注释是「canonical field path 或 grammar field 名称」，`repair_example` 的注释是「**恰好一个可回喂的修复示例**」⇒ 修复锚点是**契约的一部分**，不是可选增强。
- **丢弃点**：`format_compile_failure_reasons`（`src/web/workspace_ws_handler/run/single_candidate.rs:235-247`）只拼 `format!("{}:{}:{}", code, line, message)`；引擎侧同构函数 `format_compiler_diagnostics`（`src/product/workspace_engine/single_candidate.rs:780`）形态相同。
- **诊断码产出点**：`missing_section` 出自 `work_item_plan_compiler/parse.rs:367` 的 `validate_required_parts()`（其构造调用 `diagnostic(MISSING_SECTION_CODE, line, section, "Work Item 缺少必需 section。", &format!("### {section}"))` ⇒ `field = section 名`、`repair_example = "### <section>"`）；`invalid_work_item_id` 出自 `parse.rs:424` 的 `validate_identifiers()`；`unknown_structured_key` 出自 `parse.rs:149-247` 的多分支。
- **豁免判定**：同文件 `:367-395` 内有两条判定豁免 —— `Plan Intent` 缺席合法（缺席时由 validator 以 `intent_undeclared` 停等修订，不在 parse 期硬失败）、`Blockers` 空 section 是「无 blocker」的唯一合法表达。⇒ A2 的 section 清单**必须与这两条判定逐字对齐**，否则清单会把合法形态说成缺失。
- **资格面**：`TEACHABLE_PARSE_FAILURE_CODES`（`run/single_candidate.rs:84`）= `missing_section` / `invalid_ears` / `invalid_work_item_id` / `unknown_structured_key`，四码，**本 change 不动**。
- **重驱 prompt 现状**：`build_work_item_plan_compile_reredrive_prompt`（`src/product/workspace_engine/prompts.rs:209`）= 固定短指令 + 逐条错误原文 + `unknown_structured_key` 分支的条件性白名单 + 登记 ID 清单。**无上一版 source、无必需字段表、无最小合法 source**。
- **正确做法先例**：`build_work_item_plan_markdown_fresh_revision_prompt`（`src/product/work_item_split_engine/prompts/sc_revision.rs:66-89`）在返修段之前插入 `[previous_source]` 段（durable 上一版 candidate source 全文，由调用方按 ref 装载），并声明「全新会话、不依赖任何历史会话」。
- **结构契约单源**：`grammar::STRUCTURED_SECTIONS`（`work_item_plan_compiler/grammar.rs:27`，`[&str; 14]`）、`parse.rs:30` 的 `pub(super) const REQUIRED_FIELDS: [(&str, &[&str]); 14]`、`work_item_plan_minimum_legal_source()`（`src/product/work_item_split_engine/prompts/sc_author_teaching.rs:134`，返回 `&'static str`，首轮已注入且断言可通过 compiler parse）。
- **终态收口点**：compile 失败收口在 `run/single_candidate.rs:1293-1308`（终态 message 含「after one teaching re-drive」，见 `:1296`），IR 失败收口在 `:1371-1386`（同句式，见 `:1374`）。两处都是「最后一次教学机会用尽 ⇒ 进终态」的唯一收口。
- **机会记账面**：`compile_ir_rerun_used` / `structure_rerun_used`（`run/single_candidate.rs:1219-1220`），由 `bound-sc-generation-teaching` 收敛为耐久授权窗口（`REQ-WSC-28` 系列）。
- **已知的对照口径**：仓库内**已有** D5 的事前基线树直供（`rework-prompt-input-surface`）与多仓基线修复（`plan-baseline-multi-member`）—— 那两条治的是**「事实给不给作者」**；本 change 治的是**「编译失败后重驱时给不给修复锚点」**。二者不重叠。

**本设计的边界**：只交付教学重驱**这一次机会的载荷**与**机会用尽后的降级产物**；不开新资格、不改机会计数、不改窗口模型、不动生成侧。

## Goals / Non-Goals

**Goals:**

- 教学重驱回灌给作者的每条编译诊断 MUST 含 `field` 与 `repair_example`（五段式），不得再退化为泛句。
- 重驱 prompt MUST 携带结构契约三件套（必需 section 清单 / 必需字段表 / 最小合法 source），且三者 MUST 来自既有单源，不得另抄。
- 重驱 prompt MUST 回灌上一版完整 source，使重驱轮自足（不依赖 resume 记忆）。
- 教学机会用尽进入终态前 MUST 发出「SC 结构失败诊断包」事件，携带诊断全集与末次 source 的 hash/字节数。
- 引擎侧与 runner 侧的同构格式化器 MUST 同形态，不得分叉。

**Non-Goals:**

- **不开新教学资格**：四码白名单不变，不改 `REQ-WSC-09` 的资格清单正文（由 `mechanicalize-plan-compile-repair` 持有）。
- **不改机会次数与预算归属**：不新增第三次；不改 `REQ-WSC-28` 系列条款（窗口、claim、账本、流水、停止语义）；不改通道 C 的独立有界预算归属。
- **不改窗口模型本身**：只**消费** bound 已在窗口里承载的事实。
- **不修生成侧**：首轮 prompt 逐字节不变，固定规范面 23,500 B 不变。
- **不承担纯移动**：compile 教学块迁出 `prompts.rs` 属决策六（序 0）。
- **不修多仓基线、不修闭合资格**：分属其它 change。
- **不新增持久化字段 / wire 字段 / env 开关**。

## Decisions

### D1 · 回灌诊断改五段式（`code:line:field:message:repair_example`）

把 `format_compile_failure_reasons`（`run/single_candidate.rs:235-247`）的 `code:line:message` 改为 `code:line:field:message:repair_example`。

**为什么是「修既有载荷」而不是「新增载荷」**：`field` 与 `repair_example` 已在 `CompilerDiagnostic` 里（`types.rs:63-80`），`missing_section` 的构造点（`parse.rs:367` 附近）当场就把缺失 section 名写进 `field`、把 `### <section>` 写进 `repair_example` ⇒ 本决策**不新增任何诊断字段、不改编译器**，只改**渲染**。

**为什么必须五段而不是把 `repair_example` 并进 `message`**：五段是**可机器再解析**的形态（code / line 定位、field 定位、repair_example 可直接回喂）。把示例糊进 message 会让「哪一段是示例」依赖自然语言解析。

**兼容性**：`repair_example` 可能为空串（不产示例的诊断码）。五段式在这种情况下仍**占位**（空字段），**不得**因空值退化成三段式 —— 否则同一条通道会同时产出两种形态，解析方要分支。

### D2 · 引擎侧与 runner 侧 MUST 同形态（爆炸半径登记）

`format_compiler_diagnostics`（`src/product/workspace_engine/single_candidate.rs:780`）与 runner 侧 `format_compile_failure_reasons` 是**同源诊断的两个渲染口**，现状形态相同（都只拼三段）。⇒ **两侧同改**。

**为什么这是硬要求**：否则「同一诊断在引擎通道是三段、在 runner 通道是五段」⇒ 人工排查时两个面给出**不同信息量**，且「诊断完备性」这一契约在引擎通道上不成立。**同构格式化器 MUST 是同一份实现或逐字同源的渲染规则**，不得各写一份。

**迁移形态**：为避免两份实现漂移，**优先**把五段式渲染抽成**唯一**的格式化函数，由两侧共同调用；只有在 `workspace_engine` 与 `workspace_ws_handler` 的模块边界不允许共享时才允许两侧各持一份，但必须逐字同源且有同一组断言覆盖。

### D3 · 重驱 prompt 的「结构契约三件套」

重驱 prompt MUST 携带：

1. **必需 section 清单** —— 单源 `grammar::STRUCTURED_SECTIONS`（`grammar.rs:27`，14 条）；
2. **必需字段表** —— 从 `parse.rs:30` 的 `REQUIRED_FIELDS` **单源派生**「section → 必需 key」；
3. **`[minimum_legal_source]`** —— 单源复用 `work_item_plan_minimum_legal_source()`（`sc_author_teaching.rs:134`）。

**三件套各自的边界**：

- section 清单 MUST 与 `parse.rs:367-395` 的判定**逐字对齐**，含「`Plan Intent` 可选」「`Blockers` 空即合法」两条豁免 ⇒ **不得**把清单写成「14 条全部必需」的朴素表述，否则会把合法形态说成缺失，诱导作者去补一个不该补的 section。
- 必需字段表 MUST 由 `REQUIRED_FIELDS` 派生，**不得另抄一份**。`REQUIRED_FIELDS` 现为 `pub(super)` ⇒ 须经 `grammar.rs` **重导出**后消费（`grammar.rs` 是结构契约的既有单源门面，加重导出不新增第二处定义）。
- 最小合法 source MUST **复用同一函数**，**不得复制第二份字符串** ⇒ 首轮固定面因此**逐字节不变**（A2 只影响重驱 prompt，不影响首轮）。

**为什么不把三件套塞进首轮 prompt**：首轮固定面只剩 52 B 余量，且首轮已注入 grammar 全条款与最小合法 source ⇒ 塞进去既无余量、也无新增信息。

### D4 · `[previous_source]` 回灌（A3）

重驱 prompt MUST 含 `[previous_source]` 段与**上一次 delivery source 全文**，形态先例 `sc_revision.rs:66-89`。

**为什么必要**：

- resume 会话记忆**不可靠**：基线对照 —— 重驱臂与基线臂结论一致的 run 占 3/8（**基线对照值，实施后须复测**），即「历史里有」≠「模型会用」。
- provider 无原生 resume 时 resume 返回 `None` ⇒ 只有回灌 source 才能保证重驱轮**自足**。

**取舍**：回灌全文会让重驱 prompt 变长（基线对照：source 体量 5–20 KB 量级）。这是**有界**的 —— 每 candidate 至多一次重驱 ⇒ 每 candidate 至多多付这一次。

**边界**：本段承载的是**上一版 source**（修改对象），不是结构契约（那是 D3）—— 两段职责不同，MUST NOT 合并成一段含糊文本。

### D5 · 迁出归属是**既有裁决**，本 change 不承担迁出

> ⚠️ **迁出归属不是本 change 的决定**：`cadence/designs-reviews/2026-11-09_裁决_WIG四change审阅问题处置_v1.0.md` §3.7（该裁决 §2.2:106 的展开）明确 ——「澄清 effort #6『compile 教学块迁出 prompts.rs』的归属＝**决策六**（`prompts/sc_compile_teaching.rs`），**不是 bound**——bound 的 tasks 不承接该项」。

⇒ **`build_work_item_plan_compile_reredrive_prompt` 的迁出由「决策六（串行序 0，纯移动）」完成**，目标文件 **`workspace_engine/prompts/sc_compile_teaching.rs`**（该文件当前**尚不存在**，属决策六的产出）。

⇒ **本 change 不承担迁出任务**，只在**决策六产出的文件**里改内容 ⇒ 本 change 的文件布局、守卫登记与验收 import 路径**依赖序 0 先落**。

**为什么这条必须写进 design**：若本 change 自作主张在 `workspace_engine/prompts.rs` 内就地改内容，则会与决策六的纯移动**抢同一文件**（`run/single_candidate.rs` 与 `workspace_engine/prompts.rs` 均被序 0 / bound / effort 共写，禁并发）⇒ 顺序一乱，两处语义互相覆盖。

**现状事实**（用于确认落点）：`workspace_engine/prompts.rs` 现 1,667 行，登记上限 1,681（`tests/it_core/large_file_guard.rs`）；`run/single_candidate.rs` 现 1,674 行，登记上限 1,666 ⇒ **当前已红 8 行**（非本 change 引入）。⇒ A1 零净增行；A4 每收口点约 +5~8 行 ⇒ **必须依赖决策六拆出的独立模块**，不得在本文件内加码，**不得**抬登记上限消除超限。

### D6 · A3/A4 的取数源 MUST 复用 bound 的耐久教学窗口模型

> ⚠️ **不得自建第二套管线**：`bound-sc-generation-teaching` 的 tasks 1.1 已把「**失败 source 引用与 hash**、context digest、**prompt 引用与 digest**」规划进 SC 生成教学窗口的耐久模型字段；其 design 的 D5 明确「耐久窗口 → store CAS → 真实 spawn 观察 → handler 单槽 → 流水/故障重放」是**不可拆开上线的纵向闭环**。

⇒ **A3 的 `[previous_source]`** 与 **A4 的「最后一次 delivery source 的 hash 与字节数」** MUST 从**该窗口模型**取数；MUST NOT 由本 change 自行再抓一份 source、再算一遍 hash ⇒ 否则同一事实存在两套管线，且与 bound 的「prompt 引用与 digest」重复。

⇒ **推论（同时是本 change 的排序硬前置）**：本 change **排在 bound 之后** —— bound tasks 明文「**新的教学资格与载荷只在该闭环整体就位后才开放**」。

⇒ **推论（同时是本 change 的边界）**：本 change **不开新教学资格**（四码白名单不变）⇒ 相对 bound 的「载荷才开放」口径，本 change 属**载荷增强**，仍排在 bound 闭环就位之后。

### D7 · 机会次数、资格码、槽位语义与预算常量四不变

本 change MUST NOT：

- 新增第三次机会（`REQ-WSC-28` 明文「原调用在内最多两次」）；
- 新增或变更教学资格码（`TEACHABLE_PARSE_FAILURE_CODES` 四码不变；不碰 `REQ-WSC-09` 的资格清单正文）；
- 改变两个教学槽位的记账与上限语义（`compile_ir_rerun_used` / `structure_rerun_used`，`run/single_candidate.rs:1219-1220`）—— 其收敛为耐久窗口是 bound 的活，本 change **不在 bound 之前抢改同一处**；
- 上调固定规范面 23,500 B、上下文面 32,000 B，或抬高 `large_file_guard` 的任何登记上限。

### D8 · 与 `bound-sc-generation-teaching` 互补正交

| | `bound-sc-generation-teaching` | 本 change |
|---|---|---|
| 管什么 | **几次机会 / 怎么记账 / 何时停**（窗口、原子 claim、ledger 分账、流水、停止语义） | **单次机会喂什么 / 停了之后留什么**（载荷内容、降级诊断包） |
| 载体 | `REQ-WSC-28` / 28A–28E / `REQ-SG-05` | `REQ-WSC-32` |
| 任务形态 | 全是窗口/账本/流水任务，**无任何 prompt 载荷质量任务** | 全是载荷内容与降级产物任务，**不碰窗口/账本字段** |

⇒ **不补、不替代、不前置、不夹带**。A4 是 `REQ-WSC-28E`「保留失败历史」义务的**可执行形态**（把「保留」变成一条带完整诊断与 source 引用的事件），但**不改写** `REQ-WSC-28E` 的条款正文。

### D9 · 降级诊断包（A4）的载荷与时机

**时机**：终态失败**落盘前**，在两处收口点（`run/single_candidate.rs:1293-1308` 的 compile 收口、`:1371-1386` 的 IR 收口）各发一条。

**载荷**：

1. **五段式诊断全集**（每条含 `field` / `repair_example`）；
2. **最后一次 delivery source 的 hash 与字节数**（取自 D6 的窗口模型）；
3. **`[minimum_legal_source]` 对照引用**（让读到诊断包的人能立刻对照一个「已知合法」的最小形态）。

**为什么走 execution event 而不是改终态 message**：终态 message 的既有句式（含「after one teaching re-drive」）是**人类可读的一句话**，塞入全集会破坏既有断言与呈现；而诊断包是**给工具/人排障的完整产物**，两者职责不同。

**为什么终态 message 逐字不动**：既有失败面（`REQ-WSC-18` 的完整诊断 durable 落盘、人工门呈现）依赖它；改它属另一条契约的修改，不在本 change。

**边界**：A4 是**观测面**（多一条事件、多一份产物），**不是**新的自动重试入口、**不是**新的教学机会 ⇒ MUST NOT 由该事件触发任何额外 provider 启动。

### 被否决的备选

| 备选 | 否决理由 |
|---|---|
| **C · 加重驱次数（第三次）** | 与 `REQ-WSC-28`「原调用在内最多两次」**冲突**；且实测错误码行号漂移（`:1→:33→:53→:57→:137`，**基线对照值**）证明模型每轮**整份重写**、错误在**漂移**而非收敛 ⇒ 同样信息集下再掷一次 ≈ 独立重掷，token 线性增而收益不增 |
| **D · 机械补齐缺 section**（autorepair 扩域插入占位行） | 能骗过 parse 的「字段在场且非空」检查，但这是把**结构失败转成内容失败**、**制造假阳性通过**，违反 `UNKNOWN_STRUCTURED_KEY_POLICY=fail_closed` 一路的 fail-closed 契约 ⇒ 平台不得替作者编造结构 |
| **B · fresh 自足重驱**（改用 `build_work_item_plan_streaming_input_fresh`，重驱轮输入 ≈ 首轮 23–55 KB） | **保留为 A 实测仍不收敛时的启用项**（成本约 +20 KB/次，相对 A）。当前不选：首轮固定面只剩 52 B、A 已把「缺什么 + 补什么 + 上一版」三件事补齐，B 是在 A 之外再叠一整套首轮输入。**若 A 实施后实测仍不收敛**，启用 B 是一次独立的、需再次裁决的范围扩大 |
| **只改 runner 侧格式化器**（不动引擎侧） | 同一诊断在两条通道形态分叉 ⇒ 契约在引擎通道不成立，排障时两面信息量不同（见 D2） |
| **把三件套塞进首轮 prompt** | 固定面只剩 52 B 余量，**静态注入已到顶**；且首轮已注入 grammar 全条款与最小合法 source ⇒ 无新增信息，纯挤预算 |
| **在重驱 prompt 里另抄一份 section 清单与字段表** | 两份清单会与 `grammar.rs` / `parse.rs` 漂移 ⇒ 教学口径与编译口径分叉（正是本 change 要消灭的那类分叉） |
| **把诊断包内容塞进终态 message** | 破坏既有终态句式与既有断言，且把「一句话人类摘要」与「完整排障产物」混成一段 |
| **A3 自建 source 抓取管线**（不复用 bound 窗口模型） | 同一事实两套管线，且与 bound 的「prompt 引用与 digest」重复；亦与 D6 的纵向闭环纪律冲突 |

## 共享面与排序

> 本节只登记**共享面与归属**。本 change 不修改任何其它 change 目录。

### 全局串行位置

| 序 | 包 | 与本 change 的关系 |
|---|---|---|
| **0** | **决策六：拆分大文件**（纯移动） | **硬前置**：产出 `workspace_engine/prompts/sc_compile_teaching.rs`；`run/single_candidate.rs` 与 `workspace_engine/prompts.rs` 的纯移动拆分必须先于本 change 的任何实质新增 |
| 1 | `mechanicalize-plan-compile-repair` | 持有 `REQ-WSC-09` 的资格清单扩充；本 change **依赖**其清单口径但**不修改**它 |
| 2 | quota-pool | 与本 change 无共写面 |
| **3** | **`bound-sc-generation-teaching`** | **硬前置**：耐久教学窗口模型承载 A3/A4 的取数事实；共写 `run/single_candidate.rs` ⇒ 次序 bound → 本 change |
| **3.5** | **本 change** | — |
| 4 | effort | 与本 change 无共写面（`run/single_candidate.rs` 的 effort 侧顺延在本 change 之后） |
| 5 | `isolate-plan-criterion-namespace` | 与本 change 无共写面 |
| 6 | `close-contract-findings-before-pass` | 与本 change 无共写面 |
| 7 | `rework-prompt-input-surface` | 与本 change 无共写面（其 `sc_author_teaching.rs` 复用是**只读**消费） |

### 共写文件与相对次序

| 对端包 | 共写面 | 相对次序 |
|---|---|---|
| 决策六（序 0） | `workspace_engine/prompts.rs` → 迁出产出 `workspace_engine/prompts/sc_compile_teaching.rs`；`run/single_candidate.rs` | 决策六 → 本 change |
| `bound-sc-generation-teaching`（序 3） | `run/single_candidate.rs`（槽位与收口点所在面） | bound → 本 change |
| `mechanicalize-plan-compile-repair`（序 1） | `REQ-WSC-09` 资格清单正文（本 change **只读**引用，不改） | mechanicalize → 本 change（无共写） |

### 本 change 的独占写入面

- `workspace_engine/prompts/sc_compile_teaching.rs`（**决策六产出**后由本 change 改内容）；
- `work_item_plan_compiler/grammar.rs`（`REQUIRED_FIELDS` 重导出）；
- `workspace_engine/single_candidate.rs`（A1-b 引擎侧格式化器）。

### 只读消费面

- `work_item_split_engine/prompts/sc_author_teaching.rs`（`work_item_plan_minimum_legal_source()`）；
- `work_item_plan_compiler/parse.rs`（`REQUIRED_FIELDS` 的定义处，经 `grammar.rs` 重导出消费）；
- `bound-sc-generation-teaching` 的耐久教学窗口模型（A3/A4 的取数源）。

### 归档前置

本 change **不得**在 `bound-sc-generation-teaching` 的耐久教学窗口模型就位前归档 ⇒ 因为 A3/A4 的取数源（D6）在窗口落地前不存在可消费的取数面。归档门禁与 bound 的归档门禁**不是同一条**：bound 依赖 mechanicalize 的 `REQ-WSC-09` 扩充，本 change 依赖 bound 的窗口模型就位。

## 回退

- **回退只减载荷、不减机会**：回退 = 撤掉 A2/A3/A4 与五段式 ⇒ 重驱回到「三段式 + 无契约三件套 + 无 previous_source」的状态；**教学机会的次数、资格码、记账与停止语义 MUST NOT 因回退而变化**（那些是 bound 的合同，不是本 change 的）。
- **不回退到双侧形态分叉**：回退 MUST NOT 只撤一侧格式化器而留另一侧五段式 ⇒ 两侧同改同撤。
- **不清既有产物**：已发出的诊断包事件与已落盘的失败历史保持可读，回退不改其内容寻址。
- **降级诊断包是纯观测**：回退 MUST NOT 使任何既有终态从「有诊断包」变成「无诊断包且无失败历史」⇒ 已发出的事件不被删除。

## Risks / Trade-offs

- **重驱 prompt 体积上升**（五段式 + 三件套 + 上一版 source 全文）⇒ 每 candidate 至多多付这一次，总量有界；固定面与上下文面**零触碰**（重驱 prompt 不属于这两个面）。
- **上一版 source 回灌可能携带超长内容** ⇒ 须以既有预算/上限口径承载，且**不得**用「动态段不算预算」的新例外绕过任一面（沿用 `rework-prompt-input-surface` 2.4 的两面归属纪律）。
- **五段式改变了既有诊断文本形态** ⇒ 依赖 `code:line:message` 形状的既有断言 MUST 同步更新，但**既有语义断言不得删**（既有断言「全部保留仍绿」是本 change 的验收项之一）。
- **`REQUIRED_FIELDS` 重导出扩大了 `grammar.rs` 的可见面** ⇒ 只重导出、**不复制**；`grammar.rs` 仍是结构契约的唯一门面。
- **A4 的事件是纯增量产物** ⇒ 多一条 execution event，不改终态 message、不改既有失败面。
- **依赖两个外部前置（决策六、bound）** ⇒ 本 change 无法单独完成归档；这是**有意的**，因为脱离窗口模型自行抓 source 就会造出第二套管线（D6）。

## Migration Plan

1. **序 0 先落**：决策六完成纯拆分，产出 `workspace_engine/prompts/sc_compile_teaching.rs`；本 change 的落点随之确定。
2. **序 3 先落**：`bound-sc-generation-teaching` 的耐久教学窗口模型就位，可提供「失败 source 引用与 hash / prompt 引用与 digest」。
3. **本 change 分三步落地**：① A1/A1-b 诊断五段式（两侧同改）+ A2 三件套（纯 prompt 内容，无外部依赖）⇒ 已有可观察的反馈完备性提升；② A3 回灌（依赖 2.）；③ A4 诊断包（依赖 2.）。①与②③④之间**无顺序耦合**，但 ②③④ MUST NOT 在 2. 之前落地。
4. **数据面**：无新增持久化字段 ⇒ 无数据迁移、无兼容读侧分支。
5. **回退**：见「回退」节。

## Open Questions

无阻塞项：本设计的边界、归属与次序均由主方案 §5.1 / §6.1 / §10.2 与处置裁决 §2.2 / §3.7 给定，未留下需要 owner 再裁决才能推进的关键决策。备选 B（fresh 自足重驱）保留为**条件启用项**，其启用条件是「A 实施后实测仍不收敛」—— 该判定属统计收益门（M1），不属本 change 的契约层前置。
