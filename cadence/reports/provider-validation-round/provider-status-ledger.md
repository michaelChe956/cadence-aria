# kimi provider 状态流转登记（REQ-PVR-04，D3 二值）

- 结论：**受限登记（2026-09-20 重跑轮维持；收口尝试进行中）**（依据：evidence-matrix.md §1——Coder 与 Code Reviewer 两角色已到真实终态且证据链完整（WI-001/002 completed + code_review_complete×2 approve + usage_by_role 两键），Internal Reviewer 角色未到达真实终态（60min driver 硬超时切断；后续 +80min 实读 attempt 转 blocked；2026-09-19 09:31 v25 续跑验证：人工分诊放行后 runner 仍死于 pre-provider 窗口→F-14 修复 fail-closed 生效转 awaiting_manual_recovery，internal_pr_review 阶段未达；09:42 controller 批准 abort，attempt 0556a410 终态 **aborted**）；**2026-09-20 重跑轮（wave2-2b）**：新 attempt 544a1b51（sc_advance 冻结 kimi 三角色）建链成功——计划腿经弯引号缺口→takeover 分诊→Confirmed→typed advance 打通；coding 腿因 F-17 沙箱契约冲突两度 blocked 门分诊重试+F-18 事件绑定约束下以持久驱动接管，WI-001 coder run3 运行中（截止登记时刻），三键判据与 internal_pr_review_complete 事件仍未齐；不满足转正的「三角色各 ≥1 案例到达真实终态」下限。终态以 `kimi-coding/kimi_code/rep6-takeover/coding-drive-summary-*.json` 为准复核）
- 证据：`cadence/reports/provider-validation-round/evidence-matrix.md`（§0 版本/§1 覆盖面分栏/§3 暴露问题/§4 时长异常）；原始证据 `kimi-coding/`（workitem rep2 全链 + coding attempt 0556a410 的 result.json/ws.jsonl）与 `claude-smoke/rep1.txt`
- 受限面：
  1. coding 链 Internal Reviewer 阶段证据缺口（0556a410 已 aborted 不可续跑（WI-003 stash 保全 aria/issues/issue_0278 stash@{0}）；**收口路径已启动**：2026-09-20 重跑 attempt `coding_attempt_544a1b51644c40aa8bf355cf278439af`（issue_0286，ARIA_CODING_HARD_TIMEOUT_MS=7200000 口径）由后台持久驱动（coding-drive.cjs，有界：门放行≤4/恢复≤6/110min）推进至 WI-001 coding 阶段——到达 internal_pr_review_complete 即三键齐可升级转正；**F-16 阻塞提示已解除**（v27 恢复通道 recover_coding 生产实跑一次成功，见 F-16 段）；新阻塞面=F-17（kimi coder 承包契约 vs bwrap 只读终端沙箱系统性冲突，本轮两掷两中 blocked 门）与 F-18（事件流单连接绑定，外部恢复后 campaign 驱动失明，须单连接驱动绕过）——两项不修则 kimi coding 到 internal_pr_review 的通路仍是概率性掷骰）
  2. v24 下 provider 继承路径依赖 sc_advance journal（single_candidate+auto_if_valid 直建 attempt 回落仓默认 codex，§3#1）——kimi coding 验证须走 interactive/typed advance 路径或等 per-WI 确认流接通
  3. `session/request_permission` wire 级证据未触发（auto 权限模式零审批帧，§5 如实登记）
