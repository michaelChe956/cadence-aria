# Issue 生命周期与 Provider Workspace 工作台优化技术方案

## 文档信息

- 文档类型：技术方案
- 版本：v1.0
- 日期：2026-05-16
- 适用分支：`product-workbench-issue-lifecycle`
- 适用仓库：`cadence-aria`
- 背景参考：`cadence/designs/2026-05-14_技术方案_任务管理与Workspace空间管理工作台_v1.0.md`
- 设计目标：修正 Issue 生命周期结构，把 Story Spec、Design Spec、Work Item 前置为一等实体，并把所有 provider 交互统一收敛到 Issue 管理工作台内的 Workspace 弹窗。

## 1. 背景

当前 `product-workbench-issue-lifecycle` 分支已经具备 Project、Repository、Issue、Gate、Provider 事件和产品索引层的初步能力，但工作台结构仍然偏向旧执行模型：

- 页面按 Issue 阶段分列，Story Spec、Design Spec、Work Item 仍像 Issue 内部摘要，不是一等卡片。
- 右侧区域使用 “Issue 执行 Workspace” 描述，容易把 Workspace 理解为运行代码的目标空间，而不是 provider 对话、产物生成、review 和执行追踪的统一工作区。
- 旧执行工作台 UI 承担过多 runtime 细节，不适合作为 Story、Design、Work Item 的主流程入口。
- Story Spec、Design Spec、Work Item 需要由人工触发、人工审核，并在 provider 弹窗中完成交互；当前模型还没有把这些交互前置。

本方案保留既有后端 runtime、provider adapter、artifact、checkpoint、SSE、Gate 等能力，但放弃旧执行工作台 UI 作为主交互承载。新的主入口是 Issue 管理工作台内的四列生命周期看板和统一 Provider Workspace 弹窗。

## 2. 目标与非目标

### 2.1 目标

1. 首页主工作区改为四列实体看板：Issue、Story Spec、Design Spec、Work Item。
2. Issue 创建时必须选择 Repository，后续 Story、Design、Work Item 均结合代码上下文生成。
3. Story Spec 由 provider 基于 Issue 与代码上下文建议拆分，一个 Issue 可生成多个 Story Spec。
4. Design Spec 由 provider 建议 Story 到 Design 的映射，支持一对一与多对一，支持前端、后端两类 design spec。
5. Work Item 必须由 Story Spec 与 Design Spec 联合派生，并记录覆盖关系。
6. Story、Design、Work Item、Plan、Coding、Testing、Review、Rework、Final 都在统一 Provider Workspace 弹窗中完成。
7. Provider 交叉 review 支持 Project 级默认配置与单次 Workspace 覆盖。
8. 保留 superpowers 与 OpenSpec 作为 Project 级默认流程约束，并允许单次 Workspace 覆盖。
9. Work Item Workspace 必须先生成 Plan，人工确认 Plan 后才允许进入 coding/testing/review/rework/final。
10. 文档正文和生命周期元数据默认保存到产品索引目录，不默认写入目标代码库 `cadence/`。

### 2.2 非目标

- 不保留旧执行工作台 UI 作为主流程入口。
- 不重写底层 provider adapter、checkpoint、artifact projection、SSE event hub。
- 不在本方案中实现多人协作、远端同步、PR 状态或完整验收状态机。
- 不要求 Story/Design/Work Item markdown 自动导出到目标代码库；后续可作为显式导出能力设计。
- 不在前端实现完整插件系统；前端使用统一弹窗，后端 provider runner 采用插件化边界。

## 3. 推荐架构

采用“产品生命周期 Store + Workspace Session + 复用后端 Runtime 能力”的方案。

### 3.1 分层

```text
┌─────────────────────────────────────────────────────────────┐
│ IssueLifecycleWorkbench                                     │
│ Issue / Story Spec / Design Spec / Work Item 四列看板        │
└───────────────────────────────┬─────────────────────────────┘
                                │
┌───────────────────────────────▼─────────────────────────────┐
│ ProviderWorkspaceDialog                                      │
│ 流程轨道 / provider 对话 / 产物版本 / review / 人工确认       │
└───────────────────────────────┬─────────────────────────────┘
                                │ HTTP + SSE
┌───────────────────────────────▼─────────────────────────────┐
│ Product Lifecycle API                                        │
│ story/design/work-item/workspace-session/review/version API  │
└───────────────────────────────┬─────────────────────────────┘
                                │
┌───────────────────────────────▼─────────────────────────────┐
│ Product Index Store                                          │
│ .aria/projects/{project}/issues/{issue}/...                  │
└───────────────────────────────┬─────────────────────────────┘
                                │ adapter
┌───────────────────────────────▼─────────────────────────────┐
│ Existing Runtime Capabilities                                │
│ provider adapter / artifacts / checkpoint / SSE / gate        │
└─────────────────────────────────────────────────────────────┘
```

