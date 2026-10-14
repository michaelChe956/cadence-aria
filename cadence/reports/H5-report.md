# H5 报告：Coding 工作区一键组删与清剩余（前端 N6 增补 UI）

- 设计：`cadence/designs/2026-10-14_方案设计_Coding工作区整组删除与重启_v1.0.md` C/D 节（冻结契约）
- Worktree：`.worktrees/feat-b-0808-add-monorepo`（分支 feat-b-0808-add-monorepo）
- 状态：DONE（前端红→绿→全量→build 三绿）
- 前置：H4 HTTP 端点未落——本包按设计 C/D 节冻结形状先行实现 client+UI（测试 mock 端点），端点联调归 H4 汇合

## 基线与撞面

开工 HEAD=`a2d68686`（D2.3 docs 提交后）；UI 收起（`03a253a1`）已在基线。开工
`git status` 对表：web/src 零在途改动（src/ 面为兄弟在途，零触碰）；`usePlanCodingProgress.ts`
在清单但无需修改（页面/抽屉 handler 直接走 client GET 取 generation，GroupProgress
自取数结构保持，避免牵动其既有 4 测试）。

## 实现

- **`api/types/coding.ts`**：`PlanCodingProgressDto` additive 扩展
  `workspace_lifecycle?`（`{generation, existing_member_count, missing_member_count,
  state: clean|complete|incomplete|clearing}`，旧响应缺省兼容）；新增
  `PlanCodingWorkspaceDeletionResultDto`（`operation_state: accepted|replayed`、
  `generation`、`deleted/remaining_attempt_ids`）。**字段名为按设计语义自拟，待 H4 对齐**。
- **`api/client.ts`**：`deletePlanCodingWorkspaces(projectId, issueId, planId,
  {command_id, expected_generation?, confirmed?})`——DELETE
  `/api/projects/{p}/issues/{i}/work-item-plans/{plan}/coding-workspaces`（设计 C 节
  冻结路径）；query 编码与 D2.3 同约定：默认无 `confirmed`，确认同意后显式
  `confirmed=true`；不收前端 attempt ID 列表（服务端是范围权威）。
- **`pages/plan-coding-progress.ts`**：视图模型透传 `workspaceLifecycle`；纯函数
  `planCodingGroupDeleteEntry`——全员在场「删除本计划全部 Coding 工作区(N)」、
  缺成员「删除剩余 Coding 工作区(E)」＋「本轮授权失效:已删 M/总」；无 lifecycle
  事实（H4 前）按 `target_attempts` 计数兜底、零 target 无计数（服务端预检权威）。
- **`pages/CodingWorkspaceGroupProgress.tsx`**：新增 `onDeleteAllCodingWorkspaces`/
  `groupDeleteBusy` props。三分支（聚合/局部投影/零成员 slim）都渲染入口
  `plan-coding-delete-all`——修掉旧 `units.length===0 → null` 对恢复入口的隐藏
  （设计 C 点名的 57–59 early return）；缺成员时聚合区展示
  `plan-coding-invalidation-summary`（主恢复动作即清剩余）。聚合失败＋units 空＋
  未接线才返回 null。
- **`pages/CodingWorkspacePage.tsx`**：
  - `handleDeleteAllCodingWorkspaces`：服务端分类→`deletion_confirmation_required`
    409→恰一次 `window.confirm`（消费服务端 message，浅/强分级文案权威；空文案兜底
    强警告）→`confirmed=true` 复删；取消零出站破坏请求。`command_id`
    `plan_delete_ui_*` 同一删除动作稳定复用（失败重试不换 key）；`expected_generation`
    取 GET `workspace_lifecycle.generation` 且**同一操作内冻结重用**（复删不跨代授权），
    GET 缺席/失败省略该参数不阻断服务端预检。busy 禁重复点击；成功 `onBack()` 回工作台；
    typed 409/500 直达状态栏错误面（`isCurrentDeleteRequest` 地址竞态守卫沿用
    deleteGenerationRef 机制）。
  - 渲染条件 `attemptScope==="work_item_group" && (units.length>0 || workItemGroupId)`——
    snapshot 已确认 group plan_id 即保留唯一恢复入口。
  - 顶栏单删按钮改名「仅删除当前 Coding 工作区」（设计 C）；多仓 group 下确认文案追加
    设计 D 防呆句（「这是本计划 N 个 Coding 工作区之一；仅删此项会使整轮授权失效…请
    使用删除本计划全部 Coding 工作区」，N 取 snapshot units>1 时计数，否则泛化句）。
    v1 单 attempt 端点/确认/导航行为不变（仅文案与命名）。
- **`components/lifecycle/LifecycleCardDrawer.tsx`**：plan 抽屉新增
  `onDeleteAllCodingWorkspaces`/`groupCodingDeleteBusy` props——Coding 区辅入口
  「删除本计划全部 Coding 工作区」（`drawer-delete-plan-coding-workspaces`，仅
  work_item_group）；Plan 本体按钮改名「删除计划及工作项」并放入独立危险区
  `drawer-danger-zone`（虚线分隔，与 Coding 区入口视觉分离）；work_item 删除保持原位。
