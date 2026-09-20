# kimi provider 状态流转登记（REQ-PVR-04，D3 二值）

- 结论：**受限登记**（依据：evidence-matrix.md §1——Coder 与 Code Reviewer 两角色已到真实终态且证据链完整（WI-001/002 completed + code_review_complete×2 approve + usage_by_role 两键），Internal Reviewer 角色未到达真实终态（60min driver 硬超时切断；后续 +80min 实读 attempt 转 blocked；2026-09-19 09:31 v25 续跑验证：人工分诊放行后 runner 仍死于 pre-provider 窗口→F-14 修复 fail-closed 生效转 awaiting_manual_recovery（abort-only，wire 实证三探测全拒），internal_pr_review 阶段永不可达；09:42 controller 批准 abort，attempt 终态 **aborted**），三键判据与 internal_pr_review_complete 事件未齐；不满足转正的「三角色各 ≥1 案例到达真实终态」下限）
- 证据：`cadence/reports/provider-validation-round/evidence-matrix.md`（§0 版本/§1 覆盖面分栏/§3 暴露问题/§4 时长异常）；原始证据 `kimi-coding/`（workitem rep2 全链 + coding attempt 0556a410 的 result.json/ws.jsonl）与 `claude-smoke/rep1.txt`
- 受限面：
  1. coding 链 Internal Reviewer 阶段证据缺口（attempt 0556a410 **终态 aborted**（2026-09-19T01:42:58Z，controller 批准；前置链=两度人工分诊放行+两度 runner pre-provider 死亡+awaiting_manual_recovery abort-only wire 实证）——本 attempt 不可续跑；WI-003 未提交实现已 stash 保全（aria/issues/issue_0278 stash@{0}，CHECK-003 7/7 已验证）；收口路径=**后续轮以更长 `ARIA_CODING_HARD_TIMEOUT_MS` 重跑新 attempt**，新 attempt 到达 internal_pr_review_complete 即三键齐可升级转正；**受 F-16 阻塞提示**：重跑前宜先解决 pre-provider 间歇死因（§3#5）与 F-16 恢复通道，否则新 attempt 同窗口死亡会直接烧掉一次升级机会）
  2. v24 下 provider 继承路径依赖 sc_advance journal（single_candidate+auto_if_valid 直建 attempt 回落仓默认 codex，§3#1）——kimi coding 验证须走 interactive/typed advance 路径或等 per-WI 确认流接通
  3. `session/request_permission` wire 级证据未触发（auto 权限模式零审批帧，§5 如实登记）
- gate 边界（REQ-PVR-03）：本结论只 gate kimi 自身状态流转（spec/目录标注），不作为退役（REQ-WSC-07 判据原文仅 codex+pi）、多仓（change ②）或任何其他 change 的门禁、输入前提或排序依据。
- **F-16 产品缺陷登记（2026-09-19，本升级验证轮发现，controller 在案）**：`awaiting_manual_recovery` 态无 retry/recover 通道——WS 状态门仅接受 AbortAttempt（socket.rs）、状态机仅可转 Aborted（attempt.rs）、admission 拒绝 re-admit（admission.rs）、HTTP 无 recover 端点（仅 abort），wire 实证 gate_response(retry_coding)/start_coding/stage_gate_confirm 三探测全拒（coding_message_not_allowed）。F-14 fail-closed 打断了静默死循环（修复目的达成）但「人工恢复」名不副实：人工唯一动作=终止整个 attempt，无「人工排障后原 attempt 续跑」面。处置=登记待后续 change（恢复通道设计），不在本 change 修；证据 `kimi-coding/coding-kimi_code-coding_attempt_0556a410…/v25-retry-probe-ws.jsonl`。
- spec/目录标注建议（T5 落地）：受限登记→主 spec kimi 面标注受限条目（受限面 1-3 如上）+台账引用本文件；「已验证」全量标注待 Internal Reviewer 终态证据收口后由后续轮升级。

---

# codex provider 状态流转登记（DEF-PVR-ALL A/B 验证，2026-09-20 草案）

- 结论（草案，终态归 controller/user 裁定）：**workitem/work-item-plan 面=测试通过**——v27（含确定性 capability 补齐器 770c1f71）A/B campaign：A 侧 2/2 有效跑直通 Confirmed（rep1 7.03min / rep3 5.18min，初评1/复评0/返修0/provider starts 1），capability 类缺口共 6 项全部机械补齐（timeline 补齐日志+durable CAS source 双源核实），零 capability verdict、零模型返修轮、零指纹消耗；B 侧（门重测 v25）同判据同案例族 0/2 stopped_needs_human（capability 缺口模型返修跨指纹轮转不收敛）。REQ-WSC-07 codex 面四子项（Confirmed/≤12min/初评≤1且复评≤1/返修≤1）全达成——门重测「codex Confirmed 子项挂起」在本证据下可撤销。
- 如实登记（非判据反例）：rep2（issue_0287）workspace_error——compile 阶段 lowering_error×3（输入契约缺 compatibility_policy），初评未进入，补齐器未触达（compile 失败先于校验，fail-closed 原样上抛=既有语义）；RR-3 定向复跑 rep3 Confirmed，失败集无交集→随机作者语法质量缺陷，非系统性。malformed markdown 的 compile 失败是否增设机械修复/返修路由=后续 change 裁决面，登记不扩 scope。
- 证据：`.superpowers/sdd/2026-09-04_计划文档_3.6全量收敛轮_v1.0/wave2-2a-report.md`（判据五元组钉死+逐项判定+补齐日志/CAS 落盘全文）；原始产物 `cadence/reports/provider-validation-round/codex-ab/codex/rep{1,2,3}/`；B 侧历史对照 `cadence/reports/legacy-protocol-retirement/wp1-gate-retest/campaign/codex/rep{1,2}/`；服务器=v27 PID 2698026（runtime-info git_sha=556f74b36f56，二进制字节级含补齐器字符串）。
- 范围边界：本结论只覆盖 workitem/work-item-plan campaign 面；codex coding 链（internal_pr_review 等）证据不在本轮采集范围（kimi 线先行，codex coding 面随后续轮）。gate 边界（REQ-PVR-03）同 kimi 段口径。
