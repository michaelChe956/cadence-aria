# defer 账 —— workitem-conversational-gate-advance（阶段 3 收官）

结构化源：[defer.manifest.json](defer.manifest.json)（构建器逐条校验 owner/裁决引用非空）。所有 `deferred` 条目为范围声明（「不在阶段 3 范围」），不是缺陷关闭；全部 `residual_risks` 条目为如实登记的残留观察项。

## Residual risks（残留风险，owner 在案）

| id | 内容 | owner / 裁决 |
|---|---|---|
| RR-1 | 真实 Confirmed campaign 缺失：8 次授权真实跑（codex×4/pi×4）0 次到达人工门；REQ-CG/WSC-interactive 的真实 provider 完成证据缺位（fake/确定性证据完整，真实分栏如实记缺） | 专项测量轮（controller 已排期，档期 1：本 change 收官后独立任务，届时重新授权，小规模预试验先行）；用户拍板 2026-09-02=A+档期 1；task-8.2-report.md §③ |
| RR-2 | oracle 裁决 B（原文照抄）：已知语义边界：在当前 `validate_dependency_contract_graph` 的保守、fail-closed 约束下，现行合法 2-WI 真实 `prepare_amendment` 链路实测仅可产生 `AwaitHandoff`；`Reexecute`/`Revalidate` 的恢复机制及三模式映射已由 7.2 `group_amendment_approve_updates_binding_and_resumes_target` 锁定，因此不将两模式在本链路中的真实可达性列为本 change 的验收失败项。该边界不阻塞收官，由专项测量轮 owner 继续观察；若未来出现多级依赖链或外部修订场景，再单独评估 impact 判定与图校验关系 | 专项测量轮 owner 继续观察；oracle 裁决 B（gpt-5.6-sol，与 controller 无分歧，B 共识）；引用锚 spec.md:49-66、design.md:52-54（D7）、design.md:104-107（D16）；禁写「三模式已在真实链全部验证」 |
| RR-3 | flaky 家族残留（定性纪律：须首败事实+定向复跑佐证+diff 无交集才可定性既有，不得以复跑覆盖首败事实）：①kimi terminal×2（`cross_cutting::kimi_code_provider::client_services::terminal::tests` 计时预算类，隔离过滤跑也偶发）②SC recovery 时序×2（stage3 并发时序互扰，8.3 fix round 1 已修根因=进程级全局 failpoint 注册表互扰，纯净树先例在案）③generate_endpoints（it_web `web_lifecycle_api/part_01`）。本验收轮（8.5）四命令全绿零触发 | 后续全量门禁执行者（controller/后续 change implementer）；progress.md Task 8.3/8.4 亲验记录 |
| RR-4 | 8.2 真实跑证据工件未入库（ws transcript/result 原始文件含敏感面，仅脱敏结论记台账与 [acceptance-report.json](acceptance-report.json) real_provider_runs） | 专项测量轮补齐脱敏工件（8.2 observation O1） |
| RR-5 | 7.1 上轮（5.x 修复轮前）2 Medium+5 Minor 明细未录台账，无法逐字核销；复审以 diff 通读无遗留证据结案 | controller（终审如需补录由 controller 说明此限制）；progress.md Task 7.1 note |
| RR-6 | 8.4 worker 报告⑦.2 定性：四窗口崩溃矩阵「各一 fixture」为计划层 fixture 形态要求（task-8.4-brief:12），与 D7 图 fail-closed 冲突，经用户拍板 A2 收敛为修订行 checkpoint 矩阵+四行合一骨架；不构成验收缺口 | 专项测量轮 owner（与 RR-2 同域观察）；用户拍板 2026-09-03=A2 + oracle 裁决 B |

## Deferred（范围外，非缺陷关闭）

| id | 内容 | 范围 | owner |
|---|---|---|---|
| DEF-1 | 前端 UI（对话式人工门/advance 的 UI 呈现，D12 Non-Goals） | 不在阶段 3 范围 | 后续 UI change（派工前先读 `.agents/skills/ui-ux-pro-max/SKILL.md` 规划） |
| DEF-2 | 真实 provider Confirmed campaign（真实链完成证据） | 不在阶段 3 收官范围（与 RR-1 对应：如实记缺口，不伪造不冒充） | 专项测量轮（重新授权+小规模预试验→修正→正式跑→单独测量报告） |
| DEF-3 | SC 子 session 只读呈现形态（阶段 3 只锁 typed transcript/事件面） | 不在阶段 3 范围 | 后续呈现层 change |
| DEF-4 | 未来 `auto_start_coding` 显式语义（advance 到 Ready 即止，不自动启动 coding provider） | 不在阶段 3 范围 | 后续 change |
| DEF-5 | 多仓 coding（group coding 多仓库执行） | 不在阶段 3 范围 | 后续 change |
| DEF-6 | 旧协议退役门（`HumanConfirmDecision` 等旧枚举在 REQ-WSC-07 退役门满足前不删） | 不在阶段 3 范围（legacy 零改动红线） | 退役门 change |
| DEF-7 | 各任务复审 minor/info/observation 账（已过审不阻塞，终审逐项裁量）：1.2 residual（行为级区分 fixture 已由 2.2 兑现）/2.2 m1-m4/2.3 minor（活会话 stub 死胡同已由 3.2 消除）/3.3 M4 负向组合等/4.2 LOW-1..4/4.3 low×5（TOCTOU/双写窗口等）/5.x minor/6.1 Minor×5/6.2 info×2/6.3 minor（spy 埋点面）/7.1 R2-m1..m3+info×2/7.2 Minor-1（注释失实）+info×4/8.2 O1-O3/8.3 Minor-1..3+info/8.4 ⑦.2 | 终审裁量项；均非验收缺口（各自 review pass 在案） | controller 终审 |
| DEF-8 | 7.2 Medium-1（`probe_amendment_gate_context` 无 context 分支错误码）已由用户拍板提前修复核销（`07e54dbf`）；7.2 Info-1 REST `workspace_session_confirm` 对 SC plan session 可达性封堵确认在案 | 已核销/确认，登记备查 | controller（已闭环） |

## 台账反查入口

验收产物全部可从台账与测试名反查：任务/修复轮/Ruling 账见 `.superpowers/sdd/2026-08-31_计划文档_阶段3对话式人工门与advance_v1.0/progress.md`（gitignored，会话内在盘）；scenario 行的 `rust:`/`node:` 引用由 [build_acceptance_report.mjs](build_acceptance_report.mjs) 对仓库源码 rg 解析复核；脱敏 durable 证据见 [evidence/](evidence)。
