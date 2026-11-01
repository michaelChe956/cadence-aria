# Tasks: close-provider-validation

**工作包性质**：本文件只登记**高层工作包**与验收口径（映射 specs 的 requirement 与 design.md 的 D 系列决策）。精确文件、命令、测试与提交步骤由 `superpowers:writing-plans` 展开到 `cadence/plans/`；Plan 只能展开工作包，不能重定义契约。

**主契约**：三方决议 `stage4-tripartite-decision.md`（Q3/Q5，决议承接对照表 = design.md 附录 B）+ 本 change 四件套。

**全局边界（每工作包均继承）**：

- **范围铁律**：仅决议四项（claude headless 修复收尾/kimi 验证轮/kimi terminal flaky 根治/两 active change 归档收尾），不加不减；多仓、退役、DEF-4、coding_run_campaign 未测区核均属 change ②③，不越界。
- **Q3 口径贯穿**：kimi 验证轮结论只 gate kimi 自身状态流转，不作为退役/多仓或其他任何 change 的门禁——全部产物与文案同口径（REQ-PVR-03）。
- **claude 面零行为变更**：补验性质；实跑暴露真实回归才修，修复限定提案原语义（D1 不扩权）。
- **RR-3 定性纪律**：flaky 定性须首败事实+定向复跑佐证+diff 无交集，不得以复跑覆盖首败事实。
- **归档前置**：唯一 owner 确认（无并行会话持有归档动作）前不执行任何归档动作（决议风险登记）。
- 真实轮覆盖面如实分栏：未覆盖面不以替身证据冒充（REQ-PVR-01）。

**排序总则**（ter 依赖序 B-WP3→B-WP2 + D 系列）：WP2.1 方言核对最前（防返工）∥ WP1 ∥ WP3 → WP2 验证轮主体 → WP4 归档收尾（WP2 结论落盘后收口）。

## 1. WP1 claude headless 修复验证收尾（吸收既有提案）

- [x] 1.1 核账留档：fix-claude-code 提案 tasks 1.1-3.4 逐项对照已落地三提交（`3164f6a7`/`e367a84e`/`20493b65`）与现行代码（`mod.rs:201` 无条件 stdio、`:147-150` 双模式映射 default、stream.rs 无 write_tool_result+缓存 is_error 处理），确认「实做=提案 tasks 1.x-3.x 全部」；核账表（task→提交/行号证据）留档（REQ-CCI 全部 requirement 的既成事实基线）——验证：核账表逐项有证据锚；无「提案有而代码无」缺口（有则登记并按提案语义补）
- [x] 1.2 全量门禁补验留档（吸收提案 4.1）：`cargo fmt --check`、`cargo clippy --all-targets --all-features --locked -- -D warnings`、`cargo test --locked`（或现行门禁等价命令）结果留档（REQ-CCI-06）——验证：三命令结果留档；claude provider 测试族（args/permissions/ask_user_question）全绿
- [x] 1.3 真实 claude CLI smoke 补验（吸收提案 4.2）：真实 claude code CLI（2.1.237+）headless 触发 AskUserQuestion，记录 tool_use→control_request→control_response→tool_result 完整事件序列作为新鲜证据（REQ-CCI-06、REQ-PVR-01 基建共享）——验证：事件序列证据留档；Auto 模式下 AskUserQuestion 到达 ChoiceRequest 且等待用户（发送 ChoiceResponse 前无 Completed）
- [x] 1.4 吸收处置：fix-claude-code-ask-user-question-headless 文件夹标记被本 change 吸收——**本 WP 即在旧 change 的 proposal.md 与 tasks.md 头部写入「已被 close-provider-validation 吸收，契约唯一权威版本=该 change delta，禁止单独 archive」标记（不动文件夹、不走 archive）**；归档动作与 owner 确认并入 WP4.3 统一执行（REQ-PVR-05）——验证：旧 change 两文件头部含吸收标记；本 WP 不单独移动旧文件夹；归档时主 specs 无双版本（WP4.4 复核）
- [x] 1.5 WP1 关闸——验证：1.1-1.3 证据齐备；claude 面零代码改动（或有回归修复则修复限定提案语义+测试先行）；Q3 口径检查（文案无越界 gate 表述）

## 2. WP2 kimi coding 验证轮（provider 批第一个执行流）

