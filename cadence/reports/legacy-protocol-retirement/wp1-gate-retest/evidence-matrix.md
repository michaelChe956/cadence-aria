# WP1 退役门全口径重测证据矩阵（evidence-matrix）

> change ③ retire-legacy-workitem-protocol / WP1.2（REQ-RET-01；判据基准=`openspec/specs/work-item-plan-single-candidate/spec.md:121-135` REQ-WSC-07 原文，判据零改）。
> 结构沿用阶段 2 基线报告先例（`cadence/reports/workitem-coding-campaign/reports/2026-08-31_阶段2验收_单候选C′MVP实测报告.md` §2.1 字段族）。
> Q3 口径：kimi/claude 结论不进入 REQ-WSC-07 判据（判据原文仅 codex+pi），本矩阵全部行同口径。

## §0 build/预算引用

见 `budget-build-record.md`：v25（build commit `4d097c1d21a9` 含 F-14，PID 1005268，2026-09-19 07:23:24 启动，`/api/health` ok）；单案例超时 30min（`ARIA_WORKITEM_HARD_TIMEOUT_MS=1800000`）、campaign 总预算 90min、RR-3 重试策略均在**任何重测跑之前**留档。实际 campaign 窗口 09:37:30–10:04:50（≈27.3min，预算内；单案例最长 461.9s，远低于硬超时）。重测期间服务器未重启（全程 PID 1005268）。

## §1 判据-实测-结论总表（判据列逐字摘自 spec :123/:125 原文）

| 判据子项（spec 原文逐字） | 重测证据（锚） | 结论 |
|---|---|---|
| codex 与 pi 各 1 案例到达 Confirmed（2/2） | pi ✓：rep2 `campaign/pi/rep2/result.json` `session_status=confirmed`、`confirmed_count=1`、`completed=true`、durable `.aria/projects/project_0001/issues/issue_0283/workspace-sessions/workspace_session_0453.json` status=`confirmed`+handoff.json（`plan_confirmation_status=confirmed`）。codex ✗：rep1/rep2 均 `stopped_needs_human`（issue_0280/sess_0450、issue_0281/sess_0451，durable 同步核实） | **未达成（0/2 codex）** |
| 单案例时长 ≤12 分钟 | pi Confirmed 案例 424.541s=7.09min（result.json `duration_ms=424541`，stageTimeline 起止 2.516s→424.445s 同源）；codex 无 Confirmed 案例故无判据口径时长（失败案例时长如实登记 §2.1：180.827s/461.854s）；pi 先例 938.04s 对照——本轮 pi Confirmed 案例首次落 12min 内 | pi 达标本项；codex 无可判案例（随子项 1 未达成挂起） |
| 初评 ≤1 次且复评 ≤1 次（总 ≤2） | pi Confirmed：durable `run_history.initial_review_count=1`、`verification_review_count=0`（独立计数数字，非 `pass@r1` 推算）；codex 失败案例登记 rep1=2、rep2=3（初评）/均 0（复评） | pi 达标本项；codex 无可判案例 |
| 自动返修 ≤1 次（均从服务端持久计数读取） | pi Confirmed：durable `run_history.repairs_used=0`；codex 失败案例登记 rep1=1、rep2=2 | pi 达标本项；codex 无可判案例 |
| 阶段 1 的 14 条 classifier golden（rep2/3/4 的 9 条、rep1 round-1 的 2 条 Advisory、3 条人工标注 class_hint 变体）全部归入预期 finding 分类 | `cargo test --locked work_item_plan_policy` exit 0，36 passed/0 failed（含 `tests_classify.rs:24 golden_findings_classify_to_the_expected_typed_outcomes`，:27 `assert_eq!(fixtures.len(), 14)`；fixtures=`src/product/work_item_plan_policy/fixtures/golden_findings.json` 14 items 实数复核） | 达标 |
| 仅明确属 grammar/lowering 的 reviewer finding 通过 compiler diagnostic golden（其余明确仅为 prompt few-shot 素材） | `reviewer_finding_channel` exit 0（1 passed，channel 边界守卫：9 条 prompt_few_shot/compiler_fixture=null 不伪装）；`work_item_plan_policy` 36 内含 `tests_evaluate.rs`（19 test fn）compiler diagnostic golden | 达标 |
| 断线重连/恢复测试通过 | `planning_resume` exit 0（8 passed）+`interrupted_run_recovery` exit 0（5 passed）+`campaign_stage3_recovery_matrix` exit 0（7 passed） | 达标 |
| legacy 路径回归全绿 | 全量 `cargo test --locked`（10 result 块）：lib 3370/0、it_web 408/0、it_core 175/0、其余目标 54/43/31/210/1/0 全绿=**legacy 面全部目标 0 failed**；唯一红=web_logical_codebase_entrypoints 42 passed/2 failed（§2.3 登记：codegraph 环境漂移，非 legacy 面、非本 change 面、RR-3 确定性复现） | legacy 面达标；环境红项如实登记呈报 controller |

