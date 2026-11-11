# Proposal: 教学重驱反馈完备化（SC compile 结构失败的载荷增强）

> 权威来源：主方案 `cadence/designs/2026-11-11_技术方案_WIG三缺陷与多仓事实供给修复_v1.0.md` §4.3（②的根因）、§5.1（包②完整设计）、§6.1（串行序）、§7.3（预算守恒）、§8（实施者事项）、§10.2（已结清项）。
> 归属裁决：`cadence/designs-reviews/2026-11-09_裁决_WIG四change审阅问题处置_v1.0.md` §2.2（全局串行序）与 §3.7（compile 教学块迁出归属＝决策六）。
> 本 proposal 只记录 Why / 范围 / 非目标 / 受影响 capability；架构边界在 `design.md`，验收条款在 `specs/`，工作包在 `tasks.md`。
> **数字纪律**：本文件引用的实测数字（重驱臂与基线臂一致的 run 数、错误码行号漂移序列、重驱 prompt 现状字节、固定面余量）**全部是基线对照值**，一律写成「基线对照值 / 实施后须复测」，**不得**写成已达成结论。

## Why

SC 候选在 generate 段编译失败时，平台会对作者做**恰好一次**教学重驱（`REQ-WSC-09`）。这条路径今天喂给作者的东西**比平台自己已经掌握的事实少**。

根因是源码直读复核后确定的**信息丢弃**，不是能力缺失：

- 编译器诊断对象 `CompilerDiagnostic`（`src/product/work_item_plan_compiler/types.rs:63-80`）**本就带**两个关键字段：`field`（缺哪个 section / 字段）与 `repair_example`（注释逐字写明「恰好一个可回喂的修复示例」）。
- 但把它格式化成回灌理由的 `format_compile_failure_reasons`（`src/web/workspace_ws_handler/run/single_candidate.rs:235-247`）只拼 `code:line:message` ⇒ **`field` 与 `repair_example` 被丢弃**。
- 引擎侧的同构格式化器 `format_compiler_diagnostics`（`src/product/workspace_engine/single_candidate.rs:780`）形态相同 ⇒ 同一诊断在两条通道上都只呈现三段。

后果直接可读：作者重驱时只看到「Work Item 缺少必需 section。」「结构化 section 缺少必需字段。」这类**泛句**，拿不到「缺 `Goal`」与「补一行 `- summary: value`」这样的可执行锚点 ⇒ **重驱时的处境 ≈ 首轮**。基线对照：重驱臂与基线臂结论一致的 run 占 3/8（**基线对照值，实施后须复测**），这正是「重驱 ≈ 独立重掷」的统计指纹，而不是「重驱机制无效」的证据 —— 因为重驱拿到的信息集本来就与首轮等价。

诊断码的来源不是别处（须纠正简报此处口径）：`missing_section` 产自 `work_item_plan_compiler/parse.rs:367` 的 `validate_required_parts()`；`invalid_work_item_id` 产自 `parse.rs:424` 的 `validate_identifiers()`；`unknown_structured_key` 产自 `parse.rs:149-247` 的多分支。`missing_section` 的诊断在产出时就带着 `field = 缺失的 section 名` 与 `repair_example = "### <section>"` —— **事实已在手，只是没被送到作者面前**。

对照组是仓库内**已有的正确做法**：SC 修订轮 prompt（`src/product/work_item_split_engine/prompts/sc_revision.rs:62-89`）复用首轮完整 prompt 并在其前插入 `[previous_source]` 段（durable 上一版 candidate source 全文），使 fresh 返修轮自足、不依赖任何历史会话记忆。**compile 教学重驱是唯一一条「短指令 + 依赖 resume 记忆」的裸奔路径**：它既不带上一版 source，也不带结构契约清单。

另有一个次级事实：生成侧**已经到顶**。首轮 prompt 已有 grammar 全条款、`[minimum_legal_source]`（`work_item_plan_minimum_legal_source()`，`src/product/work_item_split_engine/prompts/sc_author_teaching.rs:134`，其断言可通过 compiler parse）、尾部钳制与标题正反例 few-shot；交付前已有确定性归一化与确定性收敛器。固定规范面 23,500 B 只剩 52 B 余量（**基线对照值**）⇒ **静态注入已无余量**，结构合法率的残余失败只能靠**重驱侧喂什么**来消，而不是继续往首轮里塞字。

