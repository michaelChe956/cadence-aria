# 技术方案：Auto Flow——Work Item Group 与 Coding 全自动执行 v1.0

- 日期：2026-08-24
- 分支：`feat-b-0808-rewrite-workitem-coding`
- 状态：待评审

## 1. 背景与目标

当前 Coding 链路（design spec 确认后）由多个用户可见节点手动推进：生成 work item group → coding → review → 失败处理 → 组级最终确认。节点间的人工点击大多是机械性的（"group 生成完了，点开始 coding"），没有信息增量，纯耗时；coding 发现 plan 缺陷需要重写 work item group 时，用户还需要理解并操作中间状态。

**目标**：把这段流程变为一次自动执行（Auto Flow）：宿主状态机驱动主干循环，provider 负责循环内的生成与修复决策，全程事件留痕，phase 级可视化，多角色可交叉配置不同 provider。

**明确不做**：

- design spec 的产生与修订（对话式修订、Canvas 审核等）不在范围内，保持现状，作为 Auto Flow 的上游输入。
- 不新建流程路径，不改变现有状态机语义（Plan Repair、provider_retry、组就绪检查、人工 Final Confirm、Coder/Reviewer 语义全部保持不变）。Auto Flow 只是"替代用户的手：按顺序、带预算和升级纪律地去触发这些节点"。
- 不做"同角色多 provider 冗余/竞赛"；"连续失败切换备用 provider"记为后续可选增强，本期不做。

## 2. 关键决策（已与用户确认）

| # | 决策 | 内容 |
|---|---|---|
| D1 | 自动化边界 | 可配置，默认模式 B：自动跑完含 rewrite 循环，组级最终确认保留人工；可切模式 A：全自动到底。升级人工条件任何模式下均生效。 |
| D2 | 编排驾驶权 | 混合式：主干循环（group 生成 → coding → review → 修复/rewrite → 下一组 → 最终确认）由宿主确定性状态机驱动；循环内的内容决策（如何修复、如何重写 group）下放 provider，以结构化结果返回。 |
| D3 | 多 provider | 按角色分 provider：执行前用户在 UI 为 Planner / Coder / Reviewer 各选一个 provider（复用现有 provider 配置体系）。rewrite 循环内沿用同一套配置。 |
| D4 | 留痕可视化 | 现有 timeline 照旧并补编排事件（细粒度），另加 phase 级"自动执行总览"卡片（当前 phase、rewrite 轮次、各角色活动，可下钻）。 |
| D5 | rewrite 边界 | 组内修复循环沿用现有 provider_retry / failed_review_recovery 预算；plan 缺陷触发 group rewrite 上限可配置、默认 2 轮；升级人工条件见 §6。 |

## 3. 架构

```
已确认的 design spec（上游输入，不在范围内）
        │ 用户启动 Auto Flow（配置角色 provider + 模式 A/B + rewrite 上限）
┌───────▼──────────────────────────────────────────┐
│ 宿主：AutoFlowOrchestrator（新增，确定性状态机）      │
│  phases: group_gen → coding → review →             │
│          (repair | rewrite) → next_group → …       │
│          → readiness → final_confirm(默认暂停)      │
│  循环上限、暂停点、预算、gate 全在宿主侧              │
│  每个 phase 迁移持久化，断点可恢复                    │
└───────┬──────────────────────────────────────────┘
        │ 作为被调度执行单元调用（结构化结果返回）
┌───────▼──────────────────────────────────────────┐
│ Provider（按角色，用户执行前选择）                    │
│  Planner：work item group 生成 / rewrite patch 生成 │
│  Coder：unit 实现                                  │
│  Reviewer：组内 review / 缺陷归类                    │
└──────────────────────────────────────────────────┘
```

### 3.1 复用现有骨架

coding_workspace_engine 的 group_review_orchestrator、plan_repair_start、rework、failed_review_recovery、provider_retry 不重写；AutoFlowOrchestrator 是把它们串起来的编排层。兼容 `scalable-group-final-review` change 的方向：组级最终是客观就绪检查 + 人工 Final Confirm，不新增第三层 AI 判断。

### 3.2 失败纪律（借鉴 DeepSeek Harness 的 fatal 二分）

- provider 返回的失败是**结构化数据**（review 未过、plan 缺陷判定、重写建议等）= 可循环项，由编排器按预算决定重派/rewrite/升级；
- 编排器自身错误（状态不一致、预算超限、patch 校验失败重试耗尽）= **fatal**，立刻停并升级人工，不允许被吞掉变成"看起来正常的重试"。