**判据零改自查**：矩阵判据列与 `spec.md:121-135` 原文逐字一致，无放宽/改口径/删减；本 Task 未改 spec 任何字符（`git status` 无 openspec/specs 变更）。

## §2 逐项证据锚

### §2.1 campaign 四跑全记录（真实跑、显式非 dry-run、同款环境）

| 跑 | issue / durable session | 终态 | 时长 | 初评/复评 | 返修 | provider_start | legacy_decision_messages | 失败指纹摘要 |
|---|---|---|---|---|---|---|---|---|
| codex rep1 | issue_0280 / workspace_session_0450 | stopped_needs_human | 180.827s | 2/0 | 1 | 2 | [] | r1 三条 contract_gap（CT-001 错误响应体能力缺失/CT-002 web 引用能力缺失/CT-003 unconsumed handoff）→返修→r2 前两条重现（指纹重复=REQ-WSC-03 终态） |
| codex rep2（RR-3 定向复跑） | issue_0281 / workspace_session_0451 | stopped_needs_human | 461.854s | 3/0 | 2 | 3 | [] | r1 CT-001 Content-Type 缺失→r2 CT-001 404 text 缺失→r3 r1 指纹重现（同族异指纹，3 轮 revise） |
| pi rep1 | issue_0282 / workspace_session_0452 | stopped_needs_human | 371.912s | 2/0 | 1 | 2 | [] | r1 CT-001「unlocked 为布尔值」+CT-002「静态资源 Content-Type」→返修→r2 同两条重现 |
| pi rep2（RR-3 定向复跑） | issue_0283 / workspace_session_0453 | **confirmed** | **424.541s** | **1/0** | **0** | 1 | [] | r1 pass（3 findings 全 advisory：completeness×2/other×1，0 must-fix；work_items WI-001/002/003；handoff.json 回读核验过） |

产物目录：`campaign/codex/rep1|rep2/`、`campaign/pi/rep1|rep2/`（result.json/ws.jsonl/artifact-v*.json；pi/rep2 另有 handoff.json）。全部指标取自 result.json（README 钉死=durable SessionState 透传）+服务端 durable 会话记录双源核实（§2.2）。

### §2.2 durable 双源核实

四个 durable 会话文件实读：`workspace_session_0450/0451/0452` 均 `status=stopped_needs_human`、`workspace_session_0453` `status=confirmed`，与 campaign result.json 一致；issue_0283 另含 3 个 per-WI authoring 会话（0454-0456，workspace_type=work_item，author=pi）为 SC 流程正常 durable 产物。

### §2.3 全量测试环境红项登记（RR-3 定性呈报 controller）

