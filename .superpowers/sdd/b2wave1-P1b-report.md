# B2 波1 P1 后端段报告（B2W1P1b）

- 状态：DONE
- 分支/基线：`feat-b-0808-add-monorepo` @ `dcd905ca`
- 范围：仅 P1 后端（models/store/handler）。P1 前端段、R3 类型子提交、P2 及以后均未做（链上后续包）。

## 交付内容（REQ-WIGA-02 修订）

1. **plan_approval 冻结**（`src/product/models/automation.rs`）
   - 新增 `PlanApprovalPolicy { Manual(默认), Auto }`；`EnrollmentOptions.plan_approval: PlanApprovalPolicy`，`#[serde(default)]`——旧 JSON/旧 wire 缺字段读 manual，已绑定旧 Interactive session 不改写。
   - `PlanApprovalPolicy::expected_run_policy()`：manual→Interactive、auto→AutoIfValid。
   - `EnrollmentOptions::normalized_for_enrollment_freeze()`：auto 授权冻结时把 `plan_options.require_execution_plan_confirm` 压 false（不论弹窗复选框原值，④）；manual 原样（③）。
2. **store 冻结点**（`src/product/issue_automation_store.rs`）
   - `compare_and_set` 入口对 Enable 统一归一再进幂等比较/CAS：同语义 payload（复选框值漂移）重试仍幂等（①）；幂等/冲突比较经 `EnrollmentOptions` PartialEq 自动含 plan_approval。
3. **binding/rebind 准入**（`src/web/handlers/automation_enrollment.rs`）
   - `post_automation_enrollment_binding` 与 `validate_rebind_binding_target` 由「只认 Interactive」改为「session.run_policy 必须 == enrollment 冻结 plan_approval 的期望值」：auto 冻结放行 AutoIfValid（自动链绑定准入），manual 仍只认 Interactive；auto+Interactive（降级）、manual+AutoIfValid（错绑）一律 409（②）。enabled/精确身份/CAS 互斥等既有门全部保留。

## TDD 红→绿证据（定向命令，均带 --lib、无 -j、`CARGO_TARGET_DIR=/tmp/aria-b2-target`）

- 红（HTTP 运行时，实现前）：`cargo test --locked --lib automation_enrollment`
  - `automation_enrollment_http_put_freezes_auto_plan_approval` FAILED（`options.plan_approval` Null≠"auto"）
  - `automation_enrollment_binding_and_rebind_follow_frozen_plan_approval` FAILED（auto+Interactive 得 200 应 409）
  - 其余 10 既有测全绿（manual/Interactive 回归无漂移）
- 红（store 编译，实现前）：`cargo test --locked --lib issue_automation_store` → E0433/E0609 ×10（缺 `PlanApprovalPolicy`/`plan_approval` 字段）
- 绿（实现后）：`cargo test --locked --lib -- issue_automation_store automation_enrollment` → **32 passed, 0 failed**（含 4 新测：store 冻结归一/旧 JSON manual、HTTP auto 冻结幂等、binding/rebind 冻结策略一致）

## 提交（显式列文件，无 add -A）

1. 主 commit（P1 独占）：`src/product/models/automation.rs`、`src/product/issue_automation_store.rs`、`src/product/issue_automation_store/tests.rs`、`src/web/handlers/automation_enrollment.rs`
2. 机械补丁 commit（清单外 8 文件，IRC 已披露、Main 已批准归属 P1 后端；仅「use 列表加 PlanApprovalPolicy + 字面量补默认 Manual」，语义零变化）：`src/web/handlers/automation_target.rs`（resolve 默认 Manual，GET automation-target 投影不变）、`src/product/coding_attempt_store/admission_lease_tests.rs`、`src/product/workspace_engine/tests/advance_handler.rs`、`src/product/workspace_engine/tests/part_03/part_11.rs`、`src/web/workspace_ws_handler/tests/c1_recovery_surface.rs`、`src/web/plan_confirmed_info_c1_tests.rs`、`src/web/workspace_session/tests/part_02.rs`、`src/web/workspace_session/tests/part_07.rs`
   - ⚠️ 主 commit 单独不可编译（新字段 vs 清单外字面量），两 commit 须相邻合入后整体可编译——铁律「清单外单独 commit」所致，已披露。
3. 报告 commit：本文件。

## 备注/边界

- ④ 的 WI 物化路径：FE automatic 模式只 PUT enrollment（`useIssueLifecycleGeneration.ts:301-323`），plan 由编排器经 enrollment 冻结 options 创建（P2 接手 bound 创建），故冻结点归一即覆盖「auto 授权物化 WI 压 false」；`finalizer.rs:262-265` 复制点不改（P3 回归项）。
- FE 提交 auto：`getAutomationTarget` 的 `resolved_options` 回显后由 P1 前端段附 `plan_approval:"auto"`（服务端不猜）；`AutomationTargetQuery` 未加查询参数（R1 域）。
- fmt：4 个独占文件 `rustfmt --edition 2024 --check` 干净；`automation_target.rs:355` 一处为基线存量漂移（HEAD 同样报告），非本包引入，未动。
- edit 工具对 `part_11.rs` 锚点校验毛刺 2 次，按简报以锚点脚本+git diff 对拍完成并复核。
- 未跑：cargo fmt --check 全量/clippy/check/test 全量、pnpm——按纪律留给 controller 统一验收（§4 四条标准命令）。
