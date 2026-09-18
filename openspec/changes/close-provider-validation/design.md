# Design: close-provider-validation

## Context

**决议权威（不得漂移）**：`.superpowers/sdd/2026-09-04_计划文档_3.6全量收敛轮_v1.0/stage4-tripartite-decision.md`（Q3 kimi 验证轮/change ① 切分；oracle 裁决 `stage4-oracle-verdict.md`；ter 可行性=yield 摘要未落盘（要点以决议转述为准）；stage4-scout.md 已落盘，其引用已由 ter 逐条实读核验，本文事实基线同源）。本 change 是决议 change ① 的契约化落地，不重新论证、不引入未拍板项。

**现状技术事实基线**（起草时实读核验，行号为 worktree `feat-b-0808-add-monorepo` 实读）：

| # | 事实 | 证据 |
|---|---|---|
| 1 | **claude headless 修复已落地**（scope 接正，supersede 立项输入包「提案全就绪未实施」句）：`build_args` 无条件含 `--permission-prompt-tool=stdio`（`claude_code_provider/mod.rs:201`）；`permission_mode_for_claude` 将 Auto/Supervised 均→`"default"`（`:147-150`）；stream.rs 无 `write_tool_result`，`resolved_ask_user_questions` 缓存带 `is_error` 处理（`stream.rs:49,239-260`）。三提交 `3164f6a7`（提交 C）/`e367a84e`（提交 B）/`20493b65`（提交 A）均 08-21 落地、均在 HEAD 祖先，测试名与提案 tasks 1.x-3.x 逐项对应（`tests/args.rs:58`、`tests/ask_user_question.rs:312,403`、`tests/permissions.rs:89`） | 本 change 起草实读+`git log --all` |
| 2 | fix-claude-code change active 未归档：tasks 全未勾选，提案 4.1（全量门禁）/4.2（真实 CLI smoke 2.1.237）无留档证据；其 spec delta 五 requirement 均缺 SHALL/MUST（strict 校验 WARNING） | openspec/changes/fix-claude-code-ask-user-question-headless/ 实读 |
| 3 | kimi 落地面大于账面：tasks 仅 5.2/5.3 勾选；`render/kimi_code.rs` 已存在（与 pi/codex 并列）、`web/state.rs:477-478,513` 注册 KimiCodeProvider、`coding_models/provider_config.rs` 无 kimi Auto-only 强制、前端 8 个非测试文件含 kimi（provider.ts/provider-options.ts/ChatWorkspacePageParts.tsx/workspace-ws-message-handler.ts 等） | ter §0 核验表+本 change 起草实读 |
| 4 | kimi 验证轮从未实跑：`MIN_KIMI_VERSION="0.34.0"`（`kimi_code_provider/mod.rs:35`），ACP fixtures 冻结自 0.34.0（`tests/fixtures/*.jsonl`）；现行 CLI（本机实读 `kimi --version`=0.43.0，版本以 WP2.1 实跑记录为准）方言未核对（oracle：开放风险） | ter B-WP3、oracle Q3 理由 |
| 5 | kimi terminal flaky 家族：三测试均真实进程+计时预算——`head -c 1M|tr` 泵输出，`tokio::time::timeout(10-15s)` 包裹 wait+output（`client_services/terminal.rs:704,766,795`）；RR-3① 登记「计时预算类，隔离过滤跑也偶发」；concurrent 用例还要求 multi_thread 4 worker | terminal.rs 实读+defer-ledger RR-3 |
| 6 | cap 行为契约已在主 spec 钉死：`MAX_TERMINAL_OUTPUT_BYTES=1048576`（恰为上限完整返回，1048577 截断只标记一次）、stdout+stderr 合计、并发共享预算（`openspec/specs/kimi-acp-client-services/spec.md` 终端生命周期 requirement）——根治改验证方式不改行为语义 | 主 spec 实读 |
| 7 | add-pi 遗留 2.2/2.3 未实施（aria-ask.ts 结构化提问扩展/版本范围检测），pi 现状=文本暂停信号；3.6 先例=限制如实标注可归档 | add-pi tasks 实读+oracle Q3 理由 |
| 8 | campaign 基建在场：`cadence/reports/workitem-coding-campaign/`（workitem_run_campaign.mjs 单样本驱动器等，显式省略 `--dry-run` 才调真实 provider） | 目录实读 |

