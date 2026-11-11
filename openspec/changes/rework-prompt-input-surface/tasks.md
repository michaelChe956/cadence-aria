# Tasks: rework-prompt-input-surface

> 纪律：每个工作包先固化/先红再改实现（TDD）；精确文件、命令、提交步骤由 `writing-plans` 写进 `cadence/plans/`，本文件只给高层可验收工作包。
> 契约来源：`specs/work-item-plan-single-candidate/spec.md`（**MODIFIED** `REQ-WSC-06`）；
> 架构边界、共享面串行次序、独占声明与不承载项见 `design.md`。
> **排序前提**：`design.md`「共享面与排序」的全局串行链（决策六纯拆分 → `mechanicalize` 的 prompt 瘦身与其 fixture/断言 → `isolate` 的 WIC ID 纪律 → `bound` → `effort` → `close` 的返修轮呈现口径 → **本 change，序 7 最后**）必须被遵守。
> **数字纪律**：本文件引用的实测数字（17 次串行工具往返、探索期 275.4 秒、run1 prompt 32,609 字符、author 侧 `cache_read` 仅 128、reviewer 侧 input 61,051 token、29 条写范围冲突中 27 条跨仓假阳性）**全部是基线对照值**，一律写成「实施后须复测」，**不得**写成已测结论。

## 0. 前置：锚定基线与排序确认（不改行为）

- [ ] 0.1 **基线锚定**：在**未改实现**的状态下记录并留档 ——（a）作者 prompt 实测字节与对 `WORK_ITEM_PLAN_MARKDOWN_PROMPT_QUALITY_BUDGET_BYTES`（23,500）的余量；（b）返修轮 prompt 实测字节（对照「逐字节复用首轮全文」的现状）；（c）多仓 `structure` 段实测长度与 `busi` / `frontend` / `gateway` / `scripts` / `policy` 在该段的出现次数（对照基线：段长 276 字符、5 个仓名出现 0 次、预算 3,200 B 只用 340 B）；（d）reviewer 投影各段实测字节与判断必需内容的在场情况。验证：基线逐项可复现同一数值并登记在案；**该数值只是对照，不得写成已达成结论**
- [ ] 0.2 **排序确认**：按 `design.md`「共享面与排序」，确认 `isolate-plan-criterion-namespace`（`work_item_split_engine/prompts.rs:185,:306`）、`mechanicalize-plan-compile-repair`（`sc_author_teaching.rs` 与 `prompt_contract*.rs`）、`close-contract-findings-before-pass`（`prompts/review.rs` 与 `sc_revision.rs:147`）的对应改动**已先落地**；本 change 的三处写入面在其后串行执行。验证：共享文件的相对次序与 design 表逐行一致，本 change 的提交不覆盖前序语义
- [ ] 0.3 **独占性复核**：确认本 change 的 spec delta **只 MODIFIED REQ-WSC-06 一条**，且无任何 active change 对同一 requirement 双 MODIFIED（`isolate` / `mechanicalize` / `close` / `bound` / `effort` 对 REQ-WSC-06 零命中）。验证：全目录 grep 复核，双改为零

## 1. G-4 · 成员仓摘要排序修正（REQ-WSC-06）

- [ ] 1.1 **成员仓优先排序**：`src/product/work_item_split_engine/context.rs:233-234` 的顶层条目排序改为**成员仓条目优先**，点文件 / 点目录条目在其后填充剩余名额；上限常量 `MEMBER_REALITY_MAX_TOP_DIRECTORIES`（12）**不变**。验证：多仓场景下成员仓摘要**不再出现「三仓全部落榜」**，且名额上限未上调
- [ ] 1.2 **单仓路径逐字不变**：`summarize_repository_structure` 的单仓分支（`entries.sort()` + `truncate(30)` + `(empty repository)` 文案）**一个字都不动**。验证：REQ-PLN-17 的「单仓摘要逐字不变」既有场景仍绿，单仓 prompt 逐字节输出与改造前一致
- [ ] 1.3 **分层与显式失败语义不变**：顶层 ≤12 / 每目录一级子项 ≤6 / 总行数 ≤40 的分层结构，以及 `repository_structure_unavailable: <原因>` / `repository_structure_empty: <路径>` 的显式失败语义**保持不变**。验证：REQ-PLN-17 的其余场景全绿（分层呈现、读取失败显式报错、空仓不冒伪内容）
- [ ] 1.4 **复测**：以 0.1 的基线对照 —— 实施后须复测 `structure` 段长度与 5 个仓名在该段的出现次数（基线：276 字符 / 出现 0 次）。验证：复测值已记录且与基线并列登记；**不得**把复测值写成设计文档里的既成事实
- [ ] 1.5 **requirement 映射**：本工作包落在 MODIFIED REQ-WSC-06 的场景「成员仓摘要不得被点条目挤掉」；不新增、不修改其它 requirement。验证：场景与工作包一一对账无遗漏

