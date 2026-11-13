# Tasks: teach-sc-compile-reredrive-feedback

> 纪律：每个工作包先固化/先红再改实现（TDD）；精确文件、命令、测试与提交步骤由 `writing-plans` 写进 `cadence/plans/`，本文件只给高层可验收工作包。
> 契约来源：`specs/work-item-plan-single-candidate/spec.md`（**ADDED** `REQ-WSC-32`）；架构边界、归属裁决与串行次序见 `design.md`。
> **排序前提**：`design.md`「共享面与排序」的全局串行链（决策六纯拆分 → mechanicalize → quota-pool → **bound** → **本 change（序 3.5）** → effort → isolate → close → rework）必须被遵守。
> **数字纪律**：本文件引用的实测数字（重驱臂与基线臂结论一致的 run 数 3/8、错误码行号漂移序列 `:1→:33→:53→:57→:137`、固定面 23,500 B 余 52 B、重驱 prompt 现状体量）**全部是基线对照值**，一律写成「基线对照值 / 实施后须复测」，**不得**写成已达成结论。

## 0. 前置：排序确认与基线锚定（不改行为）

- [ ] 0.1 **排序确认**：确认**决策六（串行序 0）**已先落地并产出 `workspace_engine/prompts/sc_compile_teaching.rs`（compile 教学块迁出 = 纯移动，本 change **不承担迁出**），确认**`bound-sc-generation-teaching`（序 3）**的耐久教学窗口模型已就位且可提供「失败 source 引用与 hash / prompt 引用与 digest」。验证：两个前置在共享文件的相对次序与 `design.md` 表逐行一致；缺任一前置时本 change 的 A3/A4 **不得开工**
- [ ] 0.2 **独占性复核**：确认本 change 的 spec delta **只 ADDED `REQ-WSC-32` 一条**，且未 MODIFIED `REQ-WSC-09`（`mechanicalize-plan-compile-repair` 持有）、未 MODIFIED `REQ-WSC-28` 系列（`bound-sc-generation-teaching` 持有）、未 MODIFIED `REQ-WSC-17`（`close-contract-findings-before-pass` 持有）⇒ 无任何 requirement 被双 MODIFIED。验证：全目录 grep 复核，双改为零
- [ ] 0.3 **基线锚定**：在**未改实现**的状态下记录并留档 ——（a）重驱回灌理由的现状形态与字节；（b）重驱 prompt 的现状字节（对照「短指令 + 错误行」基线）；（c）固定规范面 23,500 B 与上下文面 32,000 B 的现状余量（基线：固定面余 52 B）；（d）终态失败事件流的现状条数与内容。验证：基线逐项可复现同一数值并登记在案；**该数值只是对照，不得写成已达成结论**
- [ ] 0.4 **无回归护栏锚定**：确认 `work_item_split_engine/tests/prompt_contract.rs` 的 `work_item_plan_markdown_prompt_inlines_grammar_boundaries_and_real_findings` 与 `sc_author_prompt_tail_clamps_heading_language_and_raises_budget` 在**未改实现**上为绿，并把这两条作为本 change 的**固定面零触碰**护栏基线。验证：两条在基线上为绿且被登记为不可放宽的断言

## 1. A1 · 回灌诊断改五段式（REQ-WSC-32）

- [ ] 1.1 **runner 侧五段式**：教学重驱的回灌理由由「码:行号:消息」改为「码:行号:缺失字段:消息:修复示例」，逐字取自编译器诊断对象既有字段，**MUST NOT** 新增编译器字段、**MUST NOT** 改编译器判定。验证：构造含缺失字段与修复示例的诊断时，回灌理由逐段可解析且两字段逐字在场
- [ ] 1.2 **空修复示例仍占位**：诊断未产出修复示例时，五段式的对应位置 MUST 仍占位，**MUST NOT** 退化成三段式。验证：同一通道上「有示例」与「无示例」两条诊断产出**同构**形态
- [ ] 1.3 **引擎侧同构**：引擎侧同源诊断渲染口 MUST 与 runner 侧产出**同构形态**（优先抽成唯一实现由两侧共用；必须分处两份时须逐字同源且有同一组断言覆盖）。验证：同一诊断经两条通道产出的形态逐段一致；**MUST NOT** 一处含缺失字段与修复示例、另一处只含三段
- [ ] 1.4 **既有断言保留**：依赖旧三段式形状的既有断言 MUST 同步更新，**既有语义断言一条不删**（教学重驱 prompt 的既有措辞、既有错误原文逐条回灌行为保持）。验证：既有断言集更新后全绿且逐条可追溯到被改的形状
- [ ] 1.5 **requirement 映射**：本工作包落在 `REQ-WSC-32` 的场景「回灌诊断不得丢弃缺失字段与修复示例」与「两条诊断通道形态同构」。验证：场景与工作包一一对账无遗漏

