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
| 5 | kimi coding 腿——Internal Reviewer 角色 | Internal Reviewer | **未到达真实终态**：driver 60min 硬超时切断时 WI-003 running、internal_pr_review_complete 事件未出现、usage_by_role 无 internal_reviewer 键；**后续实读（+80min 观察）**：attempt 转 **blocked**——WI-003 Coder 报告 plan defect（1 条 finding：「Bash 执行环境根文件系统只读挂载、git add 无法创建 index.lock、提交责任无法执行；实现与验证 CHECK-003 已完成仅提交环节被环境阻塞」）→ 分诊门（coding_blocked_gate_0001「Coder 输出需要人工分诊」）**正确拦截**。该 finding 为**幻觉环境误报**（convergence-36 收官跑同款谱系——kimi-heavy README #5「文件系统只读挂载」误报，实测环境 rw；真因=F-15 fixture 000 权限）→ R2 provider 不可控面，处置=人工分诊或重跑收敛，移交 controller。**v25 续跑验证（2026-09-19 09:31，升级收口尝试）**：人工分诊已放行（gate_0002 06:57，fixture 已 chmod 644）但 runner 复死于 pre-provider 窗口→F-14 修复 fail-closed 生效→awaiting_manual_recovery（abort-only：wire 三探测全拒 coding_message_not_allowed，F-16 登记）→controller 批准 abort（WI-003 未提交实现 stash 保全）→attempt 终态 **aborted**（09:42:58Z）→ **本 attempt internal_pr_review 永不可达，升级失败维持受限登记** | API 实读（GET coding-attempts/0556a410：pending_gates/blocked gate 全文在档）；convergence-36 谱系引用 `workitem-conversational-gate-advance/evidence/convergence-36/kimi-heavy/README.md`；v25 证据 `kimi-coding/coding-kimi_code-coding_attempt_0556a410…/v25-resume-{ws.jsonl,snapshot.json}` + `v25-retry-probe-ws.jsonl` |

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
5. **runner pre-provider 间歇性死亡（v25 复发，死因不可考）**——2026-09-19 09:31 现场：attach 半启动重启→stage_gate_0010 过期后 ~135ms runner 死亡（零 provider 输出、零 role run），F-14 修复 fail-closed 转 awaiting_manual_recovery（**修复目的达成：不再静默死循环**——v24 同场景 gate 0006/0007/0008 空转对照）。死亡模式间歇（v24 06:15/06:21 死、06:28 活）。死因串不可观测：仅达 WS 客户端（09:31 attach 方=监控会话，帧不可回收）与服务器 tty（/dev/pts/1 无落盘）；durable 层无错误尾帧。定性：**F-14 拉起链修复后的残余早期流失败，待查**（后续轮需服务端日志通道，与 §5 wire 原文回填同款口径）。处置（controller 裁决 09:4x）=attempt 0556a410 批准 abort（两度人工分诊+两度 pre-provider 死+parked 无通道=价值耗尽）→ 09:41 abort 首次被 shared_worktree_dirty_manual_gate 拦（blocked_gate_0003：WI-003 未提交实现保全需要）→ stash push -u 保全（stash@{0}，CHECK-003 7/7 已验证，清单 `v25-worktree-dirty-inventory.txt`）→ 09:42:58 二次 abort 成功，durable 终态 **aborted**，共享 worktree 锁释放（current_active_work_item_id=null）。**同窗口死亡对后续轮重跑构成现实风险**（F-16 恢复通道缺失下，新 attempt 若再死同窗口即直接烧掉升级机会）。

6. **计划腿结构化输出弯引号缺口（2026-09-20 wave2-2b，v27）**——kimi reviewer 原始 verdict=pass 但 nonce 属性用中文弯引号（`nonce=“86bc0a08”`）→ `missing_start_tag` → fallback needs_human（native_human_required）→ rep4 整腿 15min 报废。本轮经 takeover→typed confirm→Confirmed→typed advance 产品恢复面绕过（rep6-takeover/ 全链实录）。修复选项（解析器归一化/提示词加固）登记待后续轮。
7. **F-17：kimi coder 承包契约 vs bwrap 只读终端沙箱系统性冲突（2026-09-20 wave2-2b，v27）**——kimi 终端跑在 `--ro-bind / /`+`--tmpfs /tmp`（sandbox.rs），终端内 worktree/gitdir 均不可写 → coder 契约（TDD 写路径+commit 责任）在终端内不可履行 → `git add` 报 index.lock Read-only → coder 判 plan defect → blocked 门人工分诊（attempt 544a1b51 两掷两中：gate_0001 Write×15 后 commit 死、gate_0002 零 Write 纯 Bash 全灭）。**修正 §1#5 定性**：v24「幻觉环境误报」实为同机制真实现象（宿主 git add 实测成功对照在档）。台账 F-17 段+wave2-2b-report.md §3。
8. **F-18：coding runner 事件流单连接绑定、无广播（2026-09-20 wave2-2b，v27）**——runner 全部事件发进触发连接 mpsc；唯一消费者断开→runner 死 `coding_event_channel_closed`（本轮分诊脚本过早关连接实录+诊断尾帧捕获；§3#5「pre-provider 间歇死」同机制收敛）；外部恢复/分诊后 campaign 驱动重挂载永久失明（attach 后零事件实录）→产品内无「外部恢复+驱动接管」完整通路，本轮以自制单连接持久驱动 coding-drive.cjs 绕过。台账 F-18 段+wave2-2b-report.md §3。