### 3.2 工程原则

- 前端主流程彻底切换到新的 Workspace 弹窗。
- 后端能力优先复用已有 runtime，不推倒重写。
- 产品层新增明确实体，避免继续从 runtime artifact 反推 Story、Design、Work Item。
- Workspace Session 是 provider 交互、review 轮次、版本、确认记录的归档单位。

## 4. 信息架构与四列交互

主页面命名为 `IssueLifecycleWorkbench`，替代当前 `ProjectManagementWorkbench` 的阶段看板结构。

四列固定为：

| 列 | 职责 | 关键动作 |
|----|------|----------|
| Issue | 当前 Project 下的需求源头 | 新建 Issue、选择 Repository、触发 Story Spec 生成 |
| Story Spec | Issue 派生的用户故事文档 | 生成、review、修订、人工确认 |
| Design Spec | 已确认 Story 派生的前端/后端设计文档 | 生成映射、review、修订、人工确认 |
| Work Item | Story 与 Design 联合派生的工作项 | 拆分、确认、生成 Plan、执行开发流程 |

交互规则：

1. 默认展示当前 Project 下全量四列，方便扫描整体进度。
2. 点击某个 Issue 后进入聚焦态，右侧三列只展示该 Issue 派生链路。
3. 提供“显示全部 / 聚焦当前 Issue”切换。
4. 点击 Story Spec、Design Spec、Work Item 卡片打开统一 Workspace 弹窗。
5. 卡片展示标题、来源、当前版本、确认状态、provider review 轮次、最后更新时间。
6. 删除 “Issue 运行时只从当前 Project 的 Workspace 中选择” 文案；相关区域改为“代码库上下文”或通过卡片来源关系表达。

## 5. 生命周期规则

### 5.1 Issue

- 创建 Issue 时必须选择 Project 与 Repository。
- Repository 是 Story、Design、Work Item 的代码上下文来源，也是后续 Work Item 执行空间。
- 若 Repository 路径失效，Issue 与派生卡片进入 blocked 状态，Workspace 弹窗提示重新绑定或修复路径。

### 5.2 Story Spec

- Story Spec 生成由人工从 Issue 卡片触发。
- Provider 基于 Issue 描述、Repository 代码上下文和项目规则建议拆分为一个或多个 Story Spec。
- 用户审核拆分建议后创建对应 Story Spec 卡片。
- 每张 Story Spec 卡片内部使用多版本，不通过反复重做生成多张 rejected 卡片。

### 5.3 Design Spec

- Design Spec 只能从已确认 Story Spec 派生。
- Provider 建议 Story 到 Design 的映射，可一对一，也可多对一。
- Provider 同时建议 Design 类型：frontend、backend，或两者都有。
- 用户审核映射和类型后创建 Design Spec 卡片。

### 5.4 Work Item

- Work Item 必须由 Story Spec 与 Design Spec 联合派生。
- Work Item 记录 `story_spec_ids[]`、`design_spec_ids[]` 和覆盖说明。
- Work Item Workspace 必须先生成 Plan。
- Plan 人工确认前，coding/testing/review/rework/final 全部禁用。

### 5.5 交叉 Review

一轮交叉 review 定义为：

```text
作者 provider 生成 → reviewer provider 审查 → 作者 provider 修订
```

达到配置的 review 轮次后进入人工确认。人工审核不通过时，同一卡片追加新版本，保留 review 意见链和历史版本。

## 6. Provider Workspace 弹窗

统一弹窗命名为 `ProviderWorkspaceDialog`。

### 6.1 弹窗结构

| 区域 | 内容 |
|------|------|
| 顶部配置条 | provider 组合、review 轮次、superpowers/OpenSpec 约束、Repository 上下文 |
| 左侧流程轨道 | 当前 Workspace 类型的固定节点与状态 |
| 中间对话区 | 每个节点的 provider 对话、用户补充要求、澄清问答 |
| 右侧产物区 | markdown/json 产物、版本历史、review 意见、确认记录、来源引用 |

### 6.2 Workspace 类型

| 类型 | 主要流程 |
|------|----------|
| Story Workspace | 准备上下文、建议拆分、生成草稿、交叉 review、修订、人工确认 |
| Design Workspace | 读取已确认 Story、建议映射与类型、生成设计、交叉 review、修订、人工确认 |
| Work Item Workspace | 读取 Story 与 Design、拆分任务、生成 Plan、Plan 确认、执行 coding/testing/review/rework/final |

### 6.3 对话模式

Workspace 弹窗采用混合模式：

- 固定流程轨道保证过程可追踪。
- 每个节点都有 provider 对话区，允许用户补充指令和回答澄清。
- 对话、provider 输入、输出、产物、review、修订版本都绑定到同一个 Workspace Session。

