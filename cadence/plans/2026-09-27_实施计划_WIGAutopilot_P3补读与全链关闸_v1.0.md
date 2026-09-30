# WIG Autopilot P3 补读与全链关闸 Implementation Plan

> **完成状态：已实施完成（2026-09-30 打标）：对应 change `work-item-group-autopilot` 已于 2026-09-28 归档（`openspec/changes/archive/2026-09-28-work-item-group-autopilot`），归档报告 `cadence/notes/2026-09-28_归档报告_WIG自动化与引导迭代_v1.0.md`。**

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 完成 `work-item-group-autopilot` 的 tasks.md §4.1–4.3：补回 K=8 外近期 durable 完成、隔离信息与待处理提醒、核实 WIG/Coding 下钻，并用两模式真实链补证 P2 §3.4、关闸 Issue 1 十五条。

**Architecture:** 复用 `/api/issues/{issue_id}/lifecycle` 已有 plan confirmed 与 coding FinalConfirm durable 投影，按 issue 加有界近期完成 envelope；不新增通知表、状态机或第二事实源。观察者把定时器、既有生命周期失效通知和所监视 WS 的状态帧仅用作合并刷新的唤醒，K 外事实从 REST 获取；驾驶舱显式分 `displayItems`、`actionableCount`、`notificationCandidates`，以事实 key 和 `occurred_at` 控制展示/提醒。最后核对原有手工链、单 target enrolled Claude/LC 链和不可逾越的人工门。

**Tech Stack:** Rust 2024、Axum、serde、chrono、既有 JSON durable stores；React、TypeScript、Vitest、Testing Library；无新增外部依赖。

**Spec:** `openspec/changes/work-item-group-autopilot/{proposal.md,design.md,tasks.md,specs/work-item-group-autopilot/spec.md}`；本计划展开 tasks.md §4.1–4.3，且把尚未勾选的 §3.4 的 **LC enrolled 全链真实证据**并入 Task 5，不改 §3.1–3.3 已有契约。参照 `cadence/plans/2026-09-27_实施计划_WIGAutopilot_P2后台coding链_v1.0.md` 的 Task 10 未关闸实况；旧 gateway 失败记录是历史，不是本次前置条件：用户已确认 Claude gateway 可用。

## Global Constraints

- D4/REQ-WIGA-01～04、REQ-ADV-05、REQ-MTG-03：默认 off、持久显式 opt-in、per-attempt 单发、不批量、不动唯一人工门。仅恰一 logical repository/单 target enrolled 允许自动首启；其他目标整体 fail-closed，手动逐 target 仍可用。不能自动批准 plan、回答 choice、执行 compile recovery 或 Final Confirm。
- D5/REQ-WIGA-07：plan key `plan_confirmed:{plan_id}:{compile_id}`，时间取成功 committed compile transaction 的 `committed_at`，仅在 publication+session Confirmed 时有事实；coding key `coding_final_confirm:{attempt_id}:{node_id}`，时间取 FinalConfirm 节点首次 `started_at`，需完整 readiness + WaitingForHuman/FinalConfirm。人工 Completed 更新**同 key**标题「已最终确认」，不是执行结束时冒称「整组已交付」；该措辞只允许现有 `PlanGroupOverall::AllDelivered` 判据。
- 新增近期窗口 `RECENT_COMPLETION_WINDOW_MS = 24 * 60 * 60 * 1000`、info TTL `INFO_TTL_MS = RECENT_COMPLETION_WINDOW_MS`，都是源码常量，不新增设置面。只显示 `0 <= now - occurred_at < INFO_TTL_MS` 的 info；首次成功 hydration（即使空目录）和首次近期补读仅展示，不弹 toast/系统通知；后续新稳定事实 key 在当前客户端观察生命周期至多提醒一次，刷新、重连、相同 key 状态升级不续期或再提醒。浏览器完全关闭期间不承诺 OS 推送，跨设备不声称同步已读。
- K=8 observer 不扩充为无限 socket。完成事实只从 `project_id/issue_id` 定位的 REST durable 列表取得；事件序号、WS 到达顺序、snapshot 写回时间不能当完成事实的排序/TTL 依据。info 不进入标题、favicon、告警条、待处理计数、系统待处理通知、批量确认或危险动作。
- 扩展现有生命周期查询，只限 issue 级有界响应：服务端最近 24h + 至多 32 条/issue；目录遍历复用已有项目/issue 枚举，不以新跨项目未授权聚合路由代替。**存储成本边界：**当前 `CodingAttemptStore::list_attempts_for_issue` 仍遍历 issue 下 JSON 文件，计划不虚称磁盘扫描为 O(32)；有界的是发生时间与返回数量，不建索引/第二事实源。旧客户端调用仍得到原 `plan_confirmed_info`/`coding_final_confirm_info`，新增字段缺省为空。
- 所有测试/编译/重启/Claude 真实链步骤均是**未来执行**；本计划没有运行它们。实施须先 TDD，再定向 smoke、最终统一验证；重启前核对持久业务静默且无活跃 coding run，不在运行中强杀已认领 provider。真实链不得用 Fake、legacy/manual 或 mock 网关顶替。只有真实 LC enrolled 链补齐自动 advance→自动首启→无页面 coding→FinalConfirm 等待→人手 Completed 才可勾 §3.4；未过时如实记录阻断、保持 §3.4/§4.3 留白。

## Review Focus

1. **同 issue 至少 9 个会话且完成在第 9 个、同一时间有不同 key：**仍补读 K 外事实；按 `(occurred_at, kind, key)` 的稳定序与 per-issue limit 取最新事实，不依赖会话或事件顺序（Task 1/2）。
2. **第一次请求返回空、重试、并发请求乱序和中途失效：**成功的空 hydration 也完成静默基线；失败不误记为 hydration；合并唤醒后不让旧 HTTP 结果覆盖新事实，漏 WS 帧由周期补读兜底（Task 2/3）。
3. **TTL 边界、坏时间、同 key waiting→Completed、刷新和卸载重开：**到期不展示/提醒且不续期；坏时间 fail-closed；同 key 文案更新不二次 toast；重开首次加载只展示（Task 1/3）。
4. **跨 project/issue 重名 key、disabled enrollment 和手动 attempt：**不得串身份、借 disabled plan 冒认确认或把手动 attempt 贴 enrolled 完成；现有身份错误显式传播（Task 1/2/4）。
5. **无页面 choice/recovery/amendment/进程重启与人工门：**两模式分别走完整真链；返回码、真实投递、FinalConfirm 前/后状态、首启次数及五红线均有可核对证据（Task 4/5）。

---

## 文件边界与跨任务接口