- **`components/lifecycle/IssueLifecycleWorkbench(.tsx/View.tsx/Drawer.tsx)`**：
  `handleDeleteAllCodingWorkspacesFromDrawer` 与页级入口同一 DELETE 契约（同
  command_id 复用/generation 冻结/恰一次确认/取消零副作用）；typed 409 直达
  `setError` 错误面；成功 `refresh(projectId)` 刷新 lifecycle 事实（计划与工作项保留，
  抽屉不关）。辅入口不要求 plan confirmed 状态（残留清理对失效 plan 可达）。

### 旧 caller/测试迁移（登记）

1. `CodingWorkspacePage.delete-race.test.tsx`（清单内，3 处）：按钮名
   「删除 Coding Workspace」→「仅删除当前 Coding 工作区」；mock 工厂补
   `deletePlanCodingWorkspaces`/`getPlanCodingProgress`（未触达，防缺导出）。
2. `IssueLifecycleWorkbench.drawer.test.tsx`（清单内，5 处）＋
   `IssueLifecycleWorkbench.crud.test.tsx`（**清单外**，4 处）＋
   `CodingWorkspacePage.test.tsx`（**清单外**，2 处）：「删除 Work Item Group」→
   「删除计划及工作项」、顶栏按钮名同迁——均为设计 C 强制改名下的机械迁移，
   请 Main/reviewer 复核。

## 证据（cwd=web）

| 命令 | 红（实现前） | 绿（实现后） |
|---|---|---|
| `npx vitest --run`（7 文件族：coding-attempts/GroupProgress×2/Page×3/Drawer×3） | **32 failed / 52 passed**（缺函数/缺入口/旧命名） | **99 passed / 0 failed** |
| `npm test`（全量） | — | **214 文件 2192 passed / 0 failed**（首次全量 2 failed=Page.test 旧命名，迁移后复跑全绿；其间一次 1 failed 未捕获到用例名，同代码复跑全绿，判定 flaky） |
| `npm run build`（tsc -b && vite build） | — | **EXIT=0**（✓ built in 3.47s） |

无 web 侧格式化配置（prettier/biome 均无；D2.3 先例 fmt 仅 rustfmt 面），门禁为
tsc+build。

## 纪律与边界

- 只动 `web/src`（17 文件：16 改＋1 新测试文件）；src/ 兄弟在途零触碰；未 stash；
  未部署；未 push；commit 显式列文件（无 `git add -A`）。
- 确认模式完全复用 D2.3 已铺面：`ApiRequestError.code === "deletion_confirmation_required"`
  为唯一弹窗分支（ApiRequestError code/details 权威，不做前端确认历史判断）；
  busy/reference/版本 409 不弹窗不自动重试。
- 取消路径零出站破坏请求、无 onBack/refresh/退场（仅 busy 复位）——测试件断言出站
  恰 1 次（首删）。

## 移交与登记

- **待 H4 对齐**（设计文档只冻结路径与 query 语义，字段形状为自拟）：
  ①回执字段名 `deleted_attempt_ids`/`remaining_attempt_ids`/`operation_state
  (accepted|replayed)`；②GET 扩展字段名 `workspace_lifecycle` 及其四字段
  （generation/existing_member_count/missing_member_count/state 值域）；③DELETE
  必填参数边界（当前 expected_generation 未知时省略，H4 若定 REQUIRED 则旧 GET
  下会得 typed 4xx 错误面——不误删不假成功）；④`deletion_confirmation_required`
  之外 H4 实际落地的清理中/资源 busy 稳定码。汇合时若改名，client+视图模型两处
  映射即可收口（UI/流程零改动）。
- 验收矩阵 8（浏览器真实路径「打开Workspace→删除全部→回工作台→开始Coding」）归
  Main 统一调度，本包以 jsdom 契约件覆盖流程面。
- 设计 C 的「页级操作在 owner 四次删除的实际路径验收」按 controller 浏览器上下文
  执行，不捏造截图证据。

## 四字段

- **状态**：DONE
- **提交**：`010d8a90` `fix: H5 coding工作区一键组删与清剩余`（17 文件显式 add）
- **验证**：红 32/0（7 文件族）→绿族 99/0→全量 **214 文件 2192/0**→`npm run build`
  EXIT=0
- **关切**：①清单外 3 文件 6 处按钮命名机械迁移（crud 4＋Page.test 2）待 Main
  复核；②DTO 字段名自拟待 H4 对齐（见移交①—④）；③全量中途一次未捕获 flaky
  （同代码复跑全绿）；④H4 端点未落，联调前真实请求将 404/405——前端按设计先行
  是任务既定边界