### 6.4 Provider 插件边界

- 前端不做真正插件系统，使用固定的 Workspace 弹窗和组件结构。
- 后端 provider runner 插件化，支持 Claude Code、Codex、fake，并为后续 provider 扩展留接口。
- 所有需要 provider 与 Claude Code/Codex 交互的流程都必须从 Workspace 弹窗发起、恢复或追踪。

## 7. 数据模型与存储

产品层新增一等实体：

| 模型 | 关键字段 |
|------|----------|
| `IssueRecord` | `project_id`、`repository_id`、`title`、`description`、`status` |
| `StorySpecRecord` | `issue_id`、`repository_id`、`current_version`、`confirmation_status` |
| `DesignSpecRecord` | `issue_id`、`story_spec_ids[]`、`design_kind`、`current_version`、`confirmation_status` |
| `WorkItemRecord` | `issue_id`、`story_spec_ids[]`、`design_spec_ids[]`、`plan_status`、`execution_status`、`worktree_path` |
| `SpecVersionRecord` | `entity_id`、`version`、`markdown`、`provider_run_refs[]`、`review_refs[]`、`confirmed_by` |
| `WorkspaceSessionRecord` | `session_id`、`workspace_type`、`entity_id`、`flow_nodes[]`、`messages[]`、`status`、`config_override` |
| `ProviderReviewRoundRecord` | `author_provider`、`reviewer_provider`、`round_index`、`review_result`、`revision_result` |
| `ProjectProviderDefaults` | `author_provider`、`reviewer_provider`、`review_rounds`、`superpowers_enabled`、`openspec_enabled` |

默认存储在产品索引目录：

```text
.aria/projects/{project_id}/issues/{issue_id}/
├── issue.json
├── story-specs/{story_spec_id}.json
├── design-specs/{design_spec_id}.json
├── work-items/{work_item_id}.json
├── versions/{entity_id}/v{n}.json
├── workspace-sessions/{session_id}.json
├── provider-review-rounds/{round_id}.json
└── provider-inputs/{input_ref}.json
```

markdown 正文保存在 `SpecVersionRecord` 中，不默认写入目标仓库 `cadence/`。Repository path 只作为代码上下文来源和后续执行空间。

## 8. API 设计

新增产品生命周期 API：

```text
GET  /api/projects/{project_id}/issues
POST /api/projects/{project_id}/issues

GET  /api/issues/{issue_id}/lifecycle

POST /api/projects/{project_id}/issues/{issue_id}/story-specs:generate
POST /api/projects/{project_id}/issues/{issue_id}/design-specs:generate
POST /api/projects/{project_id}/issues/{issue_id}/work-items:generate

GET  /api/workspace-sessions/{session_id}
POST /api/workspace-sessions
POST /api/workspace-sessions/{session_id}/message
POST /api/workspace-sessions/{session_id}/run-next
POST /api/workspace-sessions/{session_id}/confirm
POST /api/workspace-sessions/{session_id}/request-change
POST /api/workspace-sessions/{session_id}/terminate
```

兼容策略：

- 旧 `/api/projection`、`/api/tasks/*` 保留给 runtime adapter、调试和已有测试。
- 前端主流程不再直接进入旧执行工作台。
- 旧 execution projection 可通过 adapter 映射到 Work Item Workspace 的节点流程视图。

## 9. 运行流程

完整流程：

1. 用户创建 Issue，并选择 Repository。
2. 用户点击 Issue 卡片触发 Story Spec 生成。
3. 后端创建 Story Workspace Session。
4. Provider 准备代码上下文，建议 Story 拆分。
5. 用户确认拆分，系统创建一个或多个 Story Spec 卡片。
6. 用户在 Story Workspace 内完成草稿生成、交叉 review、修订和人工确认。
7. 用户从已确认 Story Spec 触发 Design Spec 生成。
8. Provider 建议 Story 到 Design 的映射和 design 类型。
9. 用户确认后创建 Design Spec 卡片，并完成生成、review、修订、确认。
10. 用户从已确认 Story 与 Design 触发 Work Item 生成。
11. Provider 拆分 Work Item，用户确认后创建 Work Item 卡片。
12. 用户打开 Work Item Workspace，先生成 Plan。
13. 用户确认 Plan 后，系统允许进入 coding/testing/review/rework/final。
14. Work Item 完成后更新状态，并回写 Issue 生命周期覆盖与完成度。

## 10. Work Item 执行流程视图

Work Item Workspace 必须展示从 Plan 往后的全过程：