**已核验：**`src/web/handlers/lifecycle.rs::issue_lifecycle` 在 `support.rs::GateResolveQuery` 读取 `project_id`，先加载 `ProjectStore::get` / `IssueStore::get`，末段并列读取 `issue_plan_confirmed_info` 与 `issue_coding_final_confirm_info`，组装 `types.rs::IssueLifecycleResponse`；前者只对 enabled 精确绑定 enrollment 取成功 publication/compile，后者遍历本 issue 已认领 Enrolled group attempts 的 readiness/timeline。`web/src/hooks/useWorkspaceSessionObservers.ts` 现按 `selectWatchedSessionIds(..., 8)` 对 plan/coding info 做 session 过滤，`CockpitShell.tsx` 按 `countedInbox` 计数/标题/favicon/系统通知、按 `inbox` 做 info toast；`lifecycle-workbench-store.ts::subscribeToLifecycleInvalidation` 已支持同页与跨 tab 通知。

**任务与文件责任：**Task 1 在 `src/web/recent_completion_info.rs` 聚合已有两个 durable info DTO（仅派生、限窗、去重、排序），`handlers/lifecycle.rs`/`support.rs`/`types.rs` 负责 HTTP query/响应；Task 2 在 `web/src/api/{client.ts,types/lifecycle.ts}` 和 `hooks/useWorkspaceSessionObservers.ts` 接 bounded 查询与唤醒，在 `state/recent-completion.ts` 放纯前端稳定身份/时间判据；Task 3 在 `state/workspace-cockpit-projection.ts` 与 `components/cockpit/CockpitShell.tsx` 完成通知与计数分路；Task 4 对现有 WIG/Coding 入口做行为核验，仅有证据表明不符时改 UI；Task 5 只产出真实/替身关闸证据及据实回填 OpenSpec 与 P2 计划。每项只改自己的责任文件，避免借机重构 runner 或加持久通知存储。

**统一接口；新符号按负责 Task 首次定义，后续任务原样消费：**

```rust
// Task 1；新增到既有 src/web/handlers/support.rs::GateResolveQuery
pub recent_since: Option<String>,       // RFC3339；不传则不请求近期目录
pub recent_limit: Option<usize>,        // 默认 32；仅允许 1..=32，单独传 limit 报 422
// Task 1；新增 src/web/recent_completion_info.rs，经 src/web/mod.rs 声明，types.rs 引用
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecentCompletionKind { PlanConfirmed, CodingFinalConfirm }
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RecentCompletionInfoDto {
    pub kind: RecentCompletionKind, pub key: String,
    pub project_id: String, pub issue_id: String, pub plan_id: String,
    pub session_id: Option<String>, pub attempt_id: Option<String>,
    pub occurred_at: String, pub title: String,
    pub final_confirmed: Option<bool>,
}
// plan: session_id=Some，attempt_id/final_confirmed=None；coding: session_id=None，
// attempt_id=Some，final_confirmed=Some。None 在 JSON 为 null；字段均完整输出。
// Task 1；新增到既有 src/web/types.rs::IssueLifecycleResponse
#[serde(default)] pub recent_completion_info: Vec<RecentCompletionInfoDto>,
// Task 1；由 handlers/lifecycle.rs 对已读取的原有两个 DTO 派生，不重扫新表：
// recent_completion_info(project_id, issue_id, &plan_confirmed_info,
//   &coding_final_confirm_info, parsed_since, 32) -> Vec<RecentCompletionInfoDto>。
// parsed_since = max(RFC3339 query, now - 24h)；无 query -> []；旧字段不改变。
```

```ts
// Task 2；web/src/api/types/lifecycle.ts。统一 JSON 带 kind 的判别联合，null 不能省略。
export type RecentCompletionInfoItem =
  | { kind: "plan_confirmed"; key: string; project_id: string; issue_id: string;
      plan_id: string; session_id: string; attempt_id: null; occurred_at: string;
      title: string; final_confirmed: null }
  | { kind: "coding_final_confirm"; key: string; project_id: string; issue_id: string;
      plan_id: string; session_id: null; attempt_id: string; occurred_at: string;
      title: string; final_confirmed: boolean };
// IssueLifecycleResponse 增 recent_completion_info?: RecentCompletionInfoItem[]。
// Task 2；原来的两参数调用 URL 不变；只有观察者传 options 才带 recent_since/recent_limit。
getIssueLifecycle(issueId: string, projectId: string,
  options?: { recentSince: string; recentLimit?: number }): Promise<IssueLifecycleResponse>;
// Task 2；web/src/state/recent-completion.ts，读取正宗 key 不取 item.id/WS seq。
export const RECENT_COMPLETION_WINDOW_MS = 24 * 60 * 60 * 1000;
export const INFO_TTL_MS = RECENT_COMPLETION_WINDOW_MS;
export function recentCompletionIdentity(item: RecentCompletionInfoItem): string;
// JSON.stringify([project_id, issue_id, kind, key]) 避免跨 project/issue/key 分隔符冲突。
export function recentCompletionInboxItem(item: RecentCompletionInfoItem): CockpitInboxItem;
export function isRecentCompletionVisible(occurredAt: string, nowMs: number): boolean;
// Date.parse 有限，0 <= nowMs - Date.parse(occurredAt) < INFO_TTL_MS，未来/坏时间不展示。
// Task 2；在既有 WorkspaceSessionObserverResult 增：
displayItems: readonly CockpitInboxItem[];
actionableCount: number;
notificationCandidates: readonly CockpitInboxItem[];
// 既有 inbox 保留作为 displayItems 别名，countedInbox 保留为 actionable items；
// records/watchedSessionIds/watchSession/codingAttemptForSession 保持不变。
// Task 2；在既有 CockpitInboxItem 增 completionIdentity?: string，仅 recent info 投影填写；Task 3 消费。
```

**身份与排序：**后端只接已存在 `ProjectStore::get`/`IssueStore::get` 校验后的 scope；归一化 coding DTO 必须再次匹配其 `project_id/issue_id`，跨 issue/provider 事实不得混入。比较时间先 RFC3339 解析为 UTC；同 scope/`kind`/`key` 重复时只留当前 durable 投影，coding waiting→Completed 同 key 后优先当前 Completed。先过滤窗口，再按 `(occurred_at DESC, kind DESC, key DESC)` 取**最近 32**；跨 issue 从不共用 32 条配额。前端同一 identity 去重，按同一 durable 顺序展示。query 的无效 RFC3339、超限、零、孤立 limit 给稳定 422：Task 1 必须在现有 `src/web/error.rs::ApiError::into_response` 显式映射新错误码，不可仅调用 `ApiError::validation`（该函数不自行决定 HTTP 状态，未知码默认 500）；坏 durable 时间 fail-closed，不投射假近期成功，并以现有错误方式暴露数据错误。`recent_completion_info` **非通知真相表**，只能消费两个已有 DTO，不用 arbitrary compile-node Completed 或 attempt.completed_at 补数。