## 2. G-5 · author 上下文分级供给（REQ-WSC-06）

- [ ] 2.1 **L1 直供（成员仓摘要）**：把成员仓摘要作为**平台直供**段注入 author prompt，来源为 1.x 修好的成员仓摘要，不得再靠模型 Read/Glob 补齐。验证：author prompt 中该段的存在有可断言的证据；模型侧无需为「知道有哪些成员仓」发起工具调用
- [ ] 2.2 **L2 直供（依赖图事实）**：把已确认的依赖图事实（confirmed member targets / change_order 等）作为平台直供段注入，**与已确认来源同源**，**不得**在 prompt 侧独立重述口径。验证：直供内容与 canonical 侧同源可对账；不存在第二套「依赖图是什么」的解释
- [ ] 2.3 **L3 预算约束不变**：`budget_markdown_context`（`src/product/work_item_split_engine/prompts.rs:395`）改签名与调用点后，L3（story / design 等可再加载上下文）的既有预算分配口径**保持不变**；`WORK_ITEM_PLAN_MARKDOWN_CONTEXT_BUDGET_BYTES`（32,000）**不调**。验证：L3 各段的既有配额断言仍绿；新直供段不挤占 L3 的既有配额
- [ ] 2.4 **总预算不逃逸（口径已裁决澄清 · 2026-11-10）**：作者 prompt 的字节预算**分两面**，新增直供内容 MUST 计入**其所属面** —— **固定规范面**内容计入 `WORK_ITEM_PLAN_MARKDOWN_PROMPT_QUALITY_BUDGET_BYTES`（23,500），**上下文面**内容计入 `WORK_ITEM_PLAN_MARKDOWN_CONTEXT_BUDGET_BYTES`（32,000）。两面的归属在实现与断言中 MUST 显式可分（固定面以 fixture 度量、上下文面以 `[confirmed_context]` 段度量），**不得**借「动态段」名义规避任一面（决策 §7.1 集成约束 4）；**两个预算常量 SHALL NOT 上调**。验证：两面各自的字节断言均在既有预算内；常量值逐字未变（23,500 / 32,000）
- [ ] 2.5 **探索行为复测**：实施后须复测首 token 前的**串行工具往返次数**与**探索期墙钟**（基线对照：17 次串行工具往返 / 探索期 275.4 秒）。验证：复测方法与基线同口径、结果已记录；**不得**把复测结果写成已测结论或收益承诺
- [ ] 2.6 **requirement 映射**：本工作包落在 MODIFIED REQ-WSC-06 的场景「author 上下文分级直供」。验证：场景对账无遗漏

## 3. 返修 prompt 不复用首轮全文（REQ-WSC-06）

- [ ] 3.1 **停止逐字节复用**：`src/product/work_item_split_engine/prompts/sc_revision.rs:27` 的 `build_work_item_plan_markdown_revision_prompt` 不再以 `build_work_item_plan_markdown_prompt` 的首轮完整 prompt 为基底逐字节复制。验证：返修轮 prompt **不含**首轮完整 prompt 逐字节副本（可用逐字节比对断言）
- [ ] 3.2 **本轮 delta 完整注入**：本轮 reviewer findings 回灌、逐条硬性修复指令与 `[revision_directives]` **保持注入**，不因 prompt 瘦身而丢失或降级。验证：返修轮 prompt 中本轮 findings 逐条在场、指令段完整
- [ ] 3.3 **相关既有上下文可得**：返修轮 SHALL 携带上一版 source 与本轮 findings 的 `required_action` 所指向的 section 及其依赖上下文，使「除该行外逐字节保持不变」可执行。验证：每个 actionable finding 的目标 section 在返修轮上下文中可得（逐条可断言）
- [ ] 3.4 **产出仍可解析**：返修轮仍须输出**完整 markdown source**，其输出语法面不得随首轮全文一起去掉。验证：返修轮产出可被既有 parser 解析（回归用例绿）
- [ ] 3.5 **「与本轮相关」口径唯一**：确定并钉死「与本轮相关」的**唯一**判定口径（不得出现第二套解释），实现与断言使用同一口径。验证：口径写入实现注释与断言，同一输入的裁剪结果唯一
- [ ] 3.6 **字节复测**：实施后须复测返修轮 prompt 实测字节（对照基线：返修轮逐字节复用首轮完整 prompt，等于每轮重付 32KB 静态规范面；run1 prompt 实测 32,609 字符）。验证：复测值已记录并与基线并列；**不得**写成确定节省量
- [ ] 3.7 **缓存命中断言**：author 侧 `cache_read_tokens`（基线仅 128 ⇒ 首轮 prompt 在重驱时完整重发一次）作为**观测口径**登记；本工作包**只**登记观测方式，不承诺缓存命中数字。验证：观测口径写入验收记录
- [ ] 3.8 **requirement 映射**：本工作包落在 MODIFIED REQ-WSC-06 的场景「返修轮不重发首轮全文」与「返修轮保留本轮修复所需上下文」。验证：场景对账无遗漏