- gate 边界（REQ-PVR-03）：本结论只 gate kimi 自身状态流转（spec/目录标注），不作为退役（REQ-WSC-07 判据原文仅 codex+pi）、多仓（change ②）或任何其他 change 的门禁、输入前提或排序依据。
- **F-16 产品缺陷登记（2026-09-19，本升级验证轮发现，controller 在案；2026-09-20 生产实跑验证修复生效）**：`awaiting_manual_recovery` 态无 retry/recover 通道——WS 状态门仅接受 AbortAttempt（socket.rs）、状态机仅可转 Aborted（attempt.rs）、admission 拒绝 re-admit（admission.rs）、HTTP 无 recover 端点（仅 abort），wire 实证 gate_response(retry_coding)/start_coding/stage_gate_confirm 三探测全拒（coding_message_not_allowed）。F-14 fail-closed 打断了静默死循环（修复目的达成）但「人工恢复」名不副实：人工唯一动作=终止整个 attempt，无「人工排障后原 attempt 续跑」面。处置=登记待后续 change（恢复通道设计）；证据 `kimi-coding/coding-kimi_code-coding_attempt_0556a410…/v25-retry-probe-ws.jsonl`。**2026-09-20 更新**：v27（32dc4de8 恢复通道+542cc5af 尾帧独立号段）已上线并经 544a1b51 生产实跑验证——AMR→`recover_coding`→running 一次成功（version+1、runner 过死亡窗），死因尾帧 `coding_event_channel_closed` durable 落盘可考（`issue_0286/coding-attempts/…/chat-entries/coding_manual_recovery_diagnostic_0001.json`）——修复目的达成。
- **F-17 产品缺陷登记（2026-09-20 wave2-2b 发现，controller 在案）**：kimi coder 承包契约与 bwrap 只读终端沙箱系统性冲突——kimi 终端跑在 `--ro-bind / /`+`--tmpfs /tmp`（sandbox.rs:257-317），worktree/gitdir 终端内均不可写；coder 契约要求 TDD 写路径+write_policy.commit_responsibility（目标仓 23457ba 开启 commit 责任）→ 终端 `git add/commit` 必死（index.lock: Read-only file system）→ coder 判 plan defect（operational_blocker）→ blocked 门人工分诊。**修正既有登记**：v24 0556a410 WI-003「幻觉环境误报」实为同机制真实现象（宿主侧 git add 实测成功 vs 沙箱内必败对照在档）；开门与否取决于 coder 分类行为（v24 两例剩余风险不开门/一例开门，v27 两掷两中开门）=概率性。证据 `issue_0286/coding-attempts/…/blocked-gates/coding_blocked_gate_000{1,2}.json`+provider-raw/coder_output_0001.txt+role-run-events 工具直方（Write×15 vs Write×0）+宿主实测对照；详见 wave2-2b-report.md §3。
- **F-18 产品缺陷登记（2026-09-20 wave2-2b 发现，controller 在案）**：coding runner 事件流绑定单连接、无广播——spawn_coding_runner 把全部事件发进触发连接的 mpsc（socket.rs 各 spawn 点传 event_tx.clone()），coding_sockets 注册表 sender() 仅 amendment 线使用；其他连接 attach 只得首帧快照。后果：①唯一消费者断开→runner 死 coding_event_channel_closed→AMR（本轮分诊放行脚本过早关连接实录；v25 §3#5「pre-provider 间歇死」同机制收敛）；②外部恢复/分诊触发 runner 后 campaign 驱动重挂载永久失明（本轮实录：attach 后零事件到进程被杀）→产品内无「外部恢复+驱动接管」完整通路。处置=登记待后续 change（广播扇出/事件回放）；本轮绕过=单连接持久驱动 coding-drive.cjs 收敛全部应答面。详见 wave2-2b-report.md §3。
- **计划腿结构化输出弯引号缺口登记（2026-09-20 wave2-2b 发现）**：kimi reviewer 原始 verdict=pass 但 nonce 属性用中文弯引号（U+201C/U+201D，`nonce=“86bc0a08”`）→ 解析器要求 ASCII 引号 → missing_start_tag → 降级 needs_human → 整腿报废（rep4 实录，15min）；修复选项=structured_output.rs nonce 属性归一化或评审提示词 nonce 字符加固，登记待后续轮。本轮经 takeover→confirm→Confirmed→typed advance 产品恢复面绕过（全链打通实录 rep6-takeover/）。
- spec/目录标注建议（T5 落地）：受限登记→主 spec kimi 面标注受限条目（受限面 1-3 如上）+台账引用本文件；「已验证」全量标注待 Internal Reviewer 终态证据收口后由后续轮升级。