### Task 1: 4.1 后端 issue 级有界 durable 完成投影

**Files:**
- Create: `src/web/recent_completion_info.rs`（两个既有 DTO 的归一化、scope/window/limit/排序与定向测试）
- Modify: `src/web/mod.rs`、`src/web/handlers/support.rs`、`src/web/handlers/lifecycle.rs`、`src/web/types.rs`、`src/web/error.rs`（新 query 错误码明确映射 422）
- Test: `src/web/recent_completion_info.rs` 的 `#[cfg(test)]`；`src/web/handlers/lifecycle_tests.inc.rs` 的 Axum 请求测试（复用既有 `EnrolledGateFixture`/真实 `build_web_router`）

**Interfaces:** Consumes `issue_plan_confirmed_info(paths, project_id, issue_id) -> Result<Vec<PlanConfirmedInfoDto>, ProductStoreError>`、`issue_coding_final_confirm_info(paths, project_id, issue_id) -> Result<Vec<CodingFinalConfirmInfoDto>, ProductStoreError>`；Produces 上述 `GateResolveQuery`、`RecentCompletionInfoDto` 与 `IssueLifecycleResponse.recent_completion_info`。现有字段和非补读 URL 原样保持。

- [x] **Step 1: 写失败行为测试。** 用同 issue 已确认 plan + 已认领 FinalConfirm waiting/Completed 的 durable fixture 检查 `recent_since=now-24h&recent_limit=1` 仅返回最新一条，旧于窗口的条目不返回；相同发生时刻以 kind/key 稳定择优，多次 GET key/time 不漂；人工确认后同 key/title 更新，disabled enrollment 不重现 plan，Manual attempt 不冒 enrolled。通过另一 project/issue 的已落盘事实及错误 project/issue query 断言响应不串授权；无 query 时旧两个 info 数组仍在，`recent_completion_info=[]`。边界请求：limit=0/33、孤立 limit、非法时间均 422，读取损坏的 scope 信息不可静默装成成功。样例断言：

  ```rust
  assert_eq!(bounded[0]["kind"], "coding_final_confirm");
  assert_eq!(bounded[0]["attempt_id"], attempt_id);
  assert_eq!(bounded[0]["occurred_at"], original_final_confirm_started_at);
  assert_eq!(unbounded["recent_completion_info"], serde_json::json!([]));
  assert_eq!(bad_limit.status(), StatusCode::UNPROCESSABLE_ENTITY);
  ```

- [x] **Step 2: 红灯。** 在 worktree 根运行 `cargo test --locked --lib recent_completion_info -- --nocapture` 与 `cargo test --locked --lib issue_lifecycle_recent_completion -- --nocapture`；应失败于缺少 bounded 字段/断言，而非测试本身编译错误。
- [x] **Step 3: 最小实现。** 在既有 handler 同一个 issue 的 plan/coding 投影之后构造归一化 envelope：

  ```rust
  let recent_completion_info = match query.recent_since.as_deref() {
      None if query.recent_limit.is_none() => Vec::new(),
      None => return Err(ApiError::validation("recent_since_required", "recent_since is required")),
      Some(raw) => {
          let since = chrono::DateTime::parse_from_rfc3339(raw)
              .map_err(|_| ApiError::validation("invalid_recent_since", "recent_since must be RFC3339"))?;
          let limit = query.recent_limit.unwrap_or(32);
          if !(1..=32).contains(&limit) {
              return Err(ApiError::validation("invalid_recent_limit", "recent_limit must be 1..=32"));
          }
          let since = since.with_timezone(&chrono::Utc)
              .max(chrono::Utc::now() - chrono::Duration::hours(24));
          crate::web::recent_completion_info::recent_completion_info(
              &project_id, &issue_id, &plan_confirmed_info,
              &coding_final_confirm_info, since, limit,
          ).map_err(product_store_api_error)?
      }
  };
  ```

  `recent_completion_info` 签名与返回在本 Task 定义为 `fn recent_completion_info(project_id: &str, issue_id: &str, plan: &[PlanConfirmedInfoDto], coding: &[CodingFinalConfirmInfoDto], since: chrono::DateTime<chrono::Utc>, limit: usize) -> Result<Vec<RecentCompletionInfoDto>, ProductStoreError>`。在 `types.rs` 增字段、`mod.rs` 声明模块，`error.rs::ApiError::into_response` 将 `recent_since_required` / `invalid_recent_since` / `invalid_recent_limit` 映射 `StatusCode::UNPROCESSABLE_ENTITY`；严格保持 `issue_plan_confirmed_info` 的 enabled 精确绑定与 coding `start_claim.origin=Enrolled` 判据，别重新写一套事实/授权判定。服务端以请求时钟过滤未来发生时间；RFC3339 格式错误的 durable 条目走 `ProductStoreError` 显式 HTTP 错误，不跳过其余事实假称完整。
- [x] **Step 4: 绿灯。** 重跑 Step 2 两条命令，并运行 `cargo test --locked --lib plan_confirmed_info -- --nocapture` 与 `cargo test --locked --lib coding_final_confirm_info -- --nocapture`；检查同一次 HTTP GET 的两个旧字段未变化。定向启动测试 Router 的 Axum 请求就是该接口的无网络 smoke。
- [x] **Step 5: 提交。** `git add src/web/recent_completion_info.rs src/web/mod.rs src/web/handlers/support.rs src/web/handlers/lifecycle.rs src/web/types.rs src/web/error.rs src/web/handlers/lifecycle_tests.inc.rs && git commit -m "feat: expose bounded durable WIG completion catalog"`。

### Task 2: 4.1 observer 的 K 外补读与合并刷新

**Files:**
- Create: `web/src/state/recent-completion.ts`、`web/src/state/recent-completion.test.ts`
- Modify: `web/src/api/client.ts`、`web/src/api/types/lifecycle.ts`、`web/src/hooks/useWorkspaceSessionObservers.ts`、`web/src/state/workspace-observer-store.ts`（仅 WS snapshot hint 接口；不改只读权限）、`web/src/state/workspace-cockpit-projection.ts`（只加 recent info 身份字段）
- Test: `web/src/api/client.test.ts`、`web/src/hooks/useWorkspaceSessionObservers.test.tsx`、`web/src/state/workspace-observer-store.test.ts`；另同步更新已手写完整 observer 返回值的 `web/src/{router.test.tsx,pages/ChatWorkspacePage.test.tsx,pages/CodingWorkspacePage.pending-choice.test.tsx,components/cockpit/CockpitShell.test.tsx}` fixture

