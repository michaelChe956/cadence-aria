# N6 报告：plan 一键与聚合 UI（REQ-CS-01/06/08）

- Change：`openspec/changes/fix-multi-repo-group-coding-start/`（Task 3.1，REQ-CS-01/06/08）
- Plan：`cadence/plans/2026-10-13_实施计划_多仓分组coding启动_v1.0.md` §7
- Worktree：`.worktrees/feat-b-0808-add-monorepo`（分支 feat-b-0808-add-monorepo）
- 状态：DONE（红→绿→vitest 全量+build 双绿；初版 commit `fix: N6 前端 plan 一键启动+聚合进度（GET互证/单发POST/selected打开/防抖重试/v1不变）`；P2 修复轮 commit `fix: N6 invalidated镜像与onboarding锚点`）

## 交付（两新建 + 八窄改）

- **Create `web/src/pages/plan-coding-progress.ts`**（DTO 纯映射）：
  `toPlanCodingProgressView(dto)` 只投影文案/入口——owning target/attempt 透传、
  `execution_state`→文案（`initialized`=已初始化、`running`=provider 运行中，严格
  区分）、`waiting_reason`→REQ-CS-06 三文案（等待依赖完成/等待执行槽位/下一项编排
  尚未启用；reason 缺失时仅按 execution_state 兜底）；`planComplete`/完成计数只透传
  DTO 事实（completed units 计数为展示派生），primary Completed 不冒充 plan 完成。
  另导出 `planCodingWaitingLabel(unit)` 供兜底投影复用。
- **Modify `web/src/api/types/coding.ts`**：N3 冻结 DTO 只读镜像
  （`PlanCodingStartRequest/TargetDto/UnitProgressDto/ProgressDto/StartResultDto`
  ＋值域 `PlanCodingLeaseState/ExecutionState/WaitingReason/OperationState` 与 N2
  tagged `CodingPlanStartOutcome`），经既有 `api/types.ts` re-export，无新导出层。
- **Modify `web/src/api/client.ts`**：`startPlanCoding(projectId,issueId,planId,request)`
  POST 精确 body；`getPlanCodingProgress(projectId,issueId,planId)` GET 同地址
  `/work-item-plans/{plan_id}/coding-start`。旧 group POST 单对象 `{}` 不变。
- **Modify `web/src/components/lifecycle/IssueLifecycleWorkbench.tsx`**：
  `handleOpenCodingWorkspaceFromDrawer` group 分支新增多仓判定
  `countPlanTargetRepositories`（plan work items 落 ≥2 个非兼容投影 target 仓才走新
  链）。多仓无现成 attempt：GET 聚合取 `bound_plan_revision_id` → 单发 POST
  `startPlanCoding`（`command_id` 一次用户动作生成、失败重试同 plan 复用同一
  command）→ 打开 `selected_attempt_id ?? primary_attempt_id`——selected 缺失时
  打开 primary 为设计接受（纯查看入口，不代表启动成功）；两 id 均缺失（零绑定
  等待窗）才停留抽屉提示等待不导航；在途 `planCodingStartInFlightRef` 防双击/
  请求中重复出站；失败走错误横幅。已有 group attempt（多仓或单仓）：原样直接打开，
  零 POST（不把 latestGroupAttempt 存在当开始成功）。单仓/无分仓投影：v1 旧
  `createGroupCodingAttempt` 端点原路径零改动。
- **Modify `web/src/pages/CodingWorkspaceGroupProgress.tsx`**：新可选 props
  `projectId/issueId/selectedAttemptId/refreshKey`；地址在场时组件自己 GET 聚合并渲染
  `plan-coding-progress`（per-WI owning 仓@、状态文案、等待文案＋直接未满足依赖 ID
  集、completed x/y、plan 完成/未完成、selected 入口）；GET 失败/缺席静默降级回既有
  单仓局部投影（`1 / 2` 条），selected WS 仍局部实时，不新增 plan 全局事件框架；
  `refreshKey` 变化重拉。旧 props 渲染保留。
- **Modify `web/src/pages/CodingWorkspacePage.tsx`**：父页面透传当前 address
  （projectId/issueId/selectedAttemptId=address.attemptId），refreshKey 由 store
  status/stage/units 长度派生（手动刷新/快照更新触发聚合重拉）；零 WS start。
- 测试四改两新（清单见下）。

## 证据（cwd=web；红→绿→双绿）