## 4. X2 · reviewer 投影裁剪（REQ-WSC-06）

- [ ] 4.1 **投影呈现收窄为四类**：`src/product/workspace_engine/prompts/review_parts/single_candidate_input.inc.rs` 的 reviewer 投影呈现收窄为 —— 能力覆盖缺口（gaps-only）、handoff 消费闭环、**同仓**写范围冲突、Projection Validation Report。验证：四类内容在 reviewer prompt 中在场；非缺口 edge 的重复信息不在投影里二次呈现
- [ ] 4.2 **同仓限定显式生效**：写范围冲突只保留同一 `target_repository_id` 内的 exclusive / forbidden 重叠；跨仓重叠不进入投影。验证：跨仓用例不入投影、同仓真实冲突用例**保留**（基线事实：29 条写范围冲突中 **27 条为跨仓假阳性**、2 条同仓为真实信号）；收窄理由在 spec 中有对应文字（**不得**以「信息仍可从 CCC 推导」代替理由）
- [ ] 4.3 **平台侧校验不受影响**：source / IR / 机械报告的绑定校验、同代结构事实与 hash 复算全部保留；不得因 prompt 不展示而删验证（决策 §7.1 集成约束 3）。验证：平台侧校验用例全绿，且与投影裁剪**解耦**（裁剪不改变校验路径）
- [ ] 4.4 **判断力不减**：reviewer 仍能看到能力缺口证据（`from → to`、`contract_id`、`missing_capabilities`）与 handoff 消费证据，gap edge 的 evidence 要求仍在 prompt 中逐条列出。验证：缺口证据行逐条在场断言绿
- [ ] 4.5 **只注单候选**：legacy / story / design reviewer 路径**不接收**该投影且行为不变（主规格既有场景「投影仅注单候选」）。验证：该场景仍绿，其它 reviewer 路径 prompt 逐字节不变
- [ ] 4.6 **共享计算复用**：投影数据仍复用 `src/product/work_item_contract/dependency.rs` 的确定性共享计算逻辑生成，**不得**以独立重述口径替代（主规格既有场景「投影复用共享计算」）。验证：该场景仍绿；同一输入两次生成的投影逐字节一致
- [ ] 4.7 **字节复测**：实施后须复测 review prompt 实测字节（对照基线：reviewer prompt 52,297 字节，投影两段合计 16,084 字符 / 18,362 字节）与 reviewer 侧 input token（基线 61,051 token，其中作者探索结论未以任何形式传递）。验证：复测值已记录并与基线并列；**不得**把「−33.9%」一类换算写成已达成结论
- [ ] 4.8 **requirement 映射**：本工作包落在 MODIFIED REQ-WSC-06 的场景「投影最小内容为缺口与消费闭环」「写范围冲突事实按同仓限定」「投影收窄不删平台侧校验」。验证：场景对账无遗漏

## 5. D5 事后 · 越界报错回灌基线树最近候选路径（REQ-WSC-06）