- Plan 生成、review、人工确认。
- Coding 节点的 provider 输入引用、输出流、artifact、修改范围。
- Testing 节点的命令、结果、失败分类。
- Review 节点的审查意见、修订请求。
- Rework 节点的重做内容与关联 review。
- Final 节点的最终摘要和确认状态。
- 每个节点对应的 workspace session、provider run、artifact、checkpoint、worktree 信息。

旧执行工作台中的 FlowRail、NodeWorkspace、EvidencePanel 思路可以拆解复用，但不再以旧页面承载。

## 11. 错误处理

| 场景 | 行为 |
|------|------|
| 创建 Issue 未选择 Repository | 禁止提交，表单提示必须选择代码库 |
| Repository 路径失效 | Issue 和派生卡片显示 blocked，Workspace 提示修复或重新绑定 |
| Provider 不可用 | Session 进入 `blocked_provider_unavailable`，允许切换 provider 后重试 |
| Review 未通过 | 同一卡片新增版本，不覆盖确认版 |
| 人工未确认 | 后续阶段按钮禁用，并显示依赖的未确认实体 |
| Plan 未确认 | Work Item coding/testing/review/rework/final 禁用 |
| Runtime adapter 失败 | Workspace 节点显示失败，保留 provider 输入、输出和错误码 |
| Provider 输入包含敏感信息 | SSE 只发送引用和摘要，读取完整输入时执行脱敏 |

## 12. 测试策略

### 12.1 后端测试

- Store 测试：StorySpec、DesignSpec、WorkItem、SpecVersion、WorkspaceSession、ProviderReviewRound 的创建、读取、版本追加、确认状态。
- 来源关系测试：Story 从 Issue 派生，Design 从 Story 派生，Work Item 从 Story + Design 派生。
- API 测试：generate story/design/work item，workspace session message/run/confirm/request-change/terminate。
- 阶段依赖测试：未确认 Story 不能生成 Design，未确认 Design 不能生成 Work Item，未确认 Plan 不能进入 coding。
- Provider runner 插件测试：fake、Claude Code、Codex runner 的选择、配置覆盖和错误回传。
- Runtime adapter 测试：provider input 引用、artifact 写入、checkpoint 关联和 Work Item 节点投影。

### 12.2 前端测试

- 四列全量展示和 Issue 聚焦过滤。
- 创建 Issue 时必须选择 Repository。
- 点击 Story/Design/Work Item 卡片打开 `ProviderWorkspaceDialog`。
- Project provider 默认配置和单次 Workspace 覆盖。
- Provider 对话、review 轮次、版本历史、人工确认状态展示。
- 人工确认后解锁下一阶段。
- Work Item Plan 未确认时开发节点不可运行。

### 12.3 端到端测试

最小 E2E：

```text
创建 Project/Repository/Issue
→ 生成并确认 Story Spec
→ 生成并确认 Design Spec
→ 生成并确认 Work Item
→ 生成并确认 Plan
→ fake provider 执行 Work Item 到完成
→ Issue 生命周期显示覆盖与完成状态
```

### 12.4 回归测试

- 旧 runtime API 保持可用。
- 旧 task/projection 测试不因新 UI 主流程变化而失败。
- Provider input redaction、SSE replay、checkpoint rollback 现有测试继续保留。

## 13. 迁移与兼容

1. 保留当前 Project、Repository、Issue store 作为基础。
2. 新增 StorySpec、DesignSpec、WorkItem、Version、WorkspaceSession、ReviewRound store。
3. 当前 `ProductIssueArtifactDto` 仍可作为旧 runtime artifact 兼容视图，但新工作台优先读取生命周期实体。
4. 旧执行工作台 UI 从默认入口移除，可暂时保留为隐藏调试入口或开发辅助页面。
5. 当前 `/api/projects/{project_id}/issues/{issue_id}/start` 不再作为主流程入口，后续由 Workspace Session 驱动。

## 14. 验收标准

1. 默认页面展示 Issue、Story Spec、Design Spec、Work Item 四列。
2. 创建 Issue 时必须选择 Repository。
3. Issue 可通过 provider 拆分生成一个或多个 Story Spec。
4. Story Spec 确认后才能生成 Design Spec。
5. Design Spec 支持 frontend/backend 类型和多 Story 映射。
6. Work Item 必须记录 Story Spec 与 Design Spec 来源。
7. Story、Design、Work Item、Plan 和开发执行均在 Workspace 弹窗内完成。
8. Project 级 provider、review 轮次、superpowers/OpenSpec 默认配置可被 Workspace 单次覆盖。
9. Work Item Plan 未确认前不能进入 coding/testing/review。
10. Work Item Workspace 可查看从 Plan 往后的完整节点、provider、artifact、checkpoint 和 worktree 信息。
11. 旧执行工作台 UI 不再作为主流程入口。
12. 后端 runtime/provider/artifact/checkpoint/SSE 能力在新流程中通过 adapter 复用。
