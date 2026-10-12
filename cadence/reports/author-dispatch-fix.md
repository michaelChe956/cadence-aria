# author 族显式 fresh dispatch 臂修复报告（B1→B2 接缝，终审 P1）

- 日期：2026-10-12
- 提交：`d7b75730 fix: author族显式fresh dispatch臂(B1-B2接缝)`
- 范围：worktree `feat-b-0808-add-monorepo`

## 1. 终审定论回顾（证据链已冻结）

1. 生产唯一可达的 author 族等待 = `review/drive.rs` `drive_revision_session` 的
   ReviewerRevision mismatch persist（supersede → waiting）。
2. 点击 → `claim_lc_fresh`（FreshReserved）→ `socket.rs` `dispatch_lc_fresh_recovery_run`。
3. `gateway_start.rs` catch-all 对 Kind(Author/AuthorChoiceFollowup/AuthorFeedback/
   Revision) typed 拒（`lc_resume_recovery_context_unsupported`）→ 回等待。
4. 普通 `StartGeneration` 被 `lc_resume_recovery_active`（lifecycle.rs，非 Resolved
   即 active）拒；`RequestRevision` 被 socket 入站门拒——**owner 主线死循环**。

## 2. 修法（B2 同型，按终审建议）

### 2.1 dispatch 臂（`src/web/workspace_ws_handler/run/gateway_start.rs`）

- `lc_fresh_dispatch_kind` 新增 author 族臂：`Author`/`AuthorChoiceFollowup`/
  `AuthorFeedback`/`ReviewerRevision` 四 tag → 对应 kind
  （`ProviderRunKind::Author{..}`/`AuthorChoiceFollowup{..}`/`Revision`）放行。
- 新增 `verify_lc_fresh_author_family_payload`：business 事实对照（durable typed，
  漂移 → `lc_resume_recovery_stale` 拒回等待，零 spawn）：
  - 身份：entity_id / workspace_type 与 durable record 一致；
  - author/reviewer provider 与 durable record 一致；
  - artifact_refs（version/hash）与 durable artifact 版本一致（未冻结
    version/hash 跳过，不倒算——与 B1b 入场门 ⑦ 同纪律；birth 位冻结空集）；
  - feedback：冻结 `original_feedback` 与原修订节点 durable
    `revision_detail.revision_feedback` 一致（双侧均在场才比对）。
- 4.2 的 OperationResult/claim 链零改动。

### 2.2 review drive 消费点（`src/product/workspace_engine/review/drive.rs`）

- 新增 `lc_fresh_author_family_recovery()`：**durable 侧**判据（FreshRunning +
  author 族 tag）——dispatch CAS 推进的是 durable record，engine 内存镜像可能滞留
  Waiting，以 durable 为准；读取失败按无 fresh 处理（fail-safe 等待方向）。
- `drive_revision_session` fresh 分支：
  - 等待门（A2b）仅在非 fresh 时拦截；
  - 冻结 feedback 原样透传（恢复重连丢失的 `pending_revision_context`，不重算）；
  - 重建全新修订节点（retry-of 失败节点，feedback 落盘节点 detail）+ 阶段回
    Running；
  - `resolve_revision_root_launch_fresh()`：直接 `gateway.validate` 冻结全新
    launch，不进 `resume_or_start` supersede 决策、不查旧 candidate（tombstone
    候选的 FrozenBindingUnavailable 不阻断授权 fresh）；
  - `allow_resume=false` → B1a 合同 `build_revision_input_with_resume(false)`：
    resume=None + full-fresh prompt（artifact/feedback/review 全量自足）。
- 新增 `reconcile_lc_fresh_author_family_run(fresh_node_id)`：驱动结束后对账——
  fresh 节点 Completed → **Resolved**（fresh_native_session_id/fresh_node_id 回填 +
  operation Completed）；其余（Failed/悬置）→ 回 Waiting（operation Failed，结果
  保留）。整记录 CAS single-flight，Conflict 让位，engine 内存镜像同步。

### 2.3 author_root_launch 消费点（`.../provider_drive/author_root_launch.inc.rs`）

- `handle_user_message_from_run` / `handle_author_choice_followup_from_run`：
  FreshRunning author 族判据下驱动结束后对账 recovery（FullConversation prompt
  自足；旧 native 已 tombstone 不回捞）；普通 run 零变化。

## 3. TDD 证据（红→绿）

新文件 `src/web/workspace_ws_handler/tests/author_revision_fresh.rs`（anchor_t19，
B2 同型；`tests.rs` 注册 `#[path]` 模块）：