- [x] 5.1 **红测先行**：先固化「`acceptance_path_not_in_baseline` finding message 含基线树中同前缀最近现存路径」的可执行断言（**有同前缀候选**与**无同前缀候选**两分支各一条），确认红。验证：红测存在且失败原因为断言缺失（非编译错误）
- [x] 5.2 **落点唯一**：`src/product/work_item_plan_compiler/validate.rs` 生成该 finding 处追加最近候选路径；该处**已在 `baseline_tree` 参数作用域内**，MUST NOT 新增第二套基线装载。验证：代码中基线树来源唯一（`plan_baseline_tree` 同一函数）
- [x] 5.3 **候选不得臆造**：候选路径 MUST 取自同一棵基线树；无同前缀候选时如实说明「无可用候选」；MUST NOT 由平台构造任何不在树内的路径（臆造即制造新的越界引用）。验证：断言「输出中每个候选路径都 ∈ 基线树」
- [x] 5.4 **不动既有常量与判定**：`types.rs:121` 的 `ACCEPTANCE_PATH_NOT_IN_BASELINE_REPAIR_ACTION` 常量**逐字不变**；四类 typed IR 诊断的资格清单与机械化判定口径**不变**（`mechanicalize` 的 REQ-WSC-09 域）。验证：常量与资格清单逐字比对一致；`mechanicalize` 侧断言全绿
- [x] 5.5 **requirement 映射**：落在 MODIFIED REQ-WSC-06 的场景「越界路径回灌最近候选」。验证：场景对账无遗漏

## 6. D5 事前 · 基线树清单直供（REQ-WSC-06）

- [x] 6.1 **红测先行**：先固化「author prompt 含基线树可用路径清单段」与「基线树不可得（`Ok(None)`）时该段缺席且零回归」两条断言，确认红。验证：红测存在且失败原因为断言缺失
- [x] 6.2 **装载时点与来源**：在**作者轮启动前**（`src/web/workspace_ws_handler/run/single_candidate.rs` 的生产 prompt 构造点）装载 `plan_baseline_tree(lifecycle, project_id, issue_id)`；与 Evaluate 期（`single_candidate.rs:591`）**同函数同来源**。验证：作者轮前与 Evaluate 期取到的基线树**逐路径集合相同**（同源可对账）
- [x] 6.3 **注入位置与形态**：`WorkItemPlanMarkdownAuthorContext` 新增**可选**基线树字段；注入到 `[confirmed_context]` 的新增 `baseline:` 段，按成员仓分组、成员仓优先（与 D3 同序）。验证：prompt 中该段在场且分组正确；单仓场景下形态不畸变
- [x] 6.4 **失败语义**：基线树**装载失败 SHALL NOT 阻断作者轮**（记诊断，既有 fail-closed 点仍在 Evaluate 阶段）；无仓 issue（`Ok(None)`）⇒ 该段缺席、零回归。验证：注入不可得的用例下作者轮**仍可启动**并产出候选
- [x] 6.5 **上限与截断**：清单超上限时按成员仓保底截断（成员仓优先），MUST NOT 使上下文面超 `WORK_ITEM_PLAN_MARKDOWN_CONTEXT_BUDGET_BYTES`（32,000）；**固定规范面** `WORK_ITEM_PLAN_MARKDOWN_PROMPT_QUALITY_BUDGET_BYTES`（23,500）**不因本项变动**。验证：大仓 fixture 下上下文面字节仍在预算内、固定面字节不变；**预算常量未上调**
- [x] 6.6 **返修轮同源**：返修轮 prompt（`sc_revision.rs`）SHALL 携带**同一份**基线树清单（不得因返修轮不重发首轮全文而丢失该事实）。验证：返修轮 prompt 中该段在场
- [x] 6.7 **requirement 映射**：落在 MODIFIED REQ-WSC-06 的场景「基线树事实直供」「基线树清单不得占用固定规范面预算」。验证：场景对账无遗漏

## 7. D6 · 前轮 finding 状态：求交 + 去重 + 分区注入（REQ-WSC-06）