**本文只做规划产物**：精确文件、命令、测试与提交步骤由 `superpowers:writing-plans` 写入 `cadence/plans/`。

## Goals / Non-Goals

**Goals:**

- claude headless 修复闭环：提案吸收+已落地三提交补验（全量门禁+真实 CLI smoke 证据留档）（claude-code-structured-interaction 全部 requirement）。
- kimi coding 验证轮真实跑通并出结论：方言核对前置→coding 三角色真实 CLI→结论只 gate kimi 自身状态流转（REQ-PVR-01..04）。
- kimi terminal flaky 家族根治：三测试确定性锁定，真实进程 smoke 收敛为一条，RR-3① 销账（kimi-acp-client-services MODIFIED）。
- 两 active change 归档收尾：tasks 勾选与代码事实对齐、pi 2.2/2.3 显式 defer 登记、唯一 owner 确认后归档+delta 同步（REQ-PVR-05）。

**Non-Goals（与 proposal 非目标互补）:**

- 不重开 Q3/Q5 已决事项（kimi 结论 gate 边界、三 change 切分、①→③→② 落地序均按决议）。
- 不在验证轮中为 kimi 新增能力或改协议（暴露问题按既有契约修，修复不扩权）。
- flaky 根治不动 `MAX_TERMINALS`/`MAX_TERMINAL_OUTPUT_BYTES`/`TERMINAL_COMMAND_TIMEOUT_SECS` 常量语义与 grammar/沙箱约束。
- 归档不等同主 specs 大改：只同步本批 capability delta 与核账标注，不顺手改其他 spec。

## Decisions

### D1: claude 面形态 = 吸收提案 + 已落地行为的补验收尾（scope 接正）

**决策**：fix-claude-code-ask-user-question-headless 提案四件套并入本 change——其 spec 五 requirement 全量吸收进 `claude-code-structured-interaction`（补 SHALL/MUST 措辞过 strict）；工程面对已落地三提交做**补验**而非重实现：提案 4.1（`cargo fmt/clippy/test --workspace`）+4.2（真实 claude CLI 2.1.237 smoke，记录 tool_use→control_request→control_response→tool_result 事件序列）补跑并留档；旧 change 文件夹按「被吸收」处置（见 D5）。实跑若暴露真实回归，修复限定在提案原语义内（stdio 注册/模式映射/结果所有权三件事），不扩权。

**scope 接正声明**：立项输入包 v0.9 与 ter §1 面B 均表述「claude headless 修复（提案全就绪未实施）」；本 change 起草实读推翻该判断（事实基线 #1）。决议 item 1「claude headless AskUserQuestion 修复落地（吸收既有提案）」的交付物相应接正为「吸收+补验+归档收尾」——交付语义不变（修复在现行 build 上有新鲜验证证据），实施面从「实现」收缩为「验证」。此接正属决议 §scope 接正惯例（ter 已有先例：kimi tasks 勾选失真接正），呈报双审确认。

**替代方案与否决理由**：①按原提案重新走 TDD 三提交——重复实现已落地代码，浪费且引入回归风险；②仅归档旧 change 不吸收契约——提案的验证收尾（4.1/4.2）无 change 承载，账实失真延续。

### D2: kimi 验证轮执行流 = 方言核对前置 → 真实三角色 → 修暴露问题 → 结论落盘