| 用例 | 红（修复前） | 绿（修复后） |
| --- | --- | --- |
| `anchor_t19_story_revision_fresh_dispatch_resolves_after_completion` | `必须真实 spawn: Elapsed`（catch-all typed 拒零 spawn） | spawn ✓ resume=None ✓ prompt 含上一版产物全文/冻结 feedback/reviewer 意见+摘要 ✓ 完成 → AuthorConfirm ✓ recovery **Resolved** + native/node 回填 + operation Completed ✓ 无 typed 拒帧 ✓ |
| `anchor_t19_design_revision_fresh_dispatch_resolves_after_completion` | 同上 | Design 同型全链 ✓（gateway 从 Story 会话引擎克隆注入，同 issue 唯一 gateway） |
| `anchor_t19_author_family_business_drift_waits_typed_stale` | 得 `lc_resume_recovery_context_unsupported`（≠期望码） | typed `lc_resume_recovery_stale` 拒帧 ✓ recovery 回 Waiting ✓ operation Failed(reason=stale) ✓ 零 spawn ✓ |

## 4. 范围回归（全部绿）

| 目标 | 命令（cwd=worktree 根） | 结果 |
| --- | --- | --- |
| anchor_t 全族 | `cargo test --lib anchor_t` | 75 passed / 0 failed |
| gateway_start | `cargo test --lib gateway_start::` | 18 passed / 0 failed |
| planning_resume | `cargo test --lib planning_resume` | 17 passed / 0 failed |
| author_revision | `cargo test --lib author_revision` | 33 passed / 0 failed |
| ws_handler | `cargo test --lib workspace_ws_handler` | 214 passed / 0 failed |
| （扩散面）workspace_engine | `cargo test --lib workspace_engine` | 1418 passed / 0 failed |

本写面 6 文件 `rustfmt --check` 干净（仓库其余并行波次文件的 fmt 差异未触碰）。

## 5. 既有测试的契约同步更新（2 例，语义保持）

author 族由 typed 拒转为接管后，两个以「author tag 仍 typed 拒」为前提的负例
按原测试意图（非法 tag typed 拒、不回落通用 plan_author）改用仍必然 typed 拒的
非法 flow 形态（`LegacyOutlineAuthor { flow: SingleCandidate }`，kind 映射层
拒收），并同步注释指向 anchor_t19 正路覆盖：

- `anchor_t13_plan_stale_waits_for_explicit_fresh` 步骤 ⑤ 换载 tag；
- `anchor_t16_untaken_tag_still_typed_rejected_no_fallback` seed tag。

## 6. 遗留与边界

- Author/ChoiceFollowup tag 的 fresh spawn 依赖 tombstone 过滤保证 resume=None
  （生产等待出生恒 ReviewerRevision；Author tag 无生产出生位，anchor_t19 未单列
  其正例）。
- `reconcile` 的 store 写失败按 split-run 同惯例 warn（fail-safe：durable 状态
  不变），不自动重试。
- 未跑全量门禁四条（fmt/clippy -D/check/test 全库）——归 Main 统一验证；本写面
  fmt 已干净、触及模块 test 全绿。

## 7. 复审 P2 收口：fresh 驱动前失败对账（第二轮）

- 缺口：fresh 修订链**驱动前**两处 early-return（input 构建 Err、launch 决策
  Err，`review/drive.rs`）未调 reconcile——durable 滞留 FreshRunning，后续 claim
  恒 Busy，无 typed 出口。
- 修法：两处 early-return 前在 fresh 判据下补
  `reconcile_lc_fresh_author_family_run`（fail-safe 回 Waiting + operation
  Failed；fresh 节点由 finish_failed_run 终态化，对账判非 Completed 走失败臂）。
- TDD 红先：`anchor_t19_author_family_fresh_prestart_failure_returns_to_waiting`
  （注入驱动前失败：冻结 original_feedback=None + verdict 缺失 →
  `build_revision_input_with_resume` fail-closed）——红：`当前
  state=FreshRunning`（滞留 10s 轮询超时）；绿：recovery 回 Waiting ✓
  operation Failed ✓ operation_id 清空 ✓ 新 operation `op-t19-prestart-retry`
  再 claim == Claimed（非 Busy/Stale）✓ 零 provider spawn ✓。
- 红例注入点说明：两处 early-return 同缺口同修；红例取 input 构建 Err 臂
  （该夹具下 launch validate Err 无稳定注入面，validate 臂以同判据同 helper
  覆盖）。
- 回归：anchor_t19 4/4、anchor_t 全族 76/76、gateway_start 18/18、
  planning_resume 17/17 全绿；本写面 rustfmt 干净。
- 提交：`fix: author fresh 驱动前失败回等待`。