## 2. A2 · 重驱 prompt 的结构契约三件套（REQ-WSC-32）

- [ ] 2.1 **必需 section 清单（单源）**：重驱 prompt MUST 携带结构契约单源的必需 section 清单，**MUST NOT** 另抄一份；清单 MUST 含「可选 section」与「空即合法 section」两条豁免表述，**MUST NOT** 把全部 section 说成必需。验证：清单内容与结构契约单源逐项一致；两条豁免在 prompt 中可断言；一份清单被修改时 prompt 侧随之变化（无第二份副本）
- [ ] 2.2 **必需字段表（单源派生）**：从编译器的必需字段定义**单源派生**「section → 必需 key」并注入重驱 prompt；该定义当前非公开，**MUST 经既有结构契约门面重导出后消费**，**MUST NOT** 在 prompt 侧另抄一份、**MUST NOT** 复制其定义。验证：字段表与定义处逐项一致；定义处变更时 prompt 侧随之变化
- [ ] 2.3 **最小合法 source（单源复用）**：重驱 prompt MUST 复用既有「最小合法 source」单源实现并带可识别的标记，**MUST NOT** 复制第二份字符串。验证：prompt 中的最小合法 source 与单源实现逐字一致；该 source 仍可通过编译器结构解析
- [ ] 2.4 **首轮零触碰**：A2 只影响**重驱** prompt，**MUST NOT** 改动首轮 prompt 一个字节 ⇒ 首轮固定规范面 23,500 B 与上下文面 32,000 B 的现状余量保持。验证：0.3 的余量基线复测无变化；0.4 的两条固定面护栏断言仍绿
- [ ] 2.5 **requirement 映射**：本工作包落在 `REQ-WSC-32` 的场景「重驱 prompt 携带结构契约三件套」。验证：场景与工作包一一对账无遗漏

## 3. A3 · `[previous_source]` 回灌（REQ-WSC-32）

- [ ] 3.1 **回灌上一版 source**：重驱 prompt MUST 含 `[previous_source]` 段与**上一次交付 source 全文**（形态先例：仓库内 fresh 返修轮的 `[previous_source]` 段）。验证：prompt 中该段存在且承载上一版 source 全文
- [ ] 3.2 **取数源复用耐久窗口模型**：上一版 source 的引用与 hash MUST 取自 `bound-sc-generation-teaching` 的耐久教学窗口模型，**MUST NOT** 由重驱路径另行抓取或重算第二套事实。验证：重驱路径的取数调用面与窗口模型提供的面一致；同一 source 的 hash 在两处口径一致
- [ ] 3.3 **自足性**：provider 无原生 resume（resume 返回 `None`）时，重驱轮 MUST 仍能凭 prompt 自足完成修复。验证：无 resume 条件下重驱轮仍拿到上一版 source 与结构契约
- [ ] 3.4 **体量有界**：上一版 source 的回灌体量 MUST 落在既有上限口径内，**MUST NOT** 新开「动态段不计预算」类例外绕过任一面。验证：回灌后的重驱 prompt 体积在既有预算/上限内且有可断言的度量
- [ ] 3.5 **requirement 映射**：本工作包落在 `REQ-WSC-32` 的场景「上一版完整 source 随重驱回灌」。验证：场景与工作包一一对账无遗漏

## 4. A4 · 预算耗尽后的「SC 结构失败诊断包」（REQ-WSC-32）

