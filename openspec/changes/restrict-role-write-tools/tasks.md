# Tasks: restrict-role-write-tools

## 0. 实施首步实测门禁（三项独立，任一不符 → change 停止上报）

- [ ] 0.1 claude：`--disallowedTools Edit,Write,NotebookEdit` 行为复核（名单大小写/写形态覆盖/与 `--resume` 组合）；证据固定落盘 `.superpowers/sdd/2026-09-04_计划文档_3.6全量收敛轮_v1.0/f3-task0-claude.md`（版本+命令+原始输出+通过断言），失败→change 停止待用户重开
- [ ] 0.2 codex：read-only+on-request+MCP 审批全链 preflight（记录启动参数+MCP/fileChange/commandExecution 原始 wire）+ **Coder(danger-full-access+never)下 fileChange/MCP 是否到达验证**（I-R3-2 证据）；证据固定 `.superpowers/sdd/2026-09-04_计划文档_3.6全量收敛轮_v1.0/f3-task0-codex.md`（版本+命令+输出+通过断言+失败→change 停止）
- [ ] 0.3 pi：`--exclude-tools`×`--session-id` 组合与空 id 边界复核；证据固定 `.superpowers/sdd/2026-09-04_计划文档_3.6全量收敛轮_v1.0/f3-task0-pi.md`（版本+命令+输出+通过断言+失败→change 停止）

## 1. 语义策略与宿主注入（REQ-ENV-09）

- [ ] 1.1 定义 `ProviderToolPolicy`（DenyFileWriteBuiltins 唯一合法意图）+三 provider translator+canonical digest 规范实现（sha256/tp-v1/片段 token 序列，向量单测钉死），TDD 先失败测试
- [ ] 1.2 作者/评审家族 builder 按 D2 全表注入策略（SC author/revision、WorkItemPlan 普通/fresh/resume/with-session、serial/batch、SC reviewer、review repair、coding CodeReviewer/InternalReviewer、group review），表驱动断言+工厂全集断言（禁止绕过工厂裸构造直启）
- [ ] 1.3 pi adapter `build_args` 注入 `--exclude-tools edit,write`（含 `--session-id` 组合与空 id 边界）

## 2. claude/codex 执行点（REQ-ENV-09）

- [ ] 2.1 claude adapter `build_args` 注入 `--disallowedTools Edit,Write,NotebookEdit`（含 `--resume` 组合）
- [ ] 2.2 codex adapter thread/start 与 thread/resume 两路：策略会话（作者/评审）`sandbox=read-only`+`approvalPolicy=on-request`；Coder 维持现状；request id 空间隔离
- [ ] 2.3 codex 审批分类（D5）：parse 层识别三类+未知形态；策略会话按类即时决策（MCP=accept、exec/fileChange=decline+审计）；Coder commandExecution/fileChange 维持 bridge 链、MCP=accept（execution event 通道）；未知形态按确定性应答表（elicitation→JSON-RPC error -32601+data；item→decline；连续≥3→session_terminated unknown_approval_storm）+每类未知 method fixture 单测+request id 碰撞 fixture（server `0`×client `aria-0`）

## 3. 守卫、审计与 resume（REQ-ENV-09）

- [ ] 3.1 三 adapter 双向 spawn 前守卫：作者/评审 role 必带、Executor/Handoff 禁带（全 AdapterRole 枚举测试）
- [ ] 3.2 LifecycleStore tool-policy-run-audit 分区（含 D7 实现接点）：`ToolPolicyAuditSink` trait+engine 注入（input.audit_sink）、role_run_seq 分配（run 记录持久化单调）、provider_session_id 启动后回填、kill 链（adapter 错误→engine provider task）、事件枚举（provider_start/approval_decision/protocol_warning/session_terminated）+schema v1 JSON 示例钉死；追加失败⇒终止会话+run 失败（失败注入测试）；读取坏行=跳过+告警（不入分区）测试+启动握手 session id 确认超时/缺失 fail-closed fixture
- [ ] 3.3 resume 冻结比对：spawn 前分区内检索 provider_start，比对三元组（digest 含 ap-v1 规则版本）；version=CLI 探测（不可得→fail-closed）/dialect=adapter 常量；不一致/缺失⇒拒绝 resume、superseded、新建（漂移/缺失/version 变化/dialect 变化 fixture 单测）

## 4. 回归与验证（REQ-ENV-09）

- [ ] 4.1 零变化回归：Coder/非策略路径（无黑名单、无档位变化）；ApprovalBridge commandExecution 链路断言；kimi 现状断言（四角色表）；既有 pi→Auto 断言；正向可用性断言（排除面仅写工具，extension/MCP/ask_user 不受影响）；「静默不应答→拒绝应答+上报」新行为回归锁定
- [ ] 4.2 运行验收 operational checklist（owner=controller）：①全量门禁四条标准命令绿（pass=0 fail）；②部署 4317（PID/exe/md5 三对账，任一不符=失败回滚）；③pi×轻 3 连跑+codex 审批分类真实验证（证据入 `cadence/reports/workitem-conversational-gate-advance/evidence/convergence-f3/`）；④claude/codex 随 3.6 矩阵格首跑；前置检查：kimi 目标仓 Trust 状态、codex `.codex/config.toml` MCP 就绪（用户侧）；停止条件：任一门禁/连跑失败→停,记录待裁决;回滚触发：部署后 pi×轻 连跑出现写面回归