### 3.3 cadence-skills 工作流作为配置输入

Auto Flow 的 phase 结构与各角色的 prompt 组装从 cadence_skills 已同步的工作流定义（routing_reference / prompts）读取生成，工作流升级时 Auto Flow 行为跟着走，不在 Rust 里硬编码第二份流程。

### 3.4 角色声明式定义（借鉴 OpenCode）

Planner / Coder / Reviewer 定义为"权限规则集 + prompt 模板"的声明式配置（由 cadence_skills 工作流渲染生成），provider/model 只是其中一个字段。Reviewer 考虑只读约束（借鉴 Kimi Code explore 角色）。

## 4. Group Rewrite：PlanPatch 式增量修订（借鉴 OMA）

plan 缺陷触发 rewrite 时，不让 Planner 推倒重来：

1. Planner（Rewriter）输出**结构化 group patch**：`keep / invalidate / modify / add` 每个 unit 的处置 + 理由；
2. 宿主校验 patch：unit 状态引用合法、DAG 依赖一致、已通过 unit 不在作废集合中（除非显式声明并附理由，此时升级人工确认）；
3. 校验通过后**原子应用**：仅受影响 unit 进入重跑，已完成且保留的 unit 结果不动；
4. patch 以**可 diff 的产物文件**持久化留痕（借鉴 Kimi Code plan 文件审批），模式 B 下可选开启"rewrite 前人工确认"开关（默认关，保持全自动节奏）；
5. rewrite 轮次上限默认 2（可配置），每轮 patch 与校验结果均留痕。

## 5. 留痕与可视化

### 5.1 细粒度事件（timeline）

新增编排事件类型：`auto_flow_phase`、`rewrite_triggered`、`group_patch_applied`、`provider_decision`、`escalated_to_human`、`awaiting_final_confirm` 等，投影进现有 timeline / ws_event_mapper，并持久化（复用 execution_record_store / checkpoint 机制）。

### 5.2 Phase 级总览卡片（借鉴 Factory Missions 的阶段分段 + Kanban 形态）

工作台新增"自动执行总览"：按 phase 分组显示当前进度、rewrite 轮次、各角色 provider 当前活动，可下钻到 group/unit 明细与 provider transcript。UI 上把"计划协作期（spec/design）"与"执行自主期（Auto Flow）"作为两个明确的体验阶段分开呈现。Cadence 重启后可完整回放。

### 5.3 断点恢复（借鉴 Conductor durable execution）

编排器每个 phase 迁移持久化快照；进程崩溃/重启后 Auto Flow 从断点继续（at-least-once 语义，重入由现有幂等机制保证）。"等待人工"是状态机里的合法节点（`awaiting_final_confirm`、升级暂停），不是被打断的异常态。

## 6. 并发与预算纪律（借鉴 Cumora COORDINATION 实践）

1. **预算按 provider 分列**：Planner / Coder / Reviewer 可能是不同 provider（各自独立配额体系），预算三层（软提醒 → 工具降级 → 硬停）必须按角色-provider 对分别计数，不得用单一全局计数器覆盖所有角色（Cumora 教训：只限一层、另一层同步打满导致全队静默）。
2. **确定性底座垫在 AI 判断之下**：rewrite 轮次、预算硬停等上限编码在宿主状态机，不依赖 provider 自律；且升级点设 decline cap——同一升级决策用户连续忽略/拒绝 N 次（默认 3）后不再重复打扰，需显式重新打开。
3. **override 有代价（hold-token 思想）**：「继续加一轮 rewrite」等升级选项必须绑定当时呈现给用户的状态快照；轮次耗尽后同一决策不可免费重放，防止无脑点继续导致无限烧钱。
4. **AI 判断失败 fail-open + 窄确定性回退**：某角色 provider 不可用（如 Reviewer 持续 5xx）时，编排器不卡死、不伪造结论，走窄回退（标记 unreviewed 并暂停等待人工），并留痕。
5. **成本台账**：所有 provider 调用统一记账（角色、phase、token/成本），在 phase 总览卡片中按轮次展示。

## 7. 升级人工（结构化决策请求）

任何模式下，出现以下情况暂停并呈现结构化摘要：

- rewrite 轮次用尽；
- provider 输出连续不可解析 / 预算耗尽（预算三层：软提醒 → 工具降级 → 硬停，借鉴 pi-subagents）；
- review 判定"需求歧义需要用户决策"；
- patch 校验需要作废已通过 unit。

