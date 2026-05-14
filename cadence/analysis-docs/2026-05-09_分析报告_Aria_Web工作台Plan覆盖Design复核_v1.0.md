# Aria Web 工作台 Plan 覆盖 Design 复核

## 结论

复核对象在完成本次修正后，可以覆盖 `cadence/designs/2026-05-09_技术方案_Aria_Web工作台与逐节点交互Runtime设计_v1.0.md` 的第一版范围。

执行标准仍然是 design v1.0。总计划和 P1-P6 只能作为执行组织与任务拆分；如果执行中发现计划与 design 冲突，必须按 design 修正计划或先补设计评审。

## 复核对象

- Design：`cadence/designs/2026-05-09_技术方案_Aria_Web工作台与逐节点交互Runtime设计_v1.0.md`
- 总计划：`cadence/plans/2026-05-09_计划文档_项目开发_Aria_Web工作台与逐节点交互Runtime实现计划_v1.0.md`
- 拆分总览：`cadence/plans/2026-05-09_计划文档_项目开发_Aria_Web工作台_拆分执行总览_v1.0.md`
- P1：`cadence/plans/2026-05-09_计划文档_项目开发_Aria_Web工作台_P1_RuntimeCore_v1.0.md`
- P2：`cadence/plans/2026-05-09_计划文档_项目开发_Aria_Web工作台_P2_WebAPI与SSE_v1.0.md`
- P3：`cadence/plans/2026-05-09_计划文档_项目开发_Aria_Web工作台_P3_前端基础_v1.0.md`
- P4：`cadence/plans/2026-05-09_计划文档_项目开发_Aria_Web工作台_P4_工作台UI_v1.0.md`
- P5：`cadence/plans/2026-05-09_计划文档_项目开发_Aria_Web工作台_P5_执行交互与回退_v1.0.md`
- P6：`cadence/plans/2026-05-09_计划文档_项目开发_Aria_Web工作台_P6_验收回归_v1.0.md`

## 审计方法

1. 逐段读取 design：范围、目标、非目标、页面信息架构、逐节点执行、policy、provider 确认、事件流、数据模型、回退、Web API、错误处理、技术选型、测试策略、验收标准。
2. 对照总计划 master task 1-15，确认每个 design 要求都有执行任务、文件边界和验证命令。
3. 对照 P1-P6，确认拆分没有改变 design 范围，也没有把第一版以外目标纳入实现。
4. 扫描占位词和含糊项，确认没有未完成标记、延后实现描述、引用式省略和空泛测试描述等计划失败模式。
5. 用 `git diff --check` 检查本次文档修改没有空白错误。

## 覆盖矩阵

| Design 要求 | 覆盖计划 | 复核结果 |
|---|---|---|
| 本地单机、单 workspace、`aria web --workspace` | P2、P6 | 覆盖 |
| 不做云端、多用户、桌面壳、多 workspace 管理 | 拆分总览、P6 | 覆盖 |
| 默认监听本地，未指定端口时可自动选择 | 总计划 Task 8、P2 | 已补齐自动端口要求 |
| TUI 全信息域：Overview、Timeline、IO、Artifacts、Changes、Diagnostics、Action | P1、P3、P4、P5、P6 | 覆盖 |
| 第一屏即工作台，不做 landing page | P3、P4、P6 | 覆盖 |
| 顶部状态栏全部状态字段和 blocked_by_gate 拆解 | P4、P6 | 覆盖 |
| Flow Rail N00-N28、状态、provider、attempt/rework、artifact、gate 标记 | P4、P6 | 覆盖 |
| Node Workspace Overview/Inputs/Run/Outputs/Diff | P1、P4、P6 | 覆盖 |
| Evidence Panel 展示 OpenSpec、artifacts、reports、provider records、node-events、source/test/log | P1、P2、P4、P6 | 覆盖 |
| Markdown 目录/锚点、JSON 长字段折叠、source/test/log 渲染 | P4 | 覆盖 |
| Action Composer 展示 prompt/input/schema/scope，由用户确认 | P5、P6 | 覆盖 |
| PendingProviderStep 全字段：canonical input refs、context files、forbidden actions、verification commands | 总计划 Task 1/10/12、P1、P5、P6 | 已补齐前端展示要求 |
| 单节点临时 policy override | 总计划 Task 1/12/14.5、P1、P5、P6 | 已补齐 |
| 内部节点或自动执行阶段显示当前动作、事件摘要、停止入口 | P1、P5、P6 | 覆盖 |
| 每轮 turn、checkpoint、changed files、diff、dropped history | P1、P4、P5、P6 | 已补齐 Timeline/Changes 明示 |
| 逐节点推进、provider 前暂停、确认后执行 | P1、P2、P5、P6 | 覆盖 |
| confirm 后写入 provider run、turn、node run、artifacts、reports、events | P1、P2、P6 | 覆盖 |
| policy preset：manual-all、manual-write、auto-review、non-interactive | P1、P3、P6 | 覆盖 |
| SSE 使用 design 事件 taxonomy | P2、P6 | 覆盖；已去掉新增 `stop_requested` 事件类型 |
| 浏览器断线后通过 projection 恢复 | P2、P3、P6 | 覆盖 |
| WorkspaceProjection、InteractionTurn、RuntimeCheckpoint、ArtifactIndexEntry | P1 | 覆盖 |
| rollback preview、dirty 明确确认、恢复 Git/runtime 边界、dropped=true | P1、P5、P6 | 覆盖；已补强 WebRuntime 必须调用真实 checkpoint service |
| Web API 契约 listed endpoints | P2 | 覆盖 |
| Stop 控制 | P2、P5、P6 | 功能覆盖；事件不扩展 taxonomy。后端 stop route 是由 design UI 停止要求推导出的实现入口。 |
| 错误标准化和 Diagnostics Panel 分类 | P2、P4、P6 | 覆盖 |
| 前端技术选型：pnpm、React、Vite、TypeScript、TanStack Router、Tailwind、Radix、lucide | P3、P4、P5 | 覆盖 |
| URL search params 恢复 node、tab、artifact、turn | 总计划 Task 10、P3、P6 | 已补齐 |
| 视觉约束：高密度、非营销页、不照搬 vibe-kanban、不做单色渐变/装饰背景 | P3、P4、P6 | 覆盖 |
| Rust 单元、集成、前端、E2E 验收策略 | P1-P6 | 覆盖 |
| fake provider 完整闭环 E2E | 总计划 Task 15、P6 | 已从首屏 smoke 补强为完整闭环 |
| Fibonacci blocked_by_gate 样本诊断 | P4、P6 | 覆盖 |
| provider authorization/command diagnostics | P2、P6 | 覆盖 |
| `aria task run --non-interactive` 不回归 | P1、P6 | 覆盖 |

