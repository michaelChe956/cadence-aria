# S3 交付报告:投影真实化 + 逐仓 DTO

- 切片:S3(设计权威 history://DesignPushLocal2 最终输出「Q3/Q5/切片 S3」节)
- 日期:2026-10-14
- 状态:DONE
- 提交:`fix: S3 投影真实化与逐仓DTO`(文件:`src/web/handlers/coding/plan_start.rs`、`src/web/types.rs`、`src/web/handlers/coding/plan_start_tests.rs`、本报告)

## 改动内容

### Q3 投影真实化(plan_start.rs)

1. **provider_started 加 attempt.status==Running 守卫**(build_plan_coding_progress units 循环):原判据「claim 相位 ≥ ProviderMayHaveStarted + active_unit_id 对齐 + unit Running」之上新增 `attempt.status == Running`。AMR/人控/终态现场 unit 仍 Running、claim 仍 ≥Provider,但 attempt 已不在执行,不再谎报 `running`;POST 回执窗口(admission 异步未跨、status 仍 Created)同样回落 `initialized`。
2. **execution_state 新增值域(additive)**:attempt 级屏幕插在 `invalidated` 之后、`waiting_dependency` 之前(优先序:completed > invalidated > awaiting_manual_recovery > failed > aborted > waiting_human > waiting_dependency > running > paused > initialized > waiting_slot):
   - `awaiting_manual_recovery`:`waiting_reason` 原样透传 `attempt.manual_recovery_reason`(开放值集,如 `coding_runner_failed_while_running`);
   - `waiting_human`:Blocked / WaitingForHuman;
   - `failed` / `aborted`:attempt 终态。
   - unit 底层状态与 `start_claim` 一律不改写(两层事实原则)。
   - 未映射状态(Created/AwaitingPlanAmendment/ApplyingPlanAmendment/AmendmentApplyFailed)沿用既有 unit 级投影。

### Q5 逐仓 DTO(types.rs + plan_start.rs)

`PlanCodingTargetDto` 扩九字段,均 `Option<String>`(serde snake_case,additive,`None`=无对应事实):

`repository_name`(checkout 路径末段,`resolve_plan_target_repository_name` 同款解析,回落 id 字符串)、`branch_name`、`base_branch`、`stage`(coding_execution_stage_text 值域)、`head_commit`、`push_status`(push_status_text 值域,含 S2a 已落地的 `skipped_not_configured`)、`push_error`、`review_request_id`、`manual_recovery_reason`。

推送三字段取该 attempt **最新** ReviewRequest(list_review_requests 按 created_at+id 升序末元素,与 plan_group_projection 同口径);成员 attempt 已删(REQ-CS-07 失效窗口)时 attempt 面字段全 None。POST 回执与 GET 共用聚合,两入口同时生效。

## TDD 红绿记录

- **红**(实施前实跑,3 失败):`plan_coding_s3_amr_attempt_projects_recovery_not_provider_running`(得 "running" 期望 "awaiting_manual_recovery")、`plan_coding_s3_created_claim_provider_window_is_not_provider_running`(得 "running" 期望 "initialized")、`plan_coding_s3_attempt_statuses_screen_execution_state`(WaitingForHuman 得 "running" 期望 "waiting_human")。
- **绿**:`cargo test plan_coding_ --lib` → **50 passed; 0 failed**(4 个 S3 新测试 + 46 个既有 n3/h4/n7/s2 族)。
- **既有断言更新(2 处,设计裁决的直接后果)**:`plan_start_tests.rs` one_click 测试 POST 回执断言 `running→initialized`(回执窗口 admission 未跨);deleted-member 测试手工相位推进场景断言 `running→initialized`(attempt 仍 Created)。同条件 attempt Running 仍 `running` 由 AMR 测试的前置断言锁定。
- DTO 测试为实施后验证(字段种群即实现本体,无独立红阶段):`plan_coding_s3_target_attempt_rows_expose_repository_and_review_facts` 断言九字段含 review request 数据(Failed+push_error、最新请求胜出、skipped_not_configured 透传、未启 target null 面、仓名解析 n3-busi/n3-web)。

## 撞面对表

- S2a(未提交 WIP 在 review.rs/dto.rs/issue_delivery 等):我复用其已落地的 `PushStatus::SkippedNotConfigured` 与 `push_status_text`,未触碰其文件。
- FE-1:契约四点已确认(9 字段全 Option\<String\>、四新值 snake_case 字面量、waiting_reason 开放值集透传、skipped_not_configured 序列化名)。
- S4(runner/task 面):无文件交集。

## 遗留与建议

- 仪表盘六态覆盖(coding-dashboard-projection.ts)归 FE-2;role run 收尾归 S4。
- 名称解析逐 target 读 project+repo 记录(与 plan_group_projection 同成本);如 GET 热路径有压力可后续缓存,非本切片范围。
- Main P3 回归建议:`cargo test plan_coding_ --lib`(已过)+ web vitest(FE 合入后)。