- [x] 7.1 **红测先行**：先固化四条断言 ——（a）本轮机械报告**仍存在**同内容身份 ⇒ 进待办区；（b）本轮机械报告**已无**该内容身份 ⇒ 进已关闭区且**不要求作者再改**；（c）前轮多条同内容身份 ⇒ 折叠为一条；（d）求交键为**内容身份**而非含 `message` 的指纹。确认红。验证：红测存在且失败原因为断言缺失
- [x] 7.2 **求交键**：以**内容身份**（前轮 finding 的 `contract_field` 与当前候选机械报告 finding 的 `code`）求交；MUST NOT 以 `FindingFingerprint` 为键（其派生含 `message`，机械报告 message 与评审输出 message 不同 ⇒ 按指纹求交恒不相交）。验证：断言「机械报告 message 与评审 message 不同、指纹不相交而内容身份相交」
- [x] 7.3 **数据源与同代**：求交数据源为**当前候选**的机械报告（`session.mechanical_report_ref`，Evaluate 阶段落盘）；与评审输入同代校验既有 `ir_adapter.rs` 的 `mechanical_report_ref` 一致性检查兜底。验证：引用不存在的 ref 时**如实降级**（不 panic、不静默当作「全部已解决」）
- [x] 7.4 **去重**：同一内容身份只保留一条，重复条目折叠为计数。验证：断言「4 条同 fingerprint 前轮 finding ⇒ 清单内 1 条 + 计数」
- [x] 7.5 **分区注入**：清单分「待办」（附 `required_action` 与 `contract_field`）与「已关闭」（附关闭依据）两区；**已关闭区 MUST NOT 要求作者再次修改**。验证：断言「已关闭区条目不含可执行修改要求」
- [x] 7.6 **纪律段改写**：复评 prompt 的重复判定纪律段改写为「以平台给出的分区事实为准」，SHALL NOT 由模型依据清单文本自行推断某前轮 finding 是否仍存在。验证：断言纪律段文本含该约束且不再要求模型自行判断
- [x] 7.7 **不动 close 语义**：`close-contract-findings-before-pass` 的有效必关闭判定、severity 呈现与 owner 排除口径**不变**。验证：close 侧断言全绿；本项只在清单**内容**上增补
- [x] 7.8 **requirement 映射**：落在 MODIFIED REQ-WSC-06 的场景「注入前轮结构化清单」「重复判定不靠记忆」「前轮 finding 状态求交按内容身份」「前轮 finding 分区注入」「前轮清单同身份去重」。验证：场景对账无遗漏

## 8. 验收与收口（REQ-WSC-06）

