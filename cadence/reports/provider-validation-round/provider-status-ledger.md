# kimi provider 状态流转登记（REQ-PVR-04，D3 二值）

- 结论：**受限登记**（依据：evidence-matrix.md §1——Coder 与 Code Reviewer 两角色已到真实终态且证据链完整（WI-001/002 completed + code_review_complete×2 approve + usage_by_role 两键），Internal Reviewer 角色未到达真实终态（60min driver 硬超时切断；后续 +80min 实读 attempt 转 **blocked**：WI-003 Coder 幻觉环境误报「文件系统只读」→ 分诊门正确拦截待人工分诊，convergence-36 同谱系），三键判据与 internal_pr_review_complete 事件未齐；不满足转正的「三角色各 ≥1 案例到达真实终态」下限）
- 证据：`cadence/reports/provider-validation-round/evidence-matrix.md`（§0 版本/§1 覆盖面分栏/§3 暴露问题/§4 时长异常）；原始证据 `kimi-coding/`（workitem rep2 全链 + coding attempt 0556a410 的 result.json/ws.jsonl）与 `claude-smoke/rep1.txt`
- 受限面：
  1. coding 链 Internal Reviewer 阶段证据缺口（attempt 0556a410 现 blocked 于 WI-003 Coder 幻觉误报分诊门——controller 人工分诊放行或后续轮以更长 `ARIA_CODING_HARD_TIMEOUT_MS` 重跑收敛收口；收口即三键齐可升级转正）
  2. v24 下 provider 继承路径依赖 sc_advance journal（single_candidate+auto_if_valid 直建 attempt 回落仓默认 codex，§3#1）——kimi coding 验证须走 interactive/typed advance 路径或等 per-WI 确认流接通
  3. `session/request_permission` wire 级证据未触发（auto 权限模式零审批帧，§5 如实登记）
- gate 边界（REQ-PVR-03）：本结论只 gate kimi 自身状态流转（spec/目录标注），不作为退役（REQ-WSC-07 判据原文仅 codex+pi）、多仓（change ②）或任何其他 change 的门禁、输入前提或排序依据。
- spec/目录标注建议（T5 落地）：受限登记→主 spec kimi 面标注受限条目（受限面 1-3 如上）+台账引用本文件；「已验证」全量标注待 Internal Reviewer 终态证据收口后由后续轮升级。