**决策**：按 ter 依赖序 B-WP3→B-WP2：**方言核对先行**——现行 kimi CLI（版本以实跑 `kimi --version` 记录于核对矩阵为准）对冻结 0.34.0 ACP fixtures 的 wire shape 逐方法核对（initialize/session/new/session/prompt/session/request_permission/session/load），漂移则先更新 fixture+适配适配器，再进验证轮，避免在旧方言上验证返工。**验证轮主体**=真实 CLI 跑 coding 三角色（Coder/Code Reviewer/Internal Reviewer），与 claude headless smoke 合并共享 campaign 基建（`workitem-coding-campaign` 驱动器族，显式非 dry-run）。**修暴露问题**=实跑暴露的真实缺陷按既有契约修复（TDD）。**结论落盘**=证据矩阵留档+结论登记（D3）。

**验证范围口径**：coding 三角色为主面（决议原文「kimi coding 验证轮」）；普通 Workspace 角色（author/reviewer）与 image-create 已有 provider 级单测+前端回归覆盖，不在真实轮重复展开（如实记为「未在真实轮覆盖面」，不虚称）。

**替代方案与否决理由**：①跳过方言核对直接跑——方言漂移会使验证结论失真（oracle：kimi 当版方言是开放风险）；②全角色真实轮——决议只钉了 coding 验证轮，扩面违反范围铁律。

### D3: 结论 gate 边界与 kimi 状态流转二值（Q3 落地）

**决策**：验证轮结论**只 gate kimi 自身状态流转**：`待验证 → 转正`（真实轮通过：三角色到达真实终态、无阻断缺陷）or `待验证 → 受限登记`（实跑存在如实可登记的限制/缺陷——受限面+证据+限制说明登记入台账与 spec 标注，3.6 先例=限制如实标注可归档）。结论**不 gate** 退役（change ③）、多仓（change ②）或任何其他 change——REQ-WSC-07 判据原文仅 codex+pi，此口径写入本 change 全部产物与文案（REQ-PVR-03）。验证失败不挂起 provider 面攻坚（oracle 挂起问法 A=受限标记照收），本 change 内不因 kimi 结论阻塞其他 WP 收口。

**替代方案与否决理由**：①kimi 结论挂阶段 4 门禁——决议 Q3 明确否决（判据原文不含 kimi）；②验证失败即挂起攻坚——oracle 裁决「独立轮先行不挂阶段 4 门禁」+挂起问法 A 在案，受限登记是合法终态。

### D4: terminal flaky 根治 = 确定性断言迁移 + 单条真实进程 smoke（DEF-7 桶）

**决策**：cap 边界行为（恰为上限完整返回不截断、超限截断只标记一次、并发双流共享预算不超额、截断标记恰好一次）的 byte-exact 断言**迁离真实进程计时预算**：以受控内存流（如 `tokio::io::duplex` 定量喂入可控字节块）直接驱动 `read_terminal_stream` 的预算/截断逻辑，确定性锁定全部边界组合，无墙钟依赖；真实进程管道端到端行为收敛为**一条** smoke（保留 create→输出→wait→release 全链路与 cap 生效的最小断言，预算放宽到机器负载不敏感的口径）。三测试族原真实进程形态删除，不保留并行双份。RR-3 定性纪律全程适用（首败事实+定向复跑佐证+diff 无交集才可定性，不以复跑覆盖首败）。根治后 RR-3① 销账登记。

**cap 行为语义零变化**：上限常量、合计口径、截断标记语义、并发预算原子性（CAS reservation）全部不动——只改「行为如何被验证」。

**替代方案与否决理由**：①加大超时预算/重试——症状压制，负载敏感性不消除，违反 proposal 非目标；②标记 `#[ignore]`——cap 边界行为失去回归锁定；③全删真实进程测试——管道端到端（进程组/pipe/输出回收）失去唯一覆盖。

### D5: 归档收尾纪律 = 双向核账 + 显式 defer + 唯一 owner + 吸收处置