**Interfaces:** Consumes Task 1 的 `recent_completion_info` 判别联合，既有 `subscribeToLifecycleInvalidation(listener): () => void`、`selectWatchedSessionIds(sessions, watchLimit)` 与 `createObserverController`；Produces 统一接口节所列 `getIssueLifecycle(..., options?)`、`recentCompletionIdentity` / `recentCompletionInboxItem` / `isRecentCompletionVisible`、`WorkspaceSessionObserverResult` 三分字段。对既有测试注入的两参数 `getIssueLifecycle(issueId, projectId)`，第三可选参数保持兼容。

  `WorkspaceObserverControllerOptions` 新增 `onSnapshotHint?: () => void`，由 `createObserverController` 的 `onSnapshot` 在 durable 状态帧到达后触发；`WorkspaceObserverControllerFactory` 当前是单参数类型——本任务需将其签名扩展为可选第三实参 `onSnapshotHint?: () => void`（同一回调形态，类型定义与全部注入点同步改），hook 测试可从注入 factory 主动触发；未提供 callback 时完全保留旧行为。不要将「收到 WS 事件」当完成事实，回调只调度 catalog GET。

- [x] **Step 1: 写失败行为测试。** `useWorkspaceSessionObservers.test.tsx` 构造 9 个可观察 session、`watchLimit=8`，将第 9 个所属 issue 的 `recent_completion_info` 注入 plan 与 coding 事实：断言第 9 个没有 observer socket、完成信息仍进 `displayItems`，且 `actionableCount=0`；另造两个 project 下相同 key 和同 issue 双次返回 key，断言按 `[project_id,issue_id,kind,key]` 去重、两个 project 不合并。模拟首次空成功、重复 invalidation、WS snapshot 与定时轮询重叠、漏事件/重连和 HTTP 旧响应晚到：持续只展示最新成功 durable 投影，fetch 失败不清空已知目录、也不设置首次成功标志；若初始 GET 在收到 invalidation 时仍未完成，新事实须归入首次补读静默批次，随后的 refresh 才开始产生候选。`client.test.ts` 验证未传 options 完全保持老 URL，传 options 仅编码 `recent_since`/`recent_limit`；`recent-completion.test.ts` 测坏时间/未来时刻不显示、RFC3339 时区相同瞬时归一比较。测试须断言渲染/返回事实，不断言 fetch 次数等 incidental 排程细节。
- [x] **Step 2: 红灯。** `pnpm -C web exec vitest run src/state/recent-completion.test.ts src/api/client.test.ts src/hooks/useWorkspaceSessionObservers.test.tsx src/state/workspace-observer-store.test.ts`；预期 K 外 info 当前被 watched filter 丢弃、失效通知不触发补读。新建测试文件先写有意义的业务断言，再运行红灯；不要把模块不存在或语法错误当作行为失败。
- [x] **Step 3: 最小实现。** 每次 observer catalog 请求携带 `{ recentSince: new Date(now - RECENT_COMPLETION_WINDOW_MS).toISOString(), recentLimit: 32 }`；继续遍历原 `listProjects`/`listProductIssues`，对每 issue 读 `recent_completion_info ?? []`，按完整 project/issue/kind/key 做归一化（不信任错 scope 字段），新服务端以此为主；兼容旧响应时才从原 plan/coding info 仅构造既有 watched 条目，绝不把旧响应认作 K 外全量。`createObserverController` 在收到业务 snapshot（非 ping/pong）的现有 `onSnapshot` 回调旁增加可选 `onSnapshotHint: () => void`，通过现有 options 传入 hook（测试注入的 `createController` factory 相应支持可选 hint），供 hook 排队刷新；不要在 `onFrame` 的 ping/pong 上每帧遍历目录，不改 `event_seq` replay 语义。订阅原 `subscribeToLifecycleInvalidation`（含 BroadcastChannel），连同轮询和 snapshot hint 汇入约 250 ms trailing debounce；同时只允一轮 catalog 请求，期间再收 hint 标 dirty，结束后补跑一次；effect 清理取消定时器、忽略卸载后的响应，不能让旧 HTTP 响应覆盖晚到的新请求。首次成功完成整个目录（包括空结果）后标记 `catalogHydrated`，且对首次 GET 与其在途期间积累的 dirty 补读统一静默建基线；只有此后又出现的新事实才进入 `notificationCandidates`。失败保留旧事实与旧标记。完成数组按 durable 时间/稳定 key 排序，不按 SSE/WS 或 session 数组顺序。

  ```ts
  // 请求三参兼容；URLSearchParams 在传 options 时追加相同 query 键。
  getIssueLifecycle(issue.issue_id, projectId, {
    recentSince: new Date(Date.now() - RECENT_COMPLETION_WINDOW_MS).toISOString(),
    recentLimit: 32,
  });
  // onSnapshotHint/subscribeToLifecycleInvalidation/periodicTimer -> queueCatalogRefresh()
  // inFlight 时 dirty=true；settled 后只以新一轮 GET 校正被唤醒期间的事实。
  ```

  去重纯函数在 `recent-completion.ts` 复用原 `planConfirmedInfoItem`/`codingFinalConfirmInfoItem`，保留原 info 文案/下钻；从 Task 1 返回的完整 scope/kind/key 在 `workspace-cockpit-projection.ts::CockpitInboxItem` 增 `completionIdentity?: string`，归一化 info 条目由本任务赋值，不能以 `${session_id}:info:${key}` 充当跨 project 身份。本任务先返回 `displayItems`、`actionableCount` 与首次 hydration 后的事实候选；Task 3 再绑定 TTL/once/提示队列，但接口/字段此时应可被 TypeScript 编译。仍需在本任务更新所有手写 `WorkspaceSessionObserverResult` 测试 fixture（已知 `router.test.tsx`、`ChatWorkspacePage.test.tsx`、`CodingWorkspacePage.pending-choice.test.tsx`、`CockpitShell.test.tsx`）增加新增字段，避免仅局部测试可编译。
- [x] **Step 4: 绿灯。** 重跑 Step 2 的聚焦命令，加 `pnpm -C web exec tsc -b`（仓库现有 build 模式）；用已有 lifecycle HTTP 测试的 JSON payload 作为前端查询/hook 输入，K 外 item 可见且 socket 数仍受 K 约束。不可将单元 mock 当作真实服务器已经部署。
- [x] **Step 5: 提交。** `git add web/src/api/client.ts web/src/api/client.test.ts web/src/api/types/lifecycle.ts web/src/hooks/useWorkspaceSessionObservers.ts web/src/hooks/useWorkspaceSessionObservers.test.tsx web/src/state/recent-completion.ts web/src/state/recent-completion.test.ts web/src/state/workspace-observer-store.ts web/src/state/workspace-observer-store.test.ts web/src/state/workspace-cockpit-projection.ts web/src/router.test.tsx web/src/pages/ChatWorkspacePage.test.tsx web/src/pages/CodingWorkspacePage.pending-choice.test.tsx web/src/components/cockpit/CockpitShell.test.tsx && git commit -m "feat: reconcile recent WIG completions outside observer window"`。