| 命令 | 红（实现前） | 绿 |
|---|---|---|
| `pnpm test src/api/coding-attempts.test.ts src/components/lifecycle/IssueLifecycleWorkbench.drawer.test.tsx src/components/lifecycle/IssueLifecycleWorkbench.single-coding.test.tsx src/pages/CodingWorkspaceGroupProgress.test.tsx src/pages/CodingWorkspacePage.test.tsx` | 6 failed/58 passed 全业务红：`startPlanCoding/getPlanCodingProgress is not a function`；`./plan-coding-progress` 模块不存在；多仓一键三测红于 selected attempt 未打开（旧路径走旧 POST）/无防抖/失败无横幅；页面 `plan-coding-progress` 不存在；single-coding v1 族 8 绿（基线回归位） | **5 files / 72 tests 全绿** |
| `pnpm test`（全量） | — | **213 files / 2143 tests 全绿** |
| `pnpm build`（tsc -b + vite build） | — | **成功**（chunk >500kB 警告为既有基线，非本包引入） |

新增/改写测试断言覆盖 plan 验收句：

- API：POST 精确 body（command_id＋expected_plan_revision_id 编码路径）；GET 同地址
  无 method/body；旧 group POST 仍单对象 `{}`。
- Drawer：多仓开始先 GET bound revision→恰一次 POST→打开 selected workspace，旧
  `/coding-attempts` POST 与单项 POST 零调用；双击/GET 在途第二次点击零额外出站；
  失败（500）错误横幅可见且不导航，重试 GET 重新互证 revision、POST 复用同一
  command_id；已有 running/completed group attempt 多仓直接打开零 coding-start
  调用；v1 单仓 group（无分仓投影）仍旧端点且零 coding-start。
- Single-coding：v1 裸 WI（计划归属 undefined/null）创建/复用两族原断言不动，追加
  零 `/coding-start` 出站。
- 进度：四 target/五 WI（busi=wi_core、web=wi_registration、gateway=wi_unrelated、
  frontend={wi_service,wi_panel}）owning/attempt/直接依赖全集（wi_panel=
  [wi_service, wi_unrelated]，传递上游不入直接集）；三等待文案；primary Completed
  而 planComplete=false 只透传；`initialized`（含 unit Running 夹具）文案不含
  provider、`running` 含 provider；无地址 props 保持旧 `1 / 2` 展示零 fetch；GET
  404 降级局部投影；refreshKey 变化重拉。
- 页面：地址正确透传（project_0001/issue_0001/work_item_plan_0001）且
  `api.startCoding` 零调用（selected WS 仍局部实时）；页面 client mock 默认拒绝聚
  合、旧用例全部维持局部投影基线。

## 边界与移交

- 未写 Rust 共享 types/global broadcast/WS store（纯消费 N3 冻结 DTO；N5 paused/
  waiting 投影经 DTO `execution_state/waiting_reason` 直接呈现，未在前端重算）。
- 多仓判定数据源：issue lifecycle `work_item_repository_groups`（`target_repository_id`
  非空且含本 plan work items 的分组的去重 target 数 ≥2）；兼容投影（null target）
  不计入。若后端未来在 plan DTO 上补显式多仓标志，可替换该派生而不动流程。
- 一次用户动作的 command 复用以 planId 为键存于组件 ref；跨页面刷新后是新动作、新
  command（后端按 run 幂等返回原 command，不会二次首启）。
- 与 N3/N5/N7 无 Rust 文件交集；`web/src/api/client.ts` 若与 D2.3 撞面须按计划
  同窗口串行移交。
- 发布/真人启动门禁维持：N6 UI 落地不代表开放真人启动；真 provider 验收属 Main
  统一窗口（plan §9.2），REQ-CS-04 的真实 Git/provider 证据不在本包冒充。

## P2 修复轮（Main 审查派回，2×P2）

- **P2-1（invalidated 镜像）**：`types/coding.ts` `PlanCodingExecutionState`
  union 补 `"invalidated"`（plan-run 授权失效投影：成员删除/精确 purge 后，与
  unit 事实状态无关）；`plan-coding-progress.ts` `EXECUTION_STATE_LABELS` 增
  `invalidated: "授权已失效"`，注释含物化事实区分说明。红：
  `expected 'invalidated' to be '授权已失效'`（原显示英文 token）。
- **P2-2（onboarding 锚点）**：聚合分支外层补
  `data-testid="onboarding-anchor-coding-progress"` 包裹 div（section 类名/断言
  不变），恢复 onboarding 第 13 步锚点契约；`anchors.test.tsx` 补聚合分支断言
  （先 `findByTestId("plan-coding-progress")` 确认聚合已渲染，再同步查锚点，
  杜绝 legacy 初渲染蒙混）。红：`Unable to find onboarding-anchor-coding-progress`。
- 非阻断观察（selected 缺失恒打开 primary）按设计接受，上文表述已同步弱化。
- 证据：定向 6 files / **82 tests 全绿**（+2 新测）；全量 `pnpm test` **213
  files / 2145 tests 全绿**；`pnpm build` **成功**。