## What Changes

本 change 只做一件事：**把教学重驱这一次机会「喂什么」补齐**，并在这次机会用尽后**留下可诊断的降级产物**。零新增模型调用、零新增重试次数、零新增教学资格码。

- **A1 · 诊断五段式**：`format_compile_failure_reasons`（`run/single_candidate.rs:235-247`）由 `code:line:message` 改为 `code:line:field:message:repair_example`。诊断对象里 `field` 与 `repair_example` **本就存在** ⇒ 这是**修既有载荷的信息缺陷**，不是新增载荷。
- **A1-b · 两侧同构（必须同改）**：引擎侧 `format_compiler_diagnostics`（`workspace_engine/single_candidate.rs:780`）MUST 与 runner 侧**同步**改为同形态。否则同一诊断在两条通道上形态分叉（引擎侧仍三段式），问题只修了一半。
- **A2 · 重驱 prompt 补「结构契约三件套」**：
  1. **必需 section 清单** —— **单源** `grammar::STRUCTURED_SECTIONS`（`work_item_plan_compiler/grammar.rs:27`，14 条），并与 `parse.rs:367-395` 的判定逐字对齐（含「`Plan Intent` 可选」「`Blockers` 空即合法」两条豁免），**不得另抄一份清单**；
  2. **必需字段表** —— 从 `parse.rs:30` 的 `REQUIRED_FIELDS`（现为 `pub(super)`，须经 `grammar.rs` 重导出）**单源派生**「section → 必需 key」；
  3. **`[minimum_legal_source]`** —— **单源复用** `work_item_plan_minimum_legal_source()`（`sc_author_teaching.rs:134`），**不复制第二份字符串** ⇒ 首轮固定面逐字节不变。
- **A3 · `[previous_source]` 回灌**：把上一次 delivery source 全文拼入重驱 prompt（形态先例：`work_item_split_engine/prompts/sc_revision.rs:62-89`）。理由：resume 会话记忆不可靠（基线对照：重驱臂与基线臂结论一致的 run 占 3/8），且 provider 无原生 resume 时 resume 会返回 `None` ⇒ A3 同时保证重驱轮的自足性。
- **A4 · 预算耗尽后的「SC 结构失败诊断包」**：终态失败落盘前（`run/single_candidate.rs:1293-1308` 的 compile 收口点与 `:1371-1386` 的 IR 收口点）发一条显式 execution event，携带**五段式诊断全集** + 最后一次 delivery source 的 **hash 与字节数** + `[minimum_legal_source]` 对照引用。这是 `REQ-WSC-28E`「保留失败历史」义务的**可执行形态**。

## Capabilities

### New Capabilities

- 无。本 change 不新建 capability，只在既有 capability 上 ADDED 一条 requirement。

### Modified Capabilities

- `work-item-plan-single-candidate`：**ADDED** `REQ-WSC-32`（教学重驱载荷的完备性与降级可诊断性）——规定回灌诊断 MUST 五段式（含 `field` 与 `repair_example`）、重驱 prompt MUST 携带结构契约三件套与上一版完整 source、教学机会耗尽进入终态前 MUST 发出结构失败诊断包事件；并显式锁死「次数 / 资格码 / 预算常量 / 机会上限语义」四不变。

> 编号与归属说明：本 change 是 `REQ-WSC-32` 的**唯一 owner**（新增号，无既有 owner）。本 change **不 MODIFIED** `REQ-WSC-09`（教学资格清单，由 `mechanicalize-plan-compile-repair` 持有）、**不 MODIFIED** `REQ-WSC-28` 系列（授权窗口与机会记账，由 `bound-sc-generation-teaching` 持有）、**不 MODIFIED** `REQ-WSC-17`（闭合资格，由 `close-contract-findings-before-pass` 持有）⇒ 无任何 requirement 被双 MODIFIED。

