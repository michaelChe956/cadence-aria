# 修复报告:编码仪表盘跨仓依赖投影(Slice B,issue_work_item_plan_0001)

> 诊断定案见 `local://diag-dashboard-deps.md`(两问:①依赖链下游「无」④仅 WI-001 有详情卡,同根于 split 后仪表盘数据源未重定义)。本切片实施冻结修法 Slice B:plan 全图投影,①④同解。

## 1. 修法(前后端分界,诊断冻结)

- **后端单处**:`build_plan_coding_progress`(plan_start.rs)给 `PlanCodingUnitProgressDto` 补三个展示字段——全量 `dependency_logical_work_item_ids`(权威绑定,跨桶边不丢)、`order_index`(全图拓扑 rank)、`summary`(桶内 unit 摘要)。不动 WS 协议、不动 session units、不动 progress.rs。
- **前端**:纯映射 `selectCodingDashboardUnits`(coding-dashboard-projection.ts)+ 共享取数 hook `usePlanCodingProgress`;仪表盘在 group attempt 且聚合可用(≥2 target attempt)时以 plan 全图 units 建拓扑/依赖链——跨桶 WI 走既有 12→六态投影为等待灰卡,跨仓下游边自动出现;无聚合(缺席/失败)或 v1 单 attempt 原样回落桶内 session units,桶内口径零变化。

## 2. 改动清单

| 文件 | 改动 |
|---|---|
| `src/web/types.rs` | `PlanCodingUnitProgressDto` +3 字段(snake_case serde) |
| `src/web/handlers/coding/plan_start.rs` | 投影点补全量依赖(排序)、`order_index`(rank 回落 usize::MAX)、`summary` |
| `src/web/handlers/coding/plan_start_tests.rs` | 聚合测试补三字段断言(含跨桶 wi_registration→wi_core);首项完成事实落 summary |
| `web/src/api/types/coding.ts` | DTO 类型 +3 字段 |
| `web/src/state/coding-dashboard-projection.ts` | 新增 `CodingTopologyUnitInput` 结构面与 `selectCodingDashboardUnits`;拓扑/链入参放宽到该结构面(session units 结构化满足,零行为变化) |
| `web/src/hooks/usePlanCodingProgress.ts`(新) | 共享 GET 取数(refreshKey 口径沿用 `status-stage-units.length`,失败静默 null) |
| `web/src/pages/CodingWorkspaceGroupProgress.tsx` | 内联取数重构为共享 hook(行为等价) |
| `web/src/components/coding-workspace/dashboard/CodingDashboard.tsx` | 拓扑/链数据源接 `selectCodingDashboardUnits(units, aggregate)`;group attempt 才启用取数 |
| `web/src/pages/CodingWorkspacePage.tsx` | 仪表盘传 `projectId/issueId` |
| 对应测试 | projection.test +3 用例;CodingDashboard.test +4 用例;GroupProgress/Page 测试夹具补字段 |

## 3. TDD 记录

- **红①(跨桶全图)**:busi attempt 仪表盘拓扑含全部 5 WI,WI-002=等待灰卡(data-state=pending),`WI-001__WI-002` 跨仓下游边 blocking,依赖链下游含 WI-002(不再是「无」)。后端红=DTO 三字段缺失(E0609 ×12,编译期)。
- **红②(回落不回归)**:聚合 GET 失败/缺席 → 原桶内行为(单节点 WI-001、下游「无下游消费」)。
- **红③(v1 零变化)**:非 group attempt 不发 fetch;单 target attempt 聚合也回落 session units(身份引用不变)。
- 前端红实跑 4 failed(`selectCodingDashboardUnits is not a function` ×3 + `fetch called 0 times` ×1)→ 绿全过。

## 4. 验证(全部实跑)

| 检查 | 命令 | 结果 |
|---|---|---|
| 后端族(主树,含 FixLeaseScope 在途增量) | `cargo test --lib plan_coding_n3` | 7/7 ok |
| 后端族(隔离树=HEAD+仅本切片 diff) | `cargo test --lib plan_coding_n3` / `--lib plan_coding` | 7/7、31/31 ok |
| 前端相关族 | `npx vitest run`(projection/dashboard 族、GroupProgress、Page、api、onboarding) | 57/57、42/42 ok |
| 前端全量 | `npx vitest run` | 213 文件 / 2152 测试全绿 |
| 前端构建 | `npm run build`(tsc -b + vite build) | 绿(chunk 体积告警为存量) |

隔离树归因记录:首隔离轮 one_click 失败,系 `git diff` 工作树时误扫入 FixLeaseScope 未提交的 plan_start_tests.rs 轮询断言(其修复在其独占的在途源文件,不在补丁内);剔除该 hunks 后 HEAD+仅本切片 7/7 绿,与本切片无关。

## 5. 边界与后续

- 未动:WS 协议、session units、`progress.rs` 桶内聚合、REQ-MTG-04 一期人工显式编排(plan 全图投影纯只读展示,无编排语义)。
- Slice A(桶口径文案标注)未实施,与 B 不互斥;B 已消除「无下游消费」的 plan 级误导。
- 前端拓扑中 plan 路径的 `completion_commit` 置 null(聚合 DTO 不携带,该字段现无渲染消费面)。