### Task 3: 4.1 驾驶舱展示、计数与通知候选三分

**Files:**
- Modify: `web/src/state/recent-completion.ts`、`web/src/hooks/useWorkspaceSessionObservers.ts`、`web/src/components/cockpit/CockpitShell.tsx`；`web/src/state/workspace-cockpit-projection.ts` 的 info 投影若已有 Task 2 identity 不正确，仅在此修正
- Test: `web/src/state/recent-completion.test.ts`、`web/src/hooks/useWorkspaceSessionObservers.test.tsx`、`web/src/components/cockpit/CockpitShell.test.tsx`、`web/src/pages/ChatCockpitPage.inbox.test.tsx`

**Interfaces:** Consumes Task 2 的 `displayItems`/`actionableCount`/`notificationCandidates`、`RecentCompletionInfoItem` / `recentCompletionIdentity` / `INFO_TTL_MS` / `CockpitInboxItem.completionIdentity`；Produces TTL 内 `inbox === displayItems` 和 `countedInbox === actionable items` 的兼容语义、单客户端后续新 key 一次提醒；既有 `CockpitInbox` 的 `onOpenInfoSession`/`onOpenInfoCoding` 按钮与 `isSelectableGate` 不改危险动作权限。

- [x] **Step 1: 写失败行为测试。** 冻结时钟，分别给出发生在 `now-INFO_TTL_MS+1`、`now-INFO_TTL_MS`、未来和非法时刻的 info，断言只有未过期项在收件箱、到期定时消失且刷新/重连不能延寿；第一次**空**成功后新 key 提示一次，第一次非空/首次 K 外补读只展示，后续重复 GET、乱序、same key `final_confirmed=false→true` 只更新标题不再 toast，组件卸载重开首次 hydration 不重弹。测试 title/favicon/告警条/系统通知仍只随可操作 gate/choice/stopped/error/sc_failed 变化，info 计数恒零，info 无批量勾选、不可走危险动作；有多条后续新 key 时按顺序逐条提醒，不因同时存在 gate toast 永远丢失 info，反复提示仍至多一次/键。
- [x] **Step 2: 红灯。** `pnpm -C web exec vitest run src/state/recent-completion.test.ts src/hooks/useWorkspaceSessionObservers.test.tsx src/components/cockpit/CockpitShell.test.tsx src/pages/ChatCockpitPage.inbox.test.tsx`；预期目前 info 无 TTL，首次空 hydration 后的首个新事实被误静默。
- [x] **Step 3: 最小实现。** 将服务端窗口事实先按 `isRecentCompletionVisible(info.occurred_at, Date.now())` 过滤；`displayItems = [...actionableItems, ...sortedVisibleInfo]`，`actionableCount = actionableItems.length`，`notificationCandidates` 只接首次成功 catalog 后新增的、尚在 TTL 内的稳定身份 info，不把 gate/stopped/error/choice/sc_failed 从原有 alert/toast/system notification 管道搬过来。observer 用最近事实集合的完整 scope/key 做 dedup，`CockpitShell` 以 `completionIdentity` 而非容易跨 project 碰撞的 `item.id` 跟踪已提醒集合；首次成功 hydration 在记录**空集合**时也设置基线，失败不置基线，后续 status 更新保留原 key。info toast 队列每 key 最多一次，只有可见到期时间内出队；当前 gate toast 先显示但 info 留队待显示。基于最近到期时刻安排一次失效定时器，使页面不刷新也能移除过期 info；卸载清理定时器。用 `actionableCount` 控告警条/标题/favicon，以 `countedInbox` 原有 actionable 列表控制去处理和系统通知，context 的 `inbox` 明确就是 `displayItems`；`CockpitInbox` 的纯 info 卡仍不可选。

  ```ts
  const identity = recentCompletionIdentity(info); // JSON.stringify([scope, kind, key])
  const ageMs = nowMs - Date.parse(info.occurred_at);
  const visible = Number.isFinite(ageMs) && ageMs >= 0 && ageMs < INFO_TTL_MS;
  // 同一 key waiting→Completed：重新投影文案，knownIdentity.has(identity) 则无新 toast。
  // title/favicon/Notification("aria：需要处理") 只看 actionableCount/countable items。
  ```

- [x] **Step 4: 绿灯与界面 smoke。** 重跑 Step 2 聚焦测试；使用既有 ChatCockpitPage 渲染 fixture 点「查看 Coding Workspace」与「查看 Plan 会话」，确认 info 可见和下钻可达、没有 info 批量动作；真实浏览器可用时打开驾驶舱核对 badge/favicon/信息条，不可用时如实标明视觉未检查，并使用该 UI 测试的 DOM 结果作有限替代。到期后即使不触发 REST 也不可留旧 info。
- [x] **Step 5: 提交。** `git add web/src/state/recent-completion.ts web/src/state/recent-completion.test.ts web/src/hooks/useWorkspaceSessionObservers.ts web/src/hooks/useWorkspaceSessionObservers.test.tsx web/src/state/workspace-cockpit-projection.ts web/src/components/cockpit/CockpitShell.tsx web/src/components/cockpit/CockpitShell.test.tsx web/src/pages/ChatCockpitPage.inbox.test.tsx && git commit -m "feat: separate cockpit completion display and actionable alerts"`。

### Task 4: 4.2 WIG/Coding 真进度下钻、人工终态文案与手工对照