- [ ] 4.1 **事件落点**：教学机会耗尽进入终态**前**，在 compile 失败与 IR 失败两处收口点各发一条显式的结构失败诊断包 execution event（走既有 execution event 通道，**MUST NOT** 新增持久化字段 / wire 字段 / env 开关）。验证：两处收口点各产生且仅产生一条该事件
- [ ] 4.2 **载荷完整**：事件输出 MUST 含**五段式诊断全集**（每条带缺失字段与修复示例）、**最后一次交付 source 的 hash 与字节数**、以及**最小合法 source 的对照引用**。验证：三条载荷逐项可断言；hash 与字节数取自耐久窗口模型口径（复用 3.2）
- [ ] 4.3 **终态句式逐字不动**：既有终态 message（既有「一次教学重驱后仍失败」表述）MUST 仍含该表述，**MUST NOT** 因新增诊断包而被改写。验证：既有终态 message 断言仍绿
- [ ] 4.4 **纯观测、不开新机会**：诊断包事件 MUST NOT 触发任何额外 provider 启动、**MUST NOT** 构成新的教学机会或重试入口。验证：两轮失败后实际启动次数不因该事件增加
- [ ] 4.5 **requirement 映射**：本工作包落在 `REQ-WSC-32` 的场景「机会耗尽后发出结构失败诊断包」。验证：场景与工作包一一对账无遗漏

## 5. 验收（REQ-WSC-32）

- [ ] 5.1 **反馈完备性（单元）**：在既有「教学重驱 prompt」测试文件上扩展 —— 构造一条五段式回灌理由，断言重驱 prompt 含**修复示例原文**、**必需 section 清单的首项与末项**、**最小合法 source 标记**、**`[previous_source]` 段头**；该文件**既有断言全部保留仍绿**。验证：新增断言与既有断言同批绿
- [ ] 5.2 **重驱后恢复（集成）**：在既有「重驱与重开」集成测试上扩展 —— mock 首轮返回缺必需 section 的交付、重驱轮返回补全后的交付 ⇒ 断言 run **走到候选产出**（而非失败终态）。验证：断言以「产出候选」为期望值，不以「未失败」这类弱断言代替
- [ ] 5.3 **降级产物可诊断（集成）**：mock 两轮都缺 section ⇒ 断言终态失败**前**发出结构失败诊断包事件，其输出含五段式诊断与末次 source 的 hash；既有终态 message 断言仍含「一次教学重驱后仍失败」。验证：事件条数、内容项、终态句式三项分别断言
- [ ] 5.4 **预算回归**：既有 prompt 契约测试中的 `work_item_plan_markdown_prompt_inlines_grammar_boundaries_and_real_findings` 与 `sc_author_prompt_tail_clamps_heading_language_and_raises_budget` MUST **原样通过** ⇒ 固定面零触碰。验证：两条原样通过且未修改断言内容
- [ ] 5.5 **不变量回归**：四码教学资格白名单、两个教学槽位的记账语义、重驱次数上限（不新增第三次）三条 MUST 逐字不变。验证：三条各有断言覆盖；本 change 的改动前后对账无差异
- [ ] 5.6 **requirement 映射**：5.1–5.5 覆盖 `REQ-WSC-32` 的全部七个场景。验证：场景与验收项一一对账无遗漏

## Workflow follow-up / 授权后

- 归档硬前置：`bound-sc-generation-teaching` 的耐久教学窗口模型就位前，本 change **不得归档**（A3/A4 的取数源不存在）。验证：归档前该前置已满足并登记
- 与 `mechanicalize-plan-compile-repair` 的去重复核：本 change 只**引用** `REQ-WSC-09` 的资格清单口径，不改其正文；四码白名单不变（流程项，不映射可验收产出）。
- 与 `bound-sc-generation-teaching` 的边界协调：本 change 只**消费**窗口模型承载的事实，不改其字段、状态机、账本与预算归属（流程项，不映射 `REQ-WSC-32` 的可验收产出）。
- 与 `close-contract-findings-before-pass` 的边界协调：本 change 不改闭合资格谓词与七面口径（流程项，不映射 `REQ-WSC-32` 的可验收产出）。
- 按项目规则执行受影响的定向测试与全量门禁（含格式、静态检查与**行数门禁**），并由集成 owner 统一执行而非各工作包并行执行（授权后执行，不映射可验收产出）。
- 真实 provider 的重驱收敛效果评估另行授权后执行；本文档的 tasks 不含任何真实 provider 调用，且**不得**把基线对照值写成收益结论（流程项，不映射 `REQ-WSC-32` 的可验收产出）。