**决策**：
- **核账双向**：add-kimi tasks 逐项对照代码事实——「实做大于账面」补勾+证据（file:line 或提交号）；「勾而未落」如实改回并登记。核账结果留档后才归档。
- **pi 2.2/2.3 处置 = 显式 defer**：不实施；add-pi 归档时 tasks 2.2/2.3 保持未勾+显式登记（defer 台账：pi 结构化提问维持文本暂停信号现状、版本范围检测未做，限制如实标注）——与 3.6「限制如实标注可归档」先例一致。后续如有真实需求另行立项，不在本 change 预设。
- **唯一 owner**：三个 change 文件夹的归档动作（add-kimi/add-pi/fix-claude-code）在执行前确认无并行会话持有归档动作（决议风险登记），确认记录留档。
- **吸收处置**：fix-claude-code 文件夹不作为独立 change 归档 sync——其契约已并入本 change `claude-code-structured-interaction`（本 change 版本为唯一权威版本）；归档时按被吸收/superseded 处置，主 specs 以本 change delta 落地，避免同名 capability 双版本冲突。
- **归档时序**：kimi 状态标注（转正/受限登记）依赖 WP2 结论——归档收口在 WP2 结论落盘之后。

**替代方案与否决理由**：①顺手实施 pi 2.2/2.3——范围铁律（决议只钉「处置」不钉「实施」）；②三文件夹各自直接 `openspec archive`——claude-code-structured-interaction 会在主 specs 产生双版本（旧 delta 缺 SHALL 且本 change 已含超集），必须先处置吸收再归档。

### D6: 附录 B 决议承接对照表

见本文附录 B（Q3/Q5 逐句 → WP/REQ 映射，含 oracle 裁决要点与 ter 工时带承接）。与 3.8 惯例一致，作为双审（k3+oracle）对照决议的锚。

## Risks / Trade-offs

| # | 风险 / 取舍 | 缓解 |
|---|---|---|
| R1 | kimi 当版方言漂移面大→fixture 更新+适配工作量超预估 | D2 方言核对前置（宁可先核对不返工）；漂移结论如实登记，不静默适配 |
| R2 | 真实 provider 行为不可控（响应速度/偶发失败——阶段 1 flaky 教训） | 验证轮时长下限受 provider 制约（ter 工时带 8-16h 偏宽是刻意）；RR-3 定性纪律 |
| R3 | 核账疏漏（双向：漏补勾或漏揭「勾而未落」） | 核账以 ter §0 核验表+本 change 事实基线为起点，逐项 file:line 留档 |
| R4 | 归档动作与并行会话冲突 | D5 唯一 owner 确认前置，确认记录留档 |
| R5 | claude 补验暴露真实回归 | 修复限定提案原语义（D1 不扩权）；回归证据留档 |
| R6 | 同名 capability delta 双版本冲突 | D5 吸收处置钉死唯一权威版本 |
| R7 | flaky 根治后测试保护面缩水 | D4 保留一条真实进程 smoke+全部边界组合确定性断言（组合不减少） |
| R8 | kimi 受限登记被误读为「验证失败=change 失败」 | D3 口径写入 REQ-PVR-03/04：受限登记是合法终态，不阻塞本 change 收口 |

## Migration Plan

1. **交付序**：WP2 方言核对最前（防返工）∥ WP1（吸收+补验）∥ WP3（flaky 根治）→ WP2 验证轮主体 → WP4 归档收尾（WP2 结论落盘后）。每 WP 独立验收。
2. **无部署迁移、无数据迁移**：无 schema/wire 变更；验证证据与核账记录落 `cadence/` 与台账。
3. **回滚**：验证/测试改造性质，无生产行为变更；terminal 测试改造以普通提交回滚即可。
4. **归档不可逆动作**：唯一 owner 确认后执行；归档前 validate strict+全量门禁双绿。

## Open Questions