**Files:**
- Modify only on observed mismatch: `web/src/components/lifecycle/IssueLifecycleWorkbench.tsx`、`web/src/components/lifecycle/IssueLifecycleWorkbenchDrawer.tsx`（既有 confirmed 门）、`web/src/components/lifecycle/IssueLifecycleWorkbenchParts.tsx`（既有 `toDrawerEntity`）、`web/src/components/lifecycle/LifecycleCardDrawer.tsx`、`web/src/components/lifecycle/PlanGroupProjectionPanel.tsx`、`web/src/components/lifecycle/StageStepper.tsx`、`web/src/components/lifecycle/LifecycleCard.tsx`、`web/src/pages/CodingWorkspacePage.tsx`、`web/src/pages/CodingWorkspaceControls.tsx`（实际 `ActionButtons` 定义处）、`web/src/pages/CodingWorkspaceGroupProgress.tsx`、`web/src/components/coding-workspace/GroupFinalReadinessPanel.tsx`、`web/src/pages/CodingWorkspaceReports.tsx`；必要时 `web/src/hooks/useCodingWorkspaceWs.ts`（只修重放/下钻，不重写 runner）
- Test: `web/src/components/lifecycle/IssueLifecycleWorkbench.drawer.test.tsx`、`web/src/components/lifecycle/IssueLifecycleWorkbench.single-coding.test.tsx`、`web/src/components/lifecycle/LifecycleCardDrawer.test.tsx`、`web/src/components/lifecycle/PlanGroupProjectionPanel.test.tsx`、`web/src/pages/CodingWorkspacePage.test.tsx`、`web/src/pages/CodingWorkspacePage.pending-choice.test.tsx`、`web/src/router.test.tsx`、`web/src/pages/ChatCockpitPage.inbox.test.tsx`

**Interfaces:** 消费既有 `/workbench/projects/$projectId/issues/$issueId/coding/$attemptId` route、`IssueLifecycleWorkbench::handleOpenCodingWorkspaceFromDrawer`、`IssueLifecycleWorkbenchDrawer` 的 confirmed 启用条件、`toDrawerEntity`/`PlanGroupProjectionPanel`、`CodingWorkspaceGroupProgress`、`CodingWorkspaceControls::ActionButtons`、`useCodingWorkspaceWs` 的 `coding_hello.last_seen_node_id`/`finalConfirm()`；不新增自动执行服务，也不将 `CodingWorkspaceReports::PrepareExecutionPlanPanel.handleConfirm` 误作最终确认。确认 WIG 抽屉目前只显示 group projection/子项，而逐 unit 数量条在 Coding Workspace；若用户需要 WIG 入口直接展示 unit 进度，优先消费现有 plan `group_projection` 中每 target attempt 状态，不另开第二取数面或把 `CodingAttempt`（不含 units）当进度事实。

- [x] **Step 1: 写失败优先的用户行为对照。** 借现有 `IssueLifecycleWorkbench.drawer.test.tsx` 的 `lifecycleFetch` 构造后台 enrolled 单 target：不用先打开 coding 页，WIG card/drawer 展示现有 `group_projection` 中 plan 身份、当前 target/attempt 状态，点既有按钮导航同 project/issue/attempt；Coding Workspace 内 `CodingWorkspaceGroupProgress` 展示已载入单位进度，断线重进通过 `coding_hello.last_seen_node_id` 续读完整 timeline，实时/历史重放可见且不触发第二个 start。readiness Complete 且 `waiting_for_human/final_confirm` 时明确「编码执行完成，待最终确认」并有人手确认按钮；用户点击之后 `completed` 才显示「已最终确认」，不以 unit 全完成误报「整组已交付」。另将同样 fixture 换为默认 off 的手工单 target 与手工多 target，验证人工 choice/plan 批准、compile recovery、advance/StartCoding 和错误码行为未变；多 target enrolled 不展示自动首启成功、无 sibling 自动 claim。测试针对实际按钮、路由、timeline 和人工动作；对本已正确的文案/行为保留测试，不制造必然失败的断言。
- [x] **Step 2: 红灯或证实无需修改。** `pnpm -C web exec vitest run src/components/lifecycle/IssueLifecycleWorkbench.drawer.test.tsx src/components/lifecycle/IssueLifecycleWorkbench.single-coding.test.tsx src/components/lifecycle/LifecycleCardDrawer.test.tsx src/components/lifecycle/PlanGroupProjectionPanel.test.tsx src/pages/CodingWorkspacePage.test.tsx src/pages/CodingWorkspacePage.pending-choice.test.tsx src/router.test.tsx src/pages/ChatCockpitPage.inbox.test.tsx`；新失败先定位到可观察错位，不因未创建 fixture helper 将编译错误当业务红灯；若全绿且人工检查上下文无错，保留既有生产代码。
- [x] **Step 3: 只修真实不符的入口/文案。** WIG 抽屉沿 `toDrawerEntity`→`PlanGroupProjectionPanel` 读取现有 plan/group projection（不从 toast/缺 units 的 `CodingAttempt` 猜“已交付”），保留正确 attempt 地址；实时/重放复用现有 Coding Workspace WS hook。`web/src/pages/CodingWorkspaceControls.tsx::ActionButtons` 中的最终确认按钮只在 `waiting_for_human` + `final_confirm` + 完整 readiness 无诊断时调用既有 `api.finalConfirm()`；`CodingWorkspacePage.tsx` 快捷键同样只在人手按键时触发，其他按钮继续留给原手工工作流。`CodingWorkspacePage` 现有 completed banner 「组级 Coding Workspace 已完成」不得提前出现在等待态；若需「整组已交付」，必须消费现有 `PlanGroupOverall::AllDelivered` 后才展示。手动链不能因 enrolled 增加新的按钮或改变 gate 语义。

  ```tsx
  // web/src/pages/CodingWorkspaceControls.tsx：按钮只由人工点击，现有条件保留。
  if (stage === "final_confirm" && status === "waiting_for_human") {
    const ready = groupFinalReadinessStatus === "complete" &&
      groupFinalReadinessDiagnostics.length === 0;
    return <button type="button" disabled={!ready} onClick={api.finalConfirm}>确认完成</button>;
  }
  ```

- [x] **Step 4: 绿灯与表面 smoke。** 重跑 Step 2，用 UI fixture 真正点 WIG→Coding→重连/重放→人工 Final Confirm，并核对等待/完成两个文案与手工多 target；有浏览器时打开同版 Web 页面核对入口、进度、按钮上下文和信息条，不可用时记下视觉限制、不假称已视觉验收。Rust 的 fail-closed/单发真实契约留 Task 5 汇总，不用 UI mock 证明 runner。
- [x] **Step 5: 提交。** 仅暂存本任务实际变化的上述 UI/测试文件：`git add web/src/components/lifecycle/IssueLifecycleWorkbench.tsx web/src/components/lifecycle/IssueLifecycleWorkbenchDrawer.tsx web/src/components/lifecycle/IssueLifecycleWorkbenchParts.tsx web/src/components/lifecycle/LifecycleCardDrawer.tsx web/src/components/lifecycle/PlanGroupProjectionPanel.tsx web/src/components/lifecycle/StageStepper.tsx web/src/components/lifecycle/LifecycleCard.tsx web/src/components/lifecycle/IssueLifecycleWorkbench.drawer.test.tsx web/src/components/lifecycle/IssueLifecycleWorkbench.single-coding.test.tsx web/src/components/lifecycle/LifecycleCardDrawer.test.tsx web/src/components/lifecycle/PlanGroupProjectionPanel.test.tsx web/src/pages/CodingWorkspacePage.tsx web/src/pages/CodingWorkspaceControls.tsx web/src/pages/CodingWorkspaceGroupProgress.tsx web/src/components/coding-workspace/GroupFinalReadinessPanel.tsx web/src/pages/CodingWorkspaceReports.tsx web/src/hooks/useCodingWorkspaceWs.ts web/src/pages/CodingWorkspacePage.test.tsx web/src/pages/CodingWorkspacePage.pending-choice.test.tsx web/src/router.test.tsx web/src/pages/ChatCockpitPage.inbox.test.tsx && git commit -m "test: verify WIG coding drill-down and manual final-confirm copy"`；若无需生产改动，提交新增的行为测试即可，不为提交而改 UI。

