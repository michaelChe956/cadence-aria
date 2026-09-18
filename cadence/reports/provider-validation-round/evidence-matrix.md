# C1 Task 3 验证轮证据矩阵（evidence-matrix）

## §0 版本与授权

- 实跑日期：2026-09-19（UTC 2026-09-18 16:2x–18:4x）
- `kimi --version` = **0.43.0**（T2 已记录，本轮沿用同机登录态）
- `claude --version` = **2.1.272**（≥2.1.237 下限 ✓，REQ-CCI-06）
- 服务端：`aria web`（HEAD `00239804` 构建，PID 784706，`--host 127.0.0.1 --port 4317 --work-item-plan-single-candidate`，controller 于 2026-09-19 00:20 启动，本轮未重启未动服务器）
- 真实跑纪律：campaign 双驱动器链显式无 `--dry-run`；claude smoke 走 `CLAUDE_E2E=1` gated live 测试（kimi `live_kimi_tests.rs` 同款门惯例）
- 授权链：9-16/17「共识即执行」+9-18 双审通过后 controller 在案推进（controller 排期本轮执行窗口）

## §1 覆盖面分栏

### 真实轮覆盖

| # | 面 | 角色 | 终态/证据 | 证据路径 |
|---|---|---|---|---|
| 1 | claude headless smoke（WP1.3/REQ-CCI-06） | Executor（Auto） | **PASS 一次过**（12.38s）：choice_request→choice_response_sent→completed(`smoke-ok`)，所有权链完整（REQ-CCI-04 等待证明成立） | `claude-smoke/rep1.txt`（含 aria-choice-diag 四段诊断行） |
| 2 | kimi workitem 腿（author/reviewer 双角色） | Plan Author + Plan Reviewer | **全绿**（rep2，1847s）：plan Confirmed（session_status=confirmed）+ WI-001/002/003 + handoff.json + 两轮 review（round1 revise→返修→round2 pass）+ usage_by_role.reviewer（input 10883/output 1422/cache_read 38784）+ provider_start_count=2 + stage 时间线 prepare→running→cross_review→completed | `kimi-coding/kimi_code/rep2/{result.json,ws.jsonl,handoff.json}`；console 留档 `workitem-rep2-console.txt`（脚本 stdout 为空，判据以 result.json 为准） |
| 3 | kimi coding 腿——Coder 角色 | Coder | **WI-001/WI-002 到达 completed**（group_coding_progress 实读）；usage_by_role.author（input 1642/output 2700/cache_read 100800）；coding 阶段 1246s+两轮返修推进 | `kimi-coding/coding-kimi_code-coding_attempt_0556a410c0de429db96c9550c0b80fa2/{result.json,ws.jsonl}` |
| 4 | kimi coding 腿——Code Reviewer 角色 | Code Reviewer | **code_review_complete ×2 在场**：round1 verdict=approve（CT-001/002/003 契约核对+6 测试文件约 20 用例核验）、round2 verdict=approve（CT-004+2 文件 13 用例）；usage_by_role.reviewer（input 3678/output 1793/cache_read 60288） | 同上 ws.jsonl（帧类型统计：code_review_complete=2） |
| 5 | kimi coding 腿——Internal Reviewer 角色 | Internal Reviewer | **未到达真实终态**：driver 60min 硬超时切断时 WI-003 running、internal_pr_review_complete 事件未出现、usage_by_role 无 internal_reviewer 键；**后续实读（+80min 观察）**：attempt 转 **blocked**——WI-003 Coder 报告 plan defect（1 条 finding：「Bash 执行环境根文件系统只读挂载、git add 无法创建 index.lock、提交责任无法执行；实现与验证 CHECK-003 已完成仅提交环节被环境阻塞」）→ 分诊门（coding_blocked_gate_0001「Coder 输出需要人工分诊」）**正确拦截**。该 finding 为**幻觉环境误报**（convergence-36 收官跑同款谱系——kimi-heavy README #5「文件系统只读挂载」误报，实测环境 rw）→ R2 provider 不可控面，处置=人工分诊或重跑收敛，移交 controller | API 实读（GET coding-attempts/0556a410：pending_gates/blocked gate 全文在档）；convergence-36 谱系引用 `workitem-conversational-gate-advance/evidence/convergence-36/kimi-heavy/README.md` |

### 未在真实轮覆盖（如实登记）

- 普通 Workspace 角色（author/reviewer 的对话式 Workspace 流）：不在本验证轮覆盖面；已有面=provider 级单测+前端回归，**非真实轮证据**。
- image-create：不在覆盖面，同上口径。

## §2 claude smoke 四段序列映射（REQ-CCI-06 场景 1）

真实 claude CLI 2.1.272（Auto 模式、`--permission-prompt-tool=stdio` 注册面由 D1 既有实现提供）：

1. **tool_use**：`[aria-choice-diag] claude received assistant tool_use AskUserQuestion tool_use_id=call_8900d4a9b30946788eed12c6`
2. **control_request**：`claude received control_request AskUserQuestion request_id=cae0692c-… tool_use_id=call_8900…`（can_use_tool 回调到达=stdio 注册生效）
3. **control_response**：`bridge received choice_response … claude writing control_response request_id=cae0692c-… answer_keys=["Which color do you prefer?"]`（aria 只回写决策、不注入 tool_result）
4. **tool_result→Completed**：CLI 生成原生 tool_result（aria 仅消费缓存），`EVIDENCE completed output=smoke-ok`