- [x] 2.1 ACP 方言核对前置：现行 kimi CLI（版本以实跑 `kimi --version` 记录为准）对冻结 0.34.0 fixtures（`src/cross_cutting/kimi_code_provider/tests/fixtures/*.jsonl`）逐方法核对 wire shape（initialize/session/new/session/prompt/session/request_permission/session/load）；漂移→fixture 更新+适配器适配+回归先行，核对记录留档（REQ-PVR-02、D2）——验证：核对矩阵留档（方法×一致/漂移×处置）；验证轮开始前无未处置漂移
- [x] 2.2 真实 CLI 验证轮主体：真实 kimi CLI 跑 coding 三角色（Coder/Code Reviewer/Internal Reviewer，每角色 ≥1 案例到达真实终态），与 WP1.3 claude smoke 共享 campaign 基建（`workitem-coding-campaign` 驱动器族，显式非 dry-run）（REQ-PVR-01）——验证：三角色真实终态证据留档；全程无 fake/fixture 替身参与结论路径；覆盖面/未覆盖面分栏如实记录
- [x] 2.3 修暴露问题：实跑暴露的真实缺陷以失败测试先行修复并回归（修复不超出既有契约语义）（REQ-PVR-01）——验证：每个修复有失败测试→通过记录；无越权扩面改动
- [x] 2.4 结论落盘：按证据矩阵出 kimi 状态流转结论——转正 or 受限登记（受限面+证据+限制说明登记台账并反映到 spec/目录标注）（REQ-PVR-03/04、D3）——验证：结论与证据矩阵一致；受限登记不被表述为「验证失败挂起」；Q3 口径终检（结论未 gate 其他 change）
- [x] 2.5 WP2 关闸——验证：2.1-2.4 证据齐备；验证轮时长与异常如实记录（provider 不可控面如实登记）

## 3. WP3 kimi terminal flaky 根治（DEF-7 桶）

- [x] 3.1 确定性断言迁移：cap 边界行为（恰为上限完整返回不截断/超限截断只标记一次/并发双流共享预算不超额/截断标记恰一次）以受控内存流直接驱动输出预算逻辑，全部边界组合确定性锁定、无墙钟预算依赖（REQ 终端生命周期确定性验证条款、D4）——验证：改造后测试无 `tokio::time::timeout` 外层墙钟预算依赖断言；边界组合覆盖不减少
- [x] 3.2 真实进程 smoke 收敛：三测试族的真实进程形态收敛为一条端到端 smoke（create→输出流→wait_for_exit→release+cap 生效最小断言），预算口径机器负载不敏感；其余真实进程用例不重复保留双份（同上）——验证：terminal.rs 测试清单=确定性边界族+一条 smoke+既有非 cap 用例；无并行双份
- [x] 3.3 行为语义零变化回归：上限常量/合计口径/截断标记语义/并发预算原子性/幂等清理全部不变；全量回归绿（同上）——验证：改造前后行为断言等价（无断言放宽/用例删除）；`cargo test` terminal 族全绿
- [x] 3.4 RR-3① 销账：flaky 根治后多次隔离过滤跑+全量跑零触发记录留档，RR-3①（kimi terminal 部分）登记销账（RR-3 定性纪律全程适用）——验证：多次复跑记录留档；台账登记完成
- [x] 3.5 WP3 关闸——验证：3.1-3.4 证据齐备；无「放宽断言/加大超时/ignore」类症状压制手段混入

## 4. WP4 两 active change 归档收尾

- [x] 4.1 add-kimi 双向核账：tasks 逐项对照代码事实——实做大于账面（1.1 大部/3.1/4.1/4.2/render 已落等）补勾+证据（file:line 或提交号）；勾而未落项如实改回并登记；核账记录留档（REQ-PVR-05、D5）——验证：核账表双向完备（无沉默项）；以 ter §0 核验表+design 事实基线 #3 为起点
- [x] 4.2 add-pi 核账+遗留 defer 登记：2.2（aria-ask.ts 结构化提问扩展）/2.3（版本范围检测）显式 defer 登记（不实施、保持未勾、限制如实标注：pi 结构化提问维持文本暂停信号现状）；其余已落项核账补证（REQ-PVR-05、D5）——验证：defer 登记入台账；无静默勾选
- [x] 4.3 唯一 owner 确认+归档执行：确认无并行会话持有归档动作（确认记录留档）后，add-kimi-code-provider/add-pi-provider 归档+fix-claude-code 被吸收处置（`claude-code-structured-interaction` 以本 change delta 为唯一权威版本 sync 主 specs）；kimi spec/目录标注按 WP2.4 结论落状态（REQ-PVR-05、D5）——验证：三文件夹处置完成；主 specs 无同名 capability 双版本；owner 确认记录在案
- [x] 4.4 关闸：`openspec validate close-provider-validation --strict` 过+归档后主 specs 校验过+全量门禁双绿（fmt/clippy/test）——验证：validate 输出留档；主 specs 同步后无冲突；全量门禁结果留档