- [x] 8.1 **契约级验收**：MODIFIED REQ-WSC-06 的**每个场景**（主规格原有 28 个场景逐字保留 + 本 change 追加场景）各有可执行断言或可观察行为证据。验证：场景清单逐条对账，**主规格原有场景标题与正文逐字仍在**（脚本比对），追加场景全部有对应断言
- [x] 8.2 **描述段门禁**：MODIFIED REQ-WSC-06 描述段 ≤500 字符（归档进主规格后的硬门禁；change 级对 MODIFIED 豁免，归档后不豁免）。**当前增补后实测 705 字符 / 1,425 字节**（D5 事前 + D6 两项义务已并入描述段）⇒ 归档前**必须**压缩回 ≤500 字符（压缩时 MUST NOT 删除「基线树事实直供」与「前轮 finding 状态由平台机械复算」两项义务的 SHALL 强度）。验证：python 实测描述段长度 ≤500 并贴出输出
- [ ] 8.3 **归档模拟**：在 `/tmp` 副本中归档本 change 后，`openspec validate --specs --strict` 全绿（**79 passed, 0 failed**）。验证：真实命令输出贴入报告
- [x] 8.4 **change 级严格校验**：`openspec validate rework-prompt-input-surface --strict` **exit 0**。验证：真实命令输出贴入报告
- [x] 8.9 **同装置效果复测（三臂）**：实施后臂 8 run（12:00–15:30）+ 同时段基线臂 `db34f3f0`（17:33–）+ A/B 臂（`--no-default-features`，关 D5 事前）；**判决见** `cadence/analysis-docs/2026-11-10_分析报告_WIG基线树事实注入与前轮状态复算_复测_v1.0.md` §9.1。判决一：「未产出计划」失败为**环境性**（同时段基线臂同样失败）⇒ **D5 事前保留**；判决二：「重复报归零」缺同窗口阳性对照 ⇒ D6 因果效果**待重测**。实验脚手架（`sc-author-baseline-tree` 编译期 feature）**已拆除**，复现配方见 git `7b182388`
- [x] 8.10 **实验脚手架拆除**：判决「D5 事前保留」⇒ `Cargo.toml` 的 `[features]` 与生产侧 `#[cfg]` 门控、`prompts.rs` 的 `cfg_attr(allow(dead_code))` 全部移除（全仓零残留）；拆除后全量 `cargo test -p cadence-aria --lib` **6,270 passed / 0 failed**
- [x] 8.5 **边界确认**：本 change 的改动**限于**：`openspec/changes/rework-prompt-input-surface/**`、D5/D6 涉及的六个实现文件及其**纯编译性**构造点补齐（`baseline_tree: None`）；**未**改动 `openspec/specs/**`（主规格）、其它 change 目录、裁决 / 决策 / 证据文档；G-2 / G-3 / G-6 仍由 `explicit-sc-author-effort` 的 SC author 出口 env 构造机制承载，本 change 只登记未夹带。验证：`git diff --stat` 逐项可对账；design.md 的「不承载什么」逐项有登记
- [x] 8.6 **变更收口**：本 change 的 spec delta 与其它 active change **无同一 requirement 被双 MODIFIED**（本 change 只 MODIFIED REQ-WSC-06）。验证：全目录 grep 复核，双改为零
- [ ] 8.7 **【防漏检 · 新增】投影裁剪后仍能拦住「实测那 4 类破坏」** —— 4 组的 X2 投影裁剪把 reviewer 看到的契约覆盖投影**压缩**了（gaps-only + handoff 消费闭环 + **同仓**写范围冲突 + PVR），方向是**减体积**；而实测基线里 reviewer 已经**漏掉了 4 类真实破坏**（CT 编号重排 / WI-004 丢一个 input_contract / TASK 序号后移 / Traceability 三元组删除，**全部无人拦截**）。若裁剪后的投影**再少带**事实，漏检只会更重 —— 那是「改完更没法用」。故须在**裁剪前**把上述 4 类破坏各固化为一条**可复现的 review 输入 fixture**，裁剪后断言：**这 4 类仍 MUST 被 reviewer 判为 must_fix**（不得因投影瘦身而降级为 advisory 或漏过）。验证：4 条 fixture 在**未裁剪**的投影下即已被拦（证明基线可复现）→ 裁剪后逐条仍被拦；任一条降级或漏过即判为回归，裁剪须回退该字段。映射 requirement：MODIFIED REQ-WSC-06「reviewer 拦截 canonical 口径的能力缺口与未消费 handoff」
- [ ] 8.8 **【防副作用 · 新增】「裁剪后 reviewer 探索次数不增加」的观测量** —— 裁剪本意是让 reviewer 不必重复探索（实测 reviewer 自己探了 **13 次** shell，把 author 探过的仓库又探了一遍）；但若投影瘦身导致**关键事实缺失**，reviewer 反而会**探更多**次。须记录裁剪前后的 reviewer 工具调用次数与探索墙钟作为**观测量**（不作判据），并在结论中显式标注该量级
- [ ] 8.11 **【描述段门禁 · 增量义务】归档前压缩 MUST NOT 削弱本批追加的两项义务** —— 描述段 ≤500 字符的压缩（8.2）在删减时**除**保留「基线树事实直供」「前轮 finding 状态由平台机械复算」两项义务的 SHALL 强度外，**还 MUST NOT** 使「写范围越界诊断精度」「逐仓实况事实直供」两项义务失去 SHALL 强度：两项义务以**场景**形式承载（场景是 requirement 的规范组成部分），压缩 MUST NOT 删除这两个场景。验证：归档前的描述段实测 ≤500 字符，且四个义务在 requirement 中均可逐条定位

- [ ] 8.12 **【新脚手架拆除 · 新增】第二轮 D5/D6 三臂测量脚手架的拆除** —— 8.9 / 8.10 拆除的是**第一轮** `sc-author-baseline-tree` 编译期 feature（提交 `8db41c22`）；其后为做 D5/D6 三臂测量**新加**了三个 feature（提交 `32c763df`），当前仍**全量残留**：`Cargo.toml` 的 `[features]` 段（`default = ["wig-d5-before", "wig-d5-after", "wig-d6"]` 与三条 feature 声明）+ 生产侧 4 处门控（`work_item_plan_compiler/validate.rs:83` 与 `:95`、`work_item_split_engine/prompts.rs:301`、`workspace_engine/prompts/review_context.rs:573`、`web/workspace_ws_handler/run/single_candidate.rs:993`）。⇒ 该脚手架**只服务测量**，MUST 在归档前移除，且 8.10 的「全仓零残留」结论 MUST 重新成立。验证：① `grep -rn "wig-d5-before\|wig-d5-after\|wig-d6" Cargo.toml src/` **零命中**；② `Cargo.toml` 的 `[features]` 段整体移除；③ 4 处门控恢复为**默认全开**的等价直写（不保留 `cfg!` 分支、不保留 `cfg_attr(allow(dead_code))`）；④ 拆除后全量 `cargo test -p cadence-aria --lib` 通过且计数登记。⚠️ **本项 MUST NOT 与 8.10 重复计数**：8.10 是第一轮脚手架的拆除（已勾选），本项是第二轮脚手架的拆除（当前**未完成**）。映射 requirement：MODIFIED REQ-WSC-06（基线树事实直供 / 前轮 finding 状态）—— 脚手架本身不承载契约，只承载测量。