均为实施细化留白（不改架构、specs 或工作包划分）：

1. 确定性断言的内存流载体（duplex vs 手写 AsyncRead）——D4 定验收口径（全部边界组合+无计时预算），载体属 writing-plans。
2. 验证轮的样本规模与角色配比（每角色至少 1 案例到达真实终态为下限，上浮按实跑情况）——决议未钉死，实施时定并在证据矩阵留档。
3. 受限登记的具体标注载体（spec 标注措辞+台账条目形态）——按 WP2 结论定，D3 定二值语义。

## 附录 B 决议承接对照表

（决议=`stage4-tripartite-decision.md`；oracle=`stage4-oracle-verdict.md`；ter=yield 摘要未落盘（以决议转述为准）。逐句承接，双审对照锚。）

| 决议原文 | 承接位置 | 备注 |
|---|---|---|
| Q3：kimi 验证轮 → change ①内执行，不 gate change ②③ | REQ-PVR-03（gate 边界）；proposal 非目标第 2 条；tasks 全局边界 Q3 口径贯穿 | REQ-WSC-07 判据原文仅 codex+pi（主 spec 实读核对） |
| Q3：作为 provider 批第一个执行流（与 claude headless 修复验证合并共享 campaign 基建） | tasks 排序总则（WP2 置首执行流）；REQ-PVR-01（基建共享=`workitem-coding-campaign` 驱动器族）；WP1.3/WP2.2 共享 | campaign 基建实读在案（事实基线 #8） |
| Q3：结论只 gate kimi 自身状态流转（待验证→转正 or 受限登记） | REQ-PVR-04（二值流转）；D3；WP2.4 | oracle 挂起问法 A=受限标记照收，在案 |
| Q5 change ①：claude headless 修复落地（吸收既有提案） | WP1 全部；D1（scope 接正：三提交已落，交付物=吸收+补验+归档）；REQ-CCI 六 requirement（含新增 REQ-CCI-06 验证证据） | 接正声明呈报双审（事实基线 #1 推翻「未实施」判断） |
| Q5 change ①：kimi terminal flaky 根治（DEF-7 桶三测试） | WP3 全部；D4；kimi-acp-client-services MODIFIED 确定性验证条款 | 三测试名与行号实读在案（terminal.rs:704/766/795）；RR-3① 对应 |
| Q5 change ①：add-kimi/add-pi 两 active change 归档收尾 | WP4 全部；D5；REQ-PVR-05 | 核账基线=决议 §scope 接正「kimi tasks 勾选失真」句 |
| Q5 change ①：（pi 遗留 2.2/2.3 处置） | WP4.2 显式 defer 登记（不实施）；D5；REQ-PVR-05 场景「遗留项显式 defer」 | 处置=defer+如实标注（3.6 先例），非实施 |
| 决议风险登记：两 active change 归档动作唯一 owner 待确认 | WP4.3 owner 确认前置；REQ-PVR-05 场景「归档唯一 owner」；tasks 全局边界 | 确认记录留档 |
| 决议 §scope 接正：kimi tasks 勾选失真（实做面大于账面） | WP4.1 双向核账（补勾+证据，兼防「勾而未落」）；D5 | 起点=ter §0 核验表+本文事实基线 #3 |
| oracle Q3：kimi 当版方言是开放风险 | WP2.1 方言核对前置；REQ-PVR-02；D2 | ter B-WP3→B-WP2 依赖序承接 |
| ter 工时带 B=18-34h（B-WP1/2/3） | design Risks R2（工时带偏宽刻意）；WP1/WP2 对应 B-WP1/B-WP2+B-WP3 | flaky 根治（WP3）为决议新增项，ter 未单独定价，不虚编工时 |
| 决议下一步：①provider 批四件套→validate strict→双审（k3+oracle）→计划→实施 | 本四件套即交付物；WP4.4 validate strict 关闸 | 双审输入=本 change+附录 B |