### Task 5: 4.3 + 3.4 替身与两模式真实链关闸

**Files:**
- Create after actual runs: `cadence/reports/2026-09-27_进度报告_WIGAutopilot_P3全链关闸_v1.0.md`（两模式双清单、Issue 1 十五行对账、精确日志/截图引用）
- Modify only after corresponding real evidence: `openspec/changes/work-item-group-autopilot/tasks.md`（§3.4、§4.1–4.3 对应证据/勾选）、`cadence/plans/2026-09-27_实施计划_WIGAutopilot_P2后台coding链_v1.0.md`（Task 10 的 LC 补证记录）、本计划 Task 5 的实测记录
- Test: Task 1–4 定向用例、既有 `src/web/wiga_gate_fixture/p2_campaign.rs` 与 `src/web/coding_ws_handler/tests/{runner_recovery.rs,sc_start_guard.rs}`、`src/web/handlers/coding_choice.rs`、`web/src/hooks/useWorkspaceSessionObservers.test.tsx`、`web/src/components/cockpit/CockpitShell.test.tsx`；若某一条五红线或 K 外场景缺行为测试，只在该责任测试文件补齐后再跑。任务结束由主控统一跑项目级构建/测试。

**Interfaces:** 不新增生产接口；消费 Tasks 1–4 的 REST catalog、Cockpit 三分契约、现有 enrollment/choice/advance/StartCoding/Final Confirm API 和 `EnrolledGateFixture` 的已存在 campaign。只从可核对的服务器 revision、durable 记录、真实 provider 日志、UI 动作结果回填状态。已有 §3.1–3.3 勾选维持，§3.4 要求独立 LC enrolled 真实证据，不从 Fake campaign 推断。

- [x] **Step 1: 写最后的可证伪清单/行为测试。** 在本任务报告先列出**替身/自动化**每一行待验：重复 enroll/plan/attempt/key、WS 乱序与重复 frame、丢唤醒由周期补读、K=8 外最近 24h、首次空/非空 hydration、TTL 精确边界及同 key Completed、跨 project/issue 授权隔离、disabled/manual 不冒认、REST choice 完整 answers 的 200/202/409/410/404 与真 Delivered、provider 首启 claim=1、关闭/重开+journal/barrier、零 socket amendment Unsent→真实 Delivered、慢 observer、手动多 target 与自动多 target 禁令。对五红线逐项写**反例**：无 opt-in 不自动、默认 off、手动/后台并发一个 attempt 只一 start、multi-target 零 sibling 自动 claim、plan approve/compile recovery/Final Confirm 必须人手；缺任一对应断言才添加一条真正业务行为测试，不能用代码字符串搜索代替。
- [x] **Step 2: 定向红灯并识别缺口。** 分别运行 `cargo test --locked --lib recent_completion_info -- --nocapture`、`cargo test --locked --lib p2_campaign -- --nocapture`、`cargo test --locked --lib coding_start -- --nocapture`、`cargo test --locked --lib coding_choice -- --nocapture`、`pnpm -C web exec vitest run src/hooks/useWorkspaceSessionObservers.test.tsx src/components/cockpit/CockpitShell.test.tsx src/components/lifecycle/IssueLifecycleWorkbench.drawer.test.tsx src/pages/CodingWorkspacePage.test.tsx`；确实缺的验收用例先观察失败再修。复用已有绿灯仅作为替身证据，**不**替代真链；若缺口超出既有 design/验收，先停止越界修改并按 OpenSpec 更新契约。
- [x] **Step 3: 最小闭环与版本部署。** 只修 Task 1–4 揭露的对应产品断点并复跑该用例；收集命令、服务器 Git revision、构建静态产物 hash 与 pid/exe hash。由主控统一执行 `pnpm -C web build`、`cargo build --locked` 与完整定向/全量测试；启动同版服务前确认 durable 静默/零活跃 coding run；若过程安排在 Task 5 的人工门重启检查点，须再确认当前确实无活跃 provider，不能中断已认领真实 run。用同版构建启动现有 aria 服务，核对 `curl -s http://127.0.0.1:4317/api/health`、`pgrep -x aria`、`md5sum /proc/<pid>/exe`、`curl -s http://127.0.0.1:4317/` 的 `index-*.js` 资源身份。用户已确认 Claude gateway 可用，直接跑 LC 真实链，不以过去 503 或 legacy 失败待机，也不静默切 Fake/manual 兜底。
- [x] **Step 4: 分开跑两条真实 provider/人工链并逐条记账。** **手工模式**：新建未 enrolled issue，人工 story→design、选择“不自动化”、人工 plan 生成与批准、compile/recovery、人工 advance→StartCoding、coding choice、人手 Final Confirm→Completed，保留已有门/错误/恢复语义；手工多 target 分支可逐 target 手动，不被自动 fan-out。**自动模式（P2 §3.4 的唯一补证载体）**：新建真实 LC 单 logical repository/单 target issue，人工 story/design 确认后持久 opt-in（精确 source 版本/唯一 target）→无页面唯一 plan prepare/后台生成→驾驶舱人手处理 plan choice、门与 compile recovery/approve→成功 publication/Confirmed 才有 plan info→**不打开 Coding Workspace**后台独立 advance→Ready→自动单发首启→真实 Claude coding 后台运行→至少一次 coding choice 的 REST 完整答案与真实回执、一次人工确认 amendment 在无 coding socket 下应用并于重连后真投递→完整 readiness + FinalConfirm 等待/info 出现（0 待处理增量）→人手点击 Final Confirm→durable Completed/同 key 文案更新，禁止自动代点。**重启检查点**：在 plan 已确认且无活跃 coding run/等待人工门时重启一次，随后无人打开 Coding Workspace 完成自动 advance/首启；若检验已认领 run 的恢复，必须另选既有 runner 的可安全中断检查点/自然终止后的进程重启，不得为留证强停活跃真实 provider 或造第二首启。两个模式均关闭 driver/observer 与 Coding 页面至少一次，再重开核对历史重放；分别留 `project_id/issue_id`、provider/gateway 身份、enrollment+revision、plan/session/compile/attempt/claim/runner/timeline、HTTP 返回码、快照、`occurred_at`/key、toast/标题/favicon/计数/下钻、人工动作时间与服务版本的可定位证据，避免把 plan confirmed 当 coding 完成。
- [x] **Step 5: 绿灯、十五条映射与据实提交。** 实测后对照下表 Issue 1 #1–#15 按行填入两模式中的有效证据、PASS/FAIL/BLOCKED 和可重放日志/截图引用；替身与真实清单**独立**，任一 required real row 不通过则保持 §3.4/§4.3 未勾、记录精确阻断；§4.1/§4.2 也只能按实际验证据实回填。全部条件成立后勾选 §3.4、§4.1–4.3，并据实更新 P2 计划 Task 10 与新报告。`git add cadence/reports/2026-09-27_进度报告_WIGAutopilot_P3全链关闸_v1.0.md openspec/changes/work-item-group-autopilot/tasks.md cadence/plans/2026-09-27_实施计划_WIGAutopilot_P2后台coding链_v1.0.md cadence/plans/2026-09-27_实施计划_WIGAutopilot_P3补读与全链关闸_v1.0.md && git commit -m "test: gate WIG autopilot P3 and LC enrolled chains"`；测试源文件若本任务确有改动，按实际文件加入。提交前删掉所有临时 smoke 脚本、测试载体地址中的真实凭据；不存在实际通过记录时绝不勾选。