**升级不是自由文本，是结构化决策请求**，固定 schema：暂停点、触发链（哪个 unit 的 review finding → 判定 plan defect → patch 建议为何被拒/超限）、已消耗轮次/预算、具体选项列表（继续加一轮 / 接受部分完成 / 人工修改后重入 / 终止）。前端按 schema 渲染。

## 8. 用户干预

Auto Flow 对用户暴露三个显式操作（借鉴 pi-subagents 的 steer/stop/resume）：

- **暂停**：运行中打断，落回手动模式，可继续手动推进或恢复自动；
- **终止**：结束本次 Auto Flow，已完成的 unit 成果保留；
- **恢复**：从暂停/升级点继续自动执行。

## 9. 配置项

| 配置 | 默认 | 说明 |
|---|---|---|
| `automation_mode` | `B`（组级确认人工） | 可切 `A`（全自动到底） |
| `rewrite_max_rounds` | `2` | plan 缺陷 group rewrite 上限 |
| `rewrite_confirm` | `off` | rewrite 应用前人工确认开关（模式 B 可开） |
| 角色→provider 映射 | 无默认 | Planner / Coder / Reviewer 各选一个，启动前配置 |

## 10. 测试策略

- 编排状态机为纯 Rust 逻辑，TDD 全覆盖：phase 推进、循环上限、fatal 纪律、暂停点、断点恢复语义、干预三命令；
- patch 校验器：合法/非法 patch、DAG 一致性、已通过 unit 保护；
- 升级摘要 schema 解析与渲染；
- provider 决策结构化输出解析按现有 provider 测试模式；
- 验证命令遵循 `cadence/project-rules/build-test-commands.md`（禁止 `-j 1`）。

## 11. 竞品参考清单

| 来源 | 吸收点 |
|---|---|
| DeepSeek Harness workflow seam | 宿主/ provider 分层、fatal 失败纪律、事件流 + 持久记录 |
| Open Multi-Agent (OMA) Adaptive Recovery | PlanPatch 增量 rewrite + 宿主校验原子应用 |
| Claude Code Agent Teams | 只动未完成任务、成果留在共享产物不重做 |
| OpenCode | 角色声明式（权限规则集 + prompt） |
| Kimi Code | patch 产物文件化 + 显式审批开关、只读角色职责隔离 |
| pi-subagents | 预算三层（软提醒/工具降级/硬停）、干预三命令（暂停/终止/恢复） |
| Factory Missions | 计划协作期 vs 执行自主期的 UI 分段 |
| Conductor / AgentSpan | phase 迁移持久化断点恢复；人工等待 = 合法状态节点；LLM 规划一次、执行确定性 |
| Cumora（COORDINATION.md） | 预算按 provider 分列、升级 decline cap、override 绑定状态快照、fail-open 窄回退、统一成本台账 |

## 12. 依赖与顺序（add-monorepo）

本方案与 `feat-b-0808-add-monorepo` 分支存在大面积交叠与语义依赖：

- **文件级**：monorepo 分支重度修改了本方案依赖的全部核心模块（coding_attempt_store、coding_workspace_engine 各子模块、cadence_skills/routing_reference、workspace_engine），Auto Flow 若从当前 main 独立长分支再合并，冲突代价极高。
- **语义级**：monorepo 引入 logical codebase / per-target worktree routing（gates_worktree_routing、handoffs_worktree_routing、cross_target_check、target_snapshot）、admission 准入、issue_delivery 与指针发布链路。因此：
  - AutoFlowOrchestrator 必须 **target-aware**：group_gen 阶段可能按 logical codebase 成员拆分目标，coding / review / readiness 各 phase 的推进语义均带 target 维度；
  - phase 序列 `→ readiness → final_confirm` 需纳入指针发布等新环节（如：指针发布失败是否阻塞下一组、是否纳入升级条件，在实施计划中细化）；
  - §3.3 角色声明式定义的接入点以 monorepo 版 routing_reference API 为准。
- **顺序约束**：Auto Flow 实施依赖 add-monorepo 先合入 main（或在 monorepo 分支之上开发），避免约 9 万行规模的 rebase。

## 13. 影响面

- 后端：新增 AutoFlowOrchestrator 编排层（`src/product/` 新模块）、编排事件类型与持久化、patch 校验器、配置项；现有 coding_workspace_engine 子模块以被调用方式复用，语义不变。
- 前端：Auto Flow 启动配置面板（角色 provider + 模式）、自动执行总览卡片、升级决策请求渲染、干预操作入口。
- 后续增强（本期不做）：连续失败切换备用 provider、同角色多 provider 冗余。