## §4 验证轮时长与异常记录（WP2.5）

| 跑 | 时长 | 结果 |
|---|---|---|
| workitem rep1 | 1.7s | driver_error（`$.flow_kind 必须为 legacy，实际为 single_candidate`——脚本默认期望与 v24 服务端 flag 不匹配，改 `ARIA_EXPECTED_FLOW_KIND=single_candidate` 对齐；证据留档） |
| workitem rep2（auto_if_valid） | 1847s | **全绿**（§1#2） |
| coding 腿（误接 codex） | ~5min | 无效 kimi 证据（provider=codex 实读即中止，attempt 经 DELETE API 清理；ws.jsonl 留档为反证） |
| workitem rep3（interactive） | 2100s | hard_timeout：返修预算耗尽停 waiting_for_human（§3#3 游走） |
| coding 腿（kimi，0556a410） | 3600s | driver 硬超时切断；服务端继续推进（§1#3-5） |
| v25 续跑验证（0556a410 升级收口） | ~12min 观察+探测 | attempt 已被 09:31 未知 attach（监控会话嫌疑）触发半启动重启→runner pre-provider 死（§3#5）→awaiting_manual_recovery；wire 三探测全拒（F-16 登记）；controller 批准 abort：stash 保全+二次 abort 成功→终态 aborted（09:42:58）——**升级失败，维持受限登记**（kimi-coding/…0556a410…/v25-resume-{ws.jsonl,snapshot.json}+v25-retry-probe-ws.jsonl+v25-worktree-dirty-inventory.txt 在档） |
| wave2-2b 重跑计划腿 rep4（auto） | 896s | stopped_needs_human：reviewer 原始 pass 但弯引号→missing_start_tag→needs_human（§3#6）；takeover 交互子会话 confirm→Confirmed→typed advance 打通，attempt 544a1b51 建立（kimi 三角色冻结） |
| wave2-2b coding 腿（544a1b51） | run1 962s+分诊/恢复/接管 ~20min | run1 WI-001 coder（Write×15，18/18 测试过）报 commit 沙箱死→blocked_gate_0001→分诊放行（脚本过早关连接→runner 死 coding_event_channel_closed→AMR，§3#8）→F-16 recover_coding 一次成功（生产首验）→run2 纯 Bash 全灭→blocked_gate_0002→coding-drive.cjs 持久驱动接管（放行+run3 进行中）；终态以 rep6-takeover/coding-drive-summary-*.json 为准 |

异常合计 8 项（§3，#6-8 为 2026-09-20 wave2-2b 新增）；服务端 09-20 08:36 由 controller 部署 v27（worktree HEAD 9e25c45c，含 770c1f71 补齐器/F-16 恢复通道 32dc4de8/F-15 预检；本验证轮共用未重启）；wave2-2b 数据面操作=takeover POST ×1+typed confirm/advance+gate_response 分诊 ×2+recover_coding ×1（全部留痕 kimi-coding/，无 DELETE、无服务端重启、无 git 提交）。

## §5 T2 矩阵回填素材（wire 证据）

- **session/request_permission**：本轮 kimi coding 三角色全程 permission_approvals=0、ws.jsonl 无 `coding_permission_request` 帧（auto 权限模式+gate 均 stage_gate 自动放行型）——**本轮未触发**，与 T2 探测同口径如实登记「未触发」，不虚构 wire 引用；后续轮（Supervised 权限模式）回填。
- **session/update tool_call/tool_call_update 族**：coding WS 侧 `coding_execution_event`(679 帧 coding_execution_event，其中 kind=command 565 帧，agent=kimi_code)在档（Bash/Read/Edit 工具调用翻译层证据）；ACP wire 原文（session/update tool_call 通知）在服务端内部消费，本轮无服务端 wire 日志落盘通道——适配器消费面证据沿用 T2 冻结基线+本轮行为级证据（工具调用全程零协议错误/零 parse 失败），wire 级原文回填待后续轮服务端日志通道。