---

## 需求追溯与双清单核对

| 契约 | 任务/可观察证据 |
|---|---|
| §4.1 / REQ-WIGA-07 / D5 | Task 1 请求限窗/限 32、原成功判据与授权不变；Task 2 K 外刷新、乱序/漏唤醒；Task 3 24h TTL、首读静默、稳定 key 一次提醒、info 0 待处理/不可批量。 |
| §4.2 / REQ-WIGA-08、REQ-MTG-03 | Task 4 WIG 进度→Coding 路由、实时/重放、人手 Final Confirm 两态、默认 off 与手动多 target 对照。 |
| §4.3 / §3.4 / REQ-WIGA-01～08、REQ-CG-04、REQ-ADV-05、REQ-MTG-03 | Task 5 替身反例 + 两种完整真实 provider/人工链，P2 LC enrolled 自动 advance/首启/无页面 coding/FinalConfirm 人手完成独立补证。 |

| Issue 1 | 条款 | 负责验证及必须留下的证据 |
|---|---|---|
| #1 人工 story→design | REQ-WIGA-01、08 | Task 4/5 两链 story/design 人手操作、确认时间。 |
| #2 design 后模式选择 | REQ-WIGA-01 | Task 4/5 确认前无选项/默认 off 与确认后精确 enrollment。 |
| #3 不自动化旧流程 | REQ-WIGA-01、08 | Task 4/5 manual issue 无 enrollment、人工 plan/advance/coding。 |
| #4 自动 plan，人工确认 | REQ-WIGA-03、05、REQ-CG-04 | Task 5 唯一 plan/session、自动生成、驾驶舱实际批准与 compile。 |
| #5 后台全链 | REQ-WIGA-03、04、06、07 | Task 5 plan→approve→compile→advance→coding→FinalConfirm waiting→人手 Completed；真实 Claude 与 claim ledger。 |
| #6 WIG 查看进度 | REQ-WIGA-08 | Task 4/5 WIG 入口 group 进度与 plan/attempt 身份截图。 |
| #7 确认与 advance 后自动首启 | REQ-WIGA-03、04、REQ-ADV-05 | Task 5 Confirmed→Ready→独立首启，单 attempt 一个 durable claim/runner、无页面。 |
| #8 Coding 实时下钻 | REQ-WIGA-08 | Task 4/5 WIG/信息卡→正确项目/issue/attempt 的 Coding 页面与实时日志。 |
| #9 后台不用流式页面 | REQ-WIGA-03、08 | Task 4/5 关闭页面/driver/observer，真 provider 运行、重开 timeline replay。 |
| #10 choice/门/recovery | REQ-WIGA-05 | Task 4/5 驾驶舱人手回答及真实 waiter 回执、批准/recovery 操作。 |
| #11 plan confirmed 提醒 | REQ-WIGA-07 | Task 1/3/5 成功 tx/session 事实、K 外读取、首次静默/后续一次提醒。 |
| #12 coding 结束/待最终确认提醒 | REQ-WIGA-07 | Task 1/3/5 readiness + WaitingForHuman/FinalConfirm、Coding 下钻、同 key Completed。 |
| #13 提醒与待处理联动 | REQ-WIGA-05、07 | Task 3/5 info 可见/可提醒、标题/角标/系统待处理仅 actionable、真实人工卡可操作。 |
| #14 业务流程和人工门不变 | REQ-WIGA-02、03、08、REQ-CG-04 | Task 4/5 两链对照、plan approve/compile recovery/Final Confirm 人手与五红线。 |
| #15 非 enrolled 零回归 | REQ-WIGA-01、08、REQ-MTG-03 | Task 4/5 手工 full chain/multi-target、原错误/恢复/choice 契约不变。 |

**设计取舍：**推荐复用 lifecycle additive response（少一轮跨项目请求、旧客户端不变）；独立通知表/推送服务会变成第二事实源，拒绝。磁盘按 issue JSON 枚举不具时间索引，现阶段只保证**结果**的 24h/32 条上界；若将来要证明 IO 复杂度受 32 限制，须另立带索引且从 durable 原事实可重建的设计，不在 P3 暗加缓存。推荐保留旧 `inbox`/`countedInbox` 作为迁移过渡的同义返回字段但 Shell 只从显式三分数据取含义；另一方案立即删旧字段需要同时迁移所有调用点、超出本 P3 小范围。24h 时间窗是本计划显式取值，不是 spec 原本规定的小时数；Task 1/3 测试必须锁定该边界。

**自审（计划阶段）：**共 **5 个 Task**，§4.1→1–3、§4.2→4、§4.3 与 §3.4→5；Issue 1 #1–#15、REQ-WIGA-01～08、REQ-CG-04、REQ-ADV-05、REQ-MTG-03 有逐项验收证据位置。五项 Review Focus 各有对应失败行为测试；新增接口 DTO/URL/TS 字段在首次定义处一致。本文只规划未来测试、构建、部署、真实 provider 与人工操作，**未运行、未关闸、未改源码或 OpenSpec**；Claude 网关已由用户确认可用，本计划不另设等待/安全网。