## 9. 包③-b · 写范围越界的诊断与教学精度（REQ-WSC-06）

- [ ] 9.1 **红测先行（绝对路径样本）**：在 `src/product/work_item_plan_compiler/tests/write_scope_bound.rs` 固化实际 `/tmp/notice-delete-frontend-check/**` 样本的可执行断言：① 恰**一条**指向目标 WI 的 finding；② 码仍为 `write_scope_outside_bound_repository`、severity 仍为 Error；③ message **保留原始路径**且明确「绝对路径 / 不属于授权成员」；④ message **不出现** `busi//tmp` 一类「加成员前缀冒充仓内路径」的自动前缀建议。验证：红测存在且失败原因为断言缺失（非编译错误）
- [ ] 9.2 **判定不放宽**：绝对路径样本**仍被拒**、仍为同码 Error；`item_finding` 的归属判定、`path_within_prefix` 前缀口径、`..` 段 fail-closed、稳定码与 `WRITE_SCOPE_OUTSIDE_BOUND_REPOSITORY_REPAIR_ACTION` 引用**逐字不变**。验证：既有断言（单仓 / Manual 前缀表缺席零回归、被授权仓不在前缀表内 fail-closed、`..` 段 fail-closed、跨成员点名、成员内新建路径放行、顿号方言）全绿；**不得**出现「`/tmp/**` 放行」或「新增例外分支」
- [ ] 9.3 **仓内相对写法的指导保留**：`src/lib.rs` 一类仓内相对写法**仍**给「改写为带成员目录前缀的聚合根相对写法」指导（既有断言 `repo_relative_scope_is_blocked_with_rewrite_guidance_not_misclassified` 依赖它）。验证：该断言仍绿；message 含成员目录前缀示例且**不含**绝对路径式的拼接产物
- [ ] 9.4 **教学段替换（不追加）**：`src/product/work_item_split_engine/prompts/sc_author_teaching.rs:281` 的「写范围路径基准」段**整段替换**为候选文本 —— **保留**既有三条纪律的语义要点（成员目录前缀的聚合根相对形式 / 裸仓内相对写法 plan 期拒收 / 聚合根本地文件区不可写），并**增补**：① `/tmp/check/**` 一类绝对路径反例；② 明示「不能直接加成员目录前缀冒充仓内路径」；③ exclusive_scopes 必须属于本 Work Item 的授权成员；④ 明示「被授权成员内新建路径」与「AC 基线存在性」是两项独立规则。验证：既有教学语义要点逐条仍在（**逐条对账**）；新增四项逐条在场；**不得**把该段改成追加式堆叠
- [ ] 9.5 **多仓夹具断言的落位**：多仓场景的教学断言放 `src/product/work_item_split_engine/tests/prompt_contract_sc_revision.rs`（实测 703 行）；**不得**继续向已 **1,630** 行的 `src/product/work_item_split_engine/tests/prompt_contract.rs` 堆测试。验证：新断言只落在 `prompt_contract_sc_revision.rs`；`prompt_contract.rs` 行数不因本工作包增长
- [ ] 9.6 **预算复核**：固定规范面仍 ≤ `WORK_ITEM_PLAN_MARKDOWN_PROMPT_QUALITY_BUDGET_BYTES`（23,500）；legacy 单仓（`member_targets=None` ⇒ 该段整段缺席）fixture **逐字不变**（对照基线：实测 23,448 / 余 52）；教学段字节为**净减**（对照基线：原段 563 B，候选段 516 B，净减 47 B —— **候选文本未逐字给出，实施后必须实测复测**，不得写成已达成结论）；**不得**新增「动态教学不计预算」的例外。验证：两个预算常量逐字未变；实测字节已记录并与基线并列
- [ ] 9.7 **requirement 映射**：本工作包落在 MODIFIED REQ-WSC-06 的场景「写范围越界诊断区分绝对路径与仓内相对写法」「写范围教学反例精度」。验证：场景对账无遗漏

## 10. 包⑤ · 成员清单 / 逐仓实况注入（REQ-WSC-06）