- 首败事实：全量 10 结果块中 web_logical_codebase_entrypoints 42 passed/**2 failed**：`planning::aggregate_index_rebuild_endpoint_returns_active_projection`（planning.rs:1070，422=`aggregate_index_degraded:codegraph_version_mismatch: expected 1.5.0, got 1.6.0`）与 `planning::stale_story_planning_read_syncs_index_and_returns_normal_response`（planning.rs:1118，500=`product_store_error` 包装**同一** `aggregate_index_unavailable: aggregate_index_degraded:codegraph_version_mismatch`）——两例同根因。
- 定向复跑：`cargo test --locked --test web_logical_codebase_entrypoints` 同 2 例**确定性重现**（非 flaky）。
- 定性：本机 codegraph CLI=`@colbymchenry/codegraph@1.6.0`（npm 全局，shim 2026-09-10 装）vs 产品钉定 `src/product/logical_codebase/aggregate_index/codegraph_cli.rs:16 CODEGRAPH_EXACT_VERSION="1.5.0"`（58743399 引入）——**工具链环境版本漂移，非产品回归、非本 change 面**（本 Task 零 src 改动；legacy 决策路径与 codegraph 聚合索引无交集）。
- 处置：按计划 Step 2 Expected「既有基线红（非本 change 面）→如实登记呈报 controller」，不在本 Task 修（升级产品 pin=另行决策）。

### §2.4 RR-3 定性汇总（campaign 线）

- **codex**：rep1 首败+rep2 定向复跑=同终态（stopped_needs_human）、同族（contract_gap/required_capability_missing）、异指纹（三次能力串各不同）→**系统性 provider 内容缺陷**：现行 build prompt 栈下 codex 未在自动返修预算内逐字声明下游 WI 所需能力（3.6 轮 a5b06105/6cdc8a7a 的能力覆盖教学后仍复现；08-31 r26/r28 Confirmed 为旧栈证据）。无 in-scope 修复（prompt 教学调整=弱模型加固专项，越出本 change 范围铁律）→不再盲目重跑（否则=变相「限定 N 次重测取最佳」，属用户终裁 C 项）。
- **pi**：rep1 首败（两能力缺口指纹重现）+rep2 复跑 confirmed pass@r1（新计划内容无该缺口，diff 无交集）→rep1 定性=provider 内容方差单例；rep2 为有效 Confirmed 案例。
- pi 时长专项（对照基线 938.04s）：本轮 Confirmed 案例 424.541s（7.09min），首次落入 ≤12min——**pi 时长子项本身达标**；本轮挂起驱动因素不是 pi 时长，是 codex Confirmed 子项。

## §3 解锁/挂起判定（WP1.3）

**判定：挂起（SUSPEND）——任一关键子项（codex 与 pi 各 1 案例到达 Confirmed（2/2））未达成：codex 0/2。**

按计划 Step 6 挂起分支执行：

- 本 Task **不执行三选任何一项**（A=维持门不删 legacy/B=授权登记例外+修订 REQ-WSC-07 门文本放行/C=限定 N 次重测取最佳）——三选问法呈报 controller 转用户终裁；
- **零删除、零门文本修订**：`git status` 无任何 `src/` 变更、无 `openspec/specs/` 变更（本 Task 仅新增 cadence/reports 证据文件）；
- 本 change T2/T4/T5/T6 冻结（WP2/3/5 删除面零动作），T3（DEF-4，独立面）可继续；
- 附带呈报事项：①codex 系统性 contract_gap 失败（§2.4，或需弱模型加固专项/后续专项裁决）；②全量 1 目标 2 例 codegraph 环境漂移红（§2.3）；③campaign 驱动器族未测区核对（campaign-untested-audit.md：1 项标记退役登记）。

## §4 测试面子项命令与退出码（2026-09-19 实测）

| 命令（cargo test --locked <filter>） | 结果 | exit |
|---|---|---|
| work_item_plan_policy | 36 passed/0 failed（含 14-golden 与 tests_evaluate 19 用例） | 0 |
| prompt_contract | 36 passed/0 failed | 0 |
| reviewer_finding_channel | 1 passed/0 failed | 0 |
| planning_resume | 8 passed/0 failed | 0 |
| interrupted_run_recovery | 5 passed/0 failed | 0 |
| campaign_stage3_recovery_matrix | 7 passed/0 failed | 0 |
| prepare_preflight_falls_back_only_before_session | 1 passed/0 failed | 0 |
| single_candidate_preflight_is_deterministic | 1 passed/0 failed | 0 |
| preflight_fails_closed_when_legacy_shared_worktree_present | 1 passed/0 failed | 0 |
| （全量）cargo test --locked | 10 结果块 4334 passed/2 failed/16 ignored（红项 §2.3） | 非 0（红项登记呈报） |