## 非目标

- **不增加任何模型调用或重试次数**：教学重驱仍是「原调用在内最多两次」（`REQ-WSC-28`），本 change 不新增第三次机会。
- **不新增教学资格码**：`TEACHABLE_PARSE_FAILURE_CODES`（`run/single_candidate.rs:84`）四码白名单逐字不变，不扩面。
- **不改重驱槽位语义**：两个教学槽位的记账与上限语义不变（现为 `compile_ir_rerun_used` / `structure_rerun_used`，`run/single_candidate.rs:1219-1220`；其后由 `bound-sc-generation-teaching` 收敛为耐久窗口）⇒ 本 change 不碰其语义、也不在 bound 之前抢改。
- **不修生成侧**：首轮固定规范面只剩 52 B 余量，静态注入已到顶；本 change **不改首轮 prompt 一个字节**。
- **不动 `bound-sc-generation-teaching` 的窗口模型本身**：只**消费**它承载的「失败 source 引用与 hash / prompt 引用与 digest」，不改其字段、状态机、账本或预算归属。
- **不承担 compile 教学块的迁出**：`build_work_item_plan_compile_reredrive_prompt` 迁到 `workspace_engine/prompts/sc_compile_teaching.rs` 属**决策六（串行序 0，纯移动）**的既有裁决交付物，**不是 bound**、**也不是本 change** 的任务（见 `design.md` D5）。
- **不修多仓基线与基线事实供给**：属另一 change（`plan-baseline-multi-member` / `rework-prompt-input-surface`），本 change 不夹带。
- **不修必关闭判定**：属 `close-contract-findings-before-pass`，本 change 不改其谓词与七面口径。
- **不抬任何预算常量**：固定规范面 23,500 B 与上下文面 32,000 B 逐字不变；**不得**抬 `large_file_guard` 的登记上限（`tests/it_core/large_file_guard.rs`，默认 1200 行；相关登记：`workspace_engine/prompts.rs`=1681、`review/routing.rs`=1391、`tests/prompt_contract.rs`=1667、`run/single_candidate.rs`=1666）。

## Impact

- **受影响 capability**：`work-item-plan-single-candidate`（ADDED `REQ-WSC-32` 形式的 delta；不改主 specs 文件）。
- **受影响代码面**：
  - `src/web/workspace_ws_handler/run/single_candidate.rs`（A1 回灌理由格式化、A3/A4 取数与诊断包事件、两处终态收口点）；
  - `src/product/workspace_engine/single_candidate.rs`（A1-b 引擎侧同构格式化器）；
  - `src/product/workspace_engine/prompts/sc_compile_teaching.rs`（**决策六产出**，A2/A3 的重驱 prompt 装配落点）；
  - `src/product/work_item_plan_compiler/grammar.rs`（`REQUIRED_FIELDS` 重导出，单源派生必需字段表）；
  - `src/product/work_item_split_engine/prompts/sc_author_teaching.rs`（**只读复用** `work_item_plan_minimum_legal_source()`，不改其内容）；
  - 测试面：`src/product/workspace_engine/tests/sc_compile_reredrive_prompt.rs`、`src/web/workspace_ws_handler/tests/single_candidate_provider_run/reredrive_and_reopen.rs`、`src/product/work_item_split_engine/tests/prompt_contract.rs`（**回归护栏，不加测试**）。
  - 精确文件、命令、测试与提交步骤由 `writing-plans` 写入 `cadence/plans/`，不在本 change 内规定。
- **数据契约**：无新增持久化字段、无新增 wire 字段、无新增 env 开关。降级诊断包走**既有 execution event 通道**。
- **共享面与排序**：本 change 落在全局串行序 **3.5**（`bound` 序 3 之后、`effort` 序 4 之前），硬前置为**决策六（序 0）**与**`bound`（序 3）**；与 `bound-sc-generation-teaching` 共写 `run/single_candidate.rs` ⇒ 次序 bound → 本 change。详见 `design.md`「共享面与排序」。
- **无对外 API / 协议破坏性变更**：诊断文本形态变化属内部载荷增强，wire 形状不变。