---

# codex provider 状态流转登记（DEF-PVR-ALL A/B 验证，2026-09-20 草案）

- 结论（草案，终态归 controller/user 裁定）：**workitem/work-item-plan 面=测试通过**——v27（含确定性 capability 补齐器 770c1f71）A/B campaign：A 侧 2/2 有效跑直通 Confirmed（rep1 7.03min / rep3 5.18min，初评1/复评0/返修0/provider starts 1），capability 类缺口共 6 项全部机械补齐（timeline 补齐日志+durable CAS source 双源核实），零 capability verdict、零模型返修轮、零指纹消耗；B 侧（门重测 v25）同判据同案例族 0/2 stopped_needs_human（capability 缺口模型返修跨指纹轮转不收敛）。REQ-WSC-07 codex 面四子项（Confirmed/≤12min/初评≤1且复评≤1/返修≤1）全达成——门重测「codex Confirmed 子项挂起」在本证据下可撤销。
- 如实登记（非判据反例）：rep2（issue_0287）workspace_error——compile 阶段 lowering_error×3（输入契约缺 compatibility_policy），初评未进入，补齐器未触达（compile 失败先于校验，fail-closed 原样上抛=既有语义）；RR-3 定向复跑 rep3 Confirmed，失败集无交集→随机作者语法质量缺陷，非系统性。malformed markdown 的 compile 失败是否增设机械修复/返修路由=后续 change 裁决面，登记不扩 scope。
- 证据：`.superpowers/sdd/2026-09-04_计划文档_3.6全量收敛轮_v1.0/wave2-2a-report.md`（判据五元组钉死+逐项判定+补齐日志/CAS 落盘全文）；原始产物 `cadence/reports/provider-validation-round/codex-ab/codex/rep{1,2,3}/`；B 侧历史对照 `cadence/reports/legacy-protocol-retirement/wp1-gate-retest/campaign/codex/rep{1,2}/`；服务器=v27 PID 2698026（runtime-info git_sha=556f74b36f56，二进制字节级含补齐器字符串）。
- 范围边界：本结论只覆盖 workitem/work-item-plan campaign 面；codex coding 链（internal_pr_review 等）证据不在本轮采集范围（kimi 线先行，codex coding 面随后续轮）。gate 边界（REQ-PVR-03）同 kimi 段口径。

## DEF-PVR-ALL 裁决（2026-09-20，Wave 3.2）

| Provider | 终态 | 依据 |
|---|---|---|
| **pi** | ✅ **全量通过** | 门重测 rep2 Confirmed 424.5s=7.09min（pi 时长+计数全子项达标）；全程 campaign 实证 |
| **codex** | ✅ **通过（补齐器后）** | A/B 对照实证：补齐器上线后 rep1/rep3 Confirmed（7.03/5.18min，零返修零指纹）；门重测 codex 面四子项全达成；原 B 裁决例外已可撤销 |
| **claude_code** | ✅ **通过** | REQ-CCI-06 smoke 一次 PASS（四段所有权链完整）；headless 修复已落地 |
| **kimi_code** | 🔶 **受限登记维持** | Coder+Code Reviewer 真实终态证据完整（1103/1598/70272 usage+code_review_complete）；**Internal Reviewer 未到达**——三键不齐；attempt 544a 冻结 code_review>2h（drive 超时+服务端无推进）；深层障碍=F-17（bwrap 沙箱 vs coder commit 契约）+F-19 广播后驱动失明+验证证据不可观测环境限制 |
| | | **kimi 升级路径已铺平**：F-16 恢复通道+F-19 广播已修+F-15 权限预检已修——后续轮在 bwrap 沙箱面解除后可重跑收口 |