判据：ChoiceRequest 前无 Completed（Auto 不自动批准，REQ-CCI-04）✓；回 ChoiceResponse 后 240s 内 Completed 且输出含 `smoke-ok` ✓；一次通过无需重跑（无「模型不触发」场景）。

## §3 修暴露问题清单（本轮实跑暴露、按 RR-3 定性）

1. **v24 single_candidate+auto_if_valid 下 coding attempt provider 回落仓默认（codex）**——首败事实：rep1 后首次 coding 腿（attempt 41c86f73）provider_snapshot 三角色全为 codex（`coding_gate_required` 帧实读），驱动器 `handoff.provider` 仅用于输出目录命名。定性：**driver 配置面缺口+服务端 provider 继承路径依赖**（`coding.rs:592-644` runtime binding 仅从 Confirmed 的 per-WI WorkItem session 继承，single_candidate flow 下 per-WI session 停 open；无 journal 时回落 `repository.default_provider_mode`=codex）。处置：不改服务端、不动仓库配置——改走 sc_advance journal 路径（typed advance 从 plan session 冻结 kimi provider，`advance.rs:782-793`），复跑后 provider=kimi_code 三角色确认（journal 实读 admission_kind=sc_advance/provider={author:kimi_code,reviewer:kimi_code}）。codex 腿 attempt 经生产 DELETE API 清理（连带 group-initialization journal）。
2. **Failed advance durable record 无生产清除路径**——typed advance 首次尝试被旧 LegacyGroup journal 拒（record Failed 持久化，`get_advance_for_plan` 对 Failed record 返回 Replayed 永久阻塞该 plan 后续 advance；AdvanceStore 无 delete API）。处置：备份后（`advance-record-failed-backup.json`）手动移除该 record——其为本轮 5 分钟前自产的失败残留（绑定已删除 attempt），非历史台账数据。**登记为服务端恢复面缺口（呈报 controller，不在本 change 修）**。
3. **kimi plan 质量游走（R2 provider 不可控面）**：rep3（interactive）5 轮 review 全 revise、返修预算耗尽停 waiting_for_human（35min 硬超时）；rep2（auto_if_valid）同 fixture 2 轮 pass。与 convergence-36 谱系「flash 概率游走」一致，非缺陷。
4. **coding 腿 60min 硬超时不足**：kimi 三角色链（3 WI×（Coder+CodeReviewer）+Internal Reviewer）在 60min 内未收束（WI-001/002 完成、WI-003 进行中被切断）。属预算面（README 记 `ARIA_CODING_HARD_TIMEOUT_MS` 可调），非缺陷；attempt 在服务端继续推进（自主驱动，不需要 driver 干预的证据：driver 断开后 WI-003 持续 running）。

## §4 验证轮时长与异常记录（WP2.5）

| 跑 | 时长 | 结果 |
|---|---|---|
| workitem rep1 | 1.7s | driver_error（`$.flow_kind 必须为 legacy，实际为 single_candidate`——脚本默认期望与 v24 服务端 flag 不匹配，改 `ARIA_EXPECTED_FLOW_KIND=single_candidate` 对齐；证据留档） |
| workitem rep2（auto_if_valid） | 1847s | **全绿**（§1#2） |
| coding 腿（误接 codex） | ~5min | 无效 kimi 证据（provider=codex 实读即中止，attempt 经 DELETE API 清理；ws.jsonl 留档为反证） |
| workitem rep3（interactive） | 2100s | hard_timeout：返修预算耗尽停 waiting_for_human（§3#3 游走） |
| typed advance | <1s×3 | 第 1 次被旧 journal 拒（§3#2）；第 2 次被 Failed record 阻塞（清除后）；第 3 次 **advance_completed**（attempt 0556a410） |
| coding 腿（kimi，0556a410） | 3600s | driver 硬超时切断；服务端继续推进（§1#3-5） |

异常合计 4 项（§3）；无服务端崩溃/重启；数据面操作=2 次生产 API（abort+DELETE 无效 attempt）+1 次手动清自产 Failed advance record（备份在档）。

## §5 T2 矩阵回填素材（wire 证据）

- **session/request_permission**：本轮 kimi coding 三角色全程 permission_approvals=0、ws.jsonl 无 `coding_permission_request` 帧（auto 权限模式+gate 均 stage_gate 自动放行型）——**本轮未触发**，与 T2 探测同口径如实登记「未触发」，不虚构 wire 引用；后续轮（Supervised 权限模式）回填。
- **session/update tool_call/tool_call_update 族**：coding WS 侧 `coding_execution_event`(679 帧 coding_execution_event，其中 kind=command 565 帧，agent=kimi_code)在档（Bash/Read/Edit 工具调用翻译层证据）；ACP wire 原文（session/update tool_call 通知）在服务端内部消费，本轮无服务端 wire 日志落盘通道——适配器消费面证据沿用 T2 冻结基线+本轮行为级证据（工具调用全程零协议错误/零 parse 失败），wire 级原文回填待后续轮服务端日志通道。