## 本次发现并修正的问题

1. PendingProviderStep 前端展示不完整：总计划和 P5 原本只强调 prompt/input/schema/scope，未完整覆盖 canonical input refs、context files、forbidden actions、verification commands。已补齐到总计划 Task 10/12、P5、P6。
2. 单节点临时 policy override 缺失：design 明确允许单节点 override，原计划只覆盖全局 policy preset。已补齐到总计划 Task 1/12/14.5、P1、P5、P6。
3. URL search params 恢复过弱：原计划只描述基础结构，未验证 node/tab/artifact/turn 往返。已补齐到总计划 Task 10、P3、P6。
4. 端口自动选择缺失：design 要求未指定端口时可自动选择，原总计划默认固定 4317。已改为未指定 `--port` 时绑定 `0` 并记录实际监听地址，P2 明确覆盖。
5. fake provider E2E 太浅：原 Task 15 只验证首屏，不能证明闭环。已改为覆盖 create、advance、pause、prompt edit、policy override、confirm、provider output/artifact、rollback、dropped history、rerun-ready composer。
6. stop 事件越界：原计划加入 `stop_requested` SSE 事件，超出 design 事件 taxonomy。已改为 stop 后只发布 design 事件表内的 `projection_updated`，错误/失败路径使用 `node_failed` 或 `error`。
7. Timeline/Changes 在拆分计划中表达不够显式。已补齐 P4/P6 的 Timeline/Changes browse 和验收项。
8. WebRuntime rollback 表达偏 fake：总计划 Task 13 原本只证明 fake runtime 可返回 rollback 结果，不能证明 Web API 路径会恢复 Git/runtime 边界。已补齐 Task 3、Task 13、P1、P5、P6：WebRuntime rollback 必须调用 `CheckpointService`，并验证 Git head、state/projection snapshot、artifacts/reports/turns/node-runs/provider-runs dropped history。

## 剩余注意事项

- Design 的 Web API 契约列表没有显式列出 stop endpoint，但页面设计要求 Action Composer 和 AutoActionStatus 支持停止。当前计划保留 `POST /api/tasks/{task_id}/stop` 作为由 UI 停止要求推导出的本地实现入口，并约束不得扩展 SSE taxonomy。若后续要求 API 列表完全封闭，需要先更新 design 或将停止降级为不可用状态。
- P6 必须最后执行，不能用 P1-P5 的局部测试替代最终验收。
- 如果 P1-P6 实施过程中发现任何行为与 design 不一致，应优先改计划或设计评审，不应降低验收标准。

## 最终判断

按当前修正后的计划集合执行，可以完整覆盖 design v1.0 的第一版目标、非目标、UI 信息架构、runtime 行为、API/SSE、回退语义、测试策略和验收标准。