- [ ] 10.1 **红测先行**：固化两条断言 ——（a）SC author prompt 侧出现 `## 逐仓实况` 段，且成员 alias、聚合根相对 path、职责、关键目录、入口、构建测试命令、基线**逐项在场**；（b）渲染结果超预算 / 装载失败时，作者轮**不失败**、降级为「逐仓实况缺席」。验证：红测存在且失败原因为断言缺失（非编译错误）
- [ ] 10.2 **事实来源唯一**：复用 `logical_codebase/planning_context_set.rs` 的 `render_compact_inventory`（`:195`）与 `render_member_reality_section`（`:317`）及 `PlanningContextSetResolver::resolve`（`:52`）；**MUST NOT** 新造第二套 prompt 侧成员发现器，**MUST NOT** 改渲染器语义或 `PLANNING_MATERIAL_MAX_BYTES`。验证：代码中成员清单只有一处渲染来源；REQ-PLN-12 / REQ-PLN-14 域的既有断言全绿
- [ ] 10.3 **渲染顺序与目标成员**：目标成员在前（保持本计划已授权成员 `target_member_ids` 顺序），其余按 alias 序；与基线树 / 成员顺序口径**保持一致**，不得出现两段顺序打架。验证：多成员夹具下逐仓实况段的成员顺序与授权成员顺序对账一致
- [ ] 10.4 **全量不裁剪与缺席如实说明**：接入侧 **MUST NOT** 裁剪成员、**MUST NOT** 输出 `omitted_member_count` 字样；**MUST NOT** 静默丢成员。⚠️ **以源码为准**：缺 checkout 的成员**仍出现**在清单与逐仓实况段（其关键目录 / 入口 / 构建测试命令显「未识别」、基线显「基线未识别(该成员无 checkout 记录)」），其清单行 `path` 列退化为**成员 alias**（非真实聚合根相对路径）⇒ 接入侧 **MUST NOT** 把该行 `path` 当作真实路径解释，且 MUST 把「逐仓实况成员集合」与「已授权有效成员」的口径差**如实解释**。验证：缺 checkout 夹具下断言「该成员在场且显未识别」「其 path 列为 alias」「无静默丢弃」；断言中不出现 `omitted_member_count`
- [ ] 10.5 **失败路径**：`render_compact_inventory` 返回 `Err`（`planning_material_over_budget`）或装载失败时，作者轮**不失败**、**不 panic**、记诊断并令该段**整段缺席**（与 D5「额度为 0 ⇒ 整段缺席」同一口径，不留空标记）。验证：超限夹具下作者轮仍可启动并产出候选；无 panic
- [ ] 10.6 **预算分别计量**：本段落在**上下文面**（`WORK_ITEM_PLAN_MARKDOWN_CONTEXT_BUDGET_BYTES` = 32,000），与 D5 基线清单段**分别计量**（本段不占用 D5 的 32,000/4 额度，但两者之和 MUST ≤ 32,000）；额度不足 ⇒ **整段缺席 + 记诊断**，**MUST NOT** 裁剪成员、**MUST NOT** 留半截成员块。固定规范面 23,500 B **零新增**；**两个预算常量 SHALL NOT 上调**。验证：上下文面字节断言 ≤ 32,000、固定面字节不变；常量值逐字未变（23,500 / 32,000）
- [ ] 10.7 **成本实测**：实施后**必须实测**成员数与本段字节并计入上下文面；**MUST NOT** 引用主方案的自造估算值（「3 成员量级 ~1–2 KB」）。验证：实测值已记录并与基线并列；估算值未被写进任何断言或结论
- [ ] 10.8 **边界（不越权）**：逐仓实况只给「有哪些仓 / 关键目录在哪」，**不含文件清单** ⇒ **MUST NOT** 被当作多成员基线树的替代，跨仓误拒不因此消除（基线树修复属 `plan-baseline-multi-member`，REQ-WSC-02 域）；本 change 不改动基线树判定面。验证：基线树相关断言与既有 change 侧断言全绿；`plan-baseline-multi-member` 的边界在 design 中已登记
- [ ] 10.9 **与 G-4 的断言口径不重叠**：G-4 修 `structure:` 段的**名额分配**（成员仓优先、点条目让位），本工作包只针对**逐仓实况段**；`structure:` 段的断言按 **G-4 落地后**的状态写，**MUST NOT** 与 G-4 抢同一断言口径（同一断言不得被两处改写）。验证：两处断言互不覆盖；对账表登记归属
- [ ] 10.10 **requirement 映射**：本工作包落在 MODIFIED REQ-WSC-06 的场景「逐仓实况事实直供」「逐仓实况成员缺席如实说明」。验证：场景对账无遗漏
