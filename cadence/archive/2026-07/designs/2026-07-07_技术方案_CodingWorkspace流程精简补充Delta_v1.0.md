# 技术方案：Coding Workspace 流程精简补充 Delta

## 文档信息

- 文档类型：技术方案
- 版本：v1.0
- 日期：2026-07-07
- 分支：feat-b-0630
- 配套文档：
  - `cadence/designs/2026-07-06_技术方案_CodingWorkspace流程精简_coder_reviewer双角色_v1.0.md`
  - `cadence/designs/2026-07-07_技术方案_CodingWorkspace材料驱动Prompt协议_v1.0.md`

## 1. 定位

本文件是 `2026-07-06_技术方案_CodingWorkspace流程精简_coder_reviewer双角色_v1.0.md` 的补充 Delta，不替代、不重写 07-06，也不要求按 07-06 全量重新实现。

实现时只执行以下两个方案：

1. 本补充 Delta。
2. `2026-07-07_技术方案_CodingWorkspace材料驱动Prompt协议_v1.0.md`。

07-06 只作为已实现流程基线和历史上下文。实现前必须先按当前代码事实确认哪些能力已经存在；已经完成的流程精简不得重复实现。

## 2. 当前基线假设

以当前 `feat-b-0630` 代码为准，而不是以 07-06 文档文字逐条重做。

- Coder 和 Testing 已按产品方向合并为一个执行节点；后续不再新增独立 tester 节点。
- CodeReview 是 coder 后的唯一单 WorkItem 审查节点。
- WorkItemGroup 内每个 unit 都会经过 Coding 和 CodeReview。
- WorkItemGroup 需要一个组级最终审查；单 WorkItem 不需要组级最终审查。
- 代码中若仍有 Testing、Tester、单 WorkItem InternalPrReview、Internal Reviewer 配置等残留，应按本 Delta 的删除/迁移规则处理。

## 3. 实现边界

### 必须做

- CodeReview 配置页增加自动返修次数设置。
- `max_auto_rework` 从配置写入 attempt，替代硬编码默认值。
- CodeReview `request_changes` 在自动返修额度内直接触发 coder 自动返修。
- 自动返修额度耗尽后进入人工返修 gate，后续同一 attempt 的 `request_changes` 都由人工 gate 推进。
- 人工返修意见进入 coder delta prompt，且优先级最高。
- 单 WorkItem CodeReview approve 后执行 ReviewRequest commit/push，push 成功即 completed，不再运行 InternalPrReview。
- WorkItemGroup 使用一个 shared worktree；组内 unit 切换不得重复创建 WorktreePrepare timeline 节点。
- WorkItemGroup 全部 unit 完成并 ReviewRequest push 后，才运行一次 GroupFinalReview。
- 新链路中 Testing/Tester/单 WorkItem InternalPrReview 不再出现在 UI、prompt、timeline、provider run 和 API 新响应语义中。

### 不要做

- 不按 07-06 全文重新实现已完成内容。
- 不新增独立 tester、testing node 或 analyst node。
- 不为历史 attempt 快照保留新分支或新 UI。
- 不把 Rust、Node、Java 等技术栈规则写进平台固定 prompt；技术栈要求由 07-07 Prompt 协议从任务材料提取。
- 不把单 WorkItem InternalPrReview 改成隐藏步骤；应删除该主链路入口。

## 4. CodeReview 自动返修配置

### 字段语义

CodeReview 配置页新增“自动返修次数”数值设置，对应 attempt 固化字段 `max_auto_rework`。

- 默认值：2。
- 建议范围：0 到 5。
- `0` 表示 CodeReviewer 每次返回 `request_changes` 都直接进入人工返修 gate。
- 该值在创建 attempt 时固化；attempt 运行中不随配置页后续变更漂移。
- 单 WorkItem attempt 与 WorkItemGroup attempt 都使用该配置。

### 配置入口

- 前端位置：Code Reviewer 配置区域。
- 不展示 Tester / Analyst 配置。
- 单 WorkItem 不展示 Internal Reviewer 配置。
- WorkItemGroup 若需要最终审查配置，使用 GroupFinalReview / Group Final Reviewer 语义展示。

### 后端入口

- 创建 attempt 时从请求或 provider config snapshot 中读取自动返修次数。
- 替换当前创建 attempt 时的 `max_auto_rework: 2` 硬编码。
- 返回给前端的 attempt/session state 应能展示当前 `rework_count` 与 `max_auto_rework`。

## 5. CodeReview request_changes 流转

CodeReviewer 输出 `request_changes` 后按以下规则处理：

1. 若 `rework_count < max_auto_rework`：
   - 系统自动复用 coder provider 触发返修。
   - 返修 prompt 带入最新 reviewer findings。
   - 不展示人工 gate。
   - 返修完成后回到 CodeReview。

2. 若 `rework_count >= max_auto_rework`：
   - 创建人工返修 gate。
   - 页面展示 review summary、findings、evidence refs、raw provider output ref。
   - 用户可以补充人工返修意见。
   - 用户确认继续后，系统复用 coder provider 触发返修。
   - 返修完成后回到 CodeReview。

3. 自动返修额度耗尽后不重置：
   - 同一 attempt 后续每次 `request_changes` 都进入人工返修 gate。
   - 不再因为人工返修成功或再次 CodeReview 而恢复自动额度。

4. `blocked`：
   - 停下等待人工处理，不自动返修。

## 6. 人工返修意见优先级

人工返修意见是返修 prompt 的最高优先级输入。

优先级固定为：

1. 人工返修意见。
2. 最新 CodeReviewer findings。
3. 原 Work Item / Final Compile / VerificationPlan。
4. 既有上下文和历史 provider 会话。

当人工返修意见与 reviewer findings 或 Work Item 冲突时，coder 必须遵循人工返修意见，并在最终报告中说明冲突与处理方式。

人工意见来源可以是：

- 人工 gate 表单中的 `extra_context`。
- gate 期间新增的 ContextNote。
- 用户在继续返修动作中输入的补充说明。

实现时应把这些内容合并为本轮 coder delta prompt 的明确章节。

## 7. 单 WorkItem 与 WorkItemGroup 审查边界

### 单 WorkItem

目标流转：

```text
Coding -> CodeReview
  approve -> ReviewRequest(commit+push) -> Completed
  request_changes -> 自动返修或人工返修 gate -> Coding -> CodeReview
  blocked -> 等待人工处理
```

要求：

- CodeReview approve 后只进入 ReviewRequest。
- ReviewRequest push 成功后直接 completed。
- 不创建 InternalPrReview stage gate。
- 不运行 internal reviewer provider。
- 不展示 Internal Reviewer 配置。

### WorkItemGroup

目标流转：

```text
WorktreePrepare（全组一次）
  -> [每个 unit] Coding -> CodeReview -> handoff -> 下一个 unit
  -> 全部 unit 完成
  -> ReviewRequest(commit+push)
  -> GroupFinalReview（全局一次）
  -> Completed
```

要求：

- 每个 unit 的 CodeReview approve 只推进到下一个 unit 或生成 handoff。
- 全部 unit 完成后才 ReviewRequest commit/push。
- push 成功后运行一次 GroupFinalReview。
- GroupFinalReview 不等于单 WorkItem InternalPrReview；实现命名、UI 文案和 prompt 应向组级最终审查收敛。

## 8. WorkItemGroup shared worktree

一个 WorkItemGroup / issue 只能准备一个 shared worktree。

要求：

- 首个 unit 完成 WorktreePrepare 后写入 `worktree_path`。
- 后续 unit 切换时，如果 attempt 已有 `worktree_path` 且路径有效，应跳过 WorktreePrepare，直接进入 Coding。
- 不重复创建 WorktreePrepare timeline 节点。
- 不重复调用 git worktree 创建作为常规流程。
- shared worktree lock 可以随 active unit 转移，但路径保持同一个。

验收重点是 timeline 中只有一个 WorktreePrepare 节点；底层 git worktree 幂等但 timeline 重复仍视为未通过。

## 9. 残留概念清理规则

本 Delta 不要求重复实现 07-06 已完成的删除动作，但要求对当前代码中的残留做事实检查。

### Testing / Tester

如果当前代码仍存在以下新链路可见残留，应删除或迁移：

- Tester provider 配置 UI。
- Testing stage gate。
- Testing provider run。
- TestingReport 新链路展示入口。
- API 新响应中表达新流程仍包含 tester/testing 的字段语义。
- prompt 中要求独立 tester 执行。

若某个函数名包含 testing 但实际承载 coder 自检或报告解析能力，先迁移到新命名/新职责，再删除旧 testing 入口。

### 单 WorkItem InternalPrReview

如果当前代码仍存在以下残留，应删除：

- 单 WorkItem ReviewRequest 后进入 InternalPrReview 的主链路。
- 单 WorkItem Internal Reviewer 配置。
- 单 WorkItem internal review prompt。
- 单 WorkItem internal reviewer provider run。
- 单 WorkItem timeline/report 中的 InternalPrReview 节点。

WorkItemGroup 组级最终审查应使用 GroupFinalReview 语义。若现有 parser 或字段集合短期复用旧结构，必须保证单 WorkItem 入口已删除，且 UI/prompt/运行语义不再暴露为单 WorkItem InternalPrReview。

## 10. Prompt 协议衔接

本 Delta 只定义流程和交互；语言/技术栈自检规则由 `2026-07-07_技术方案_CodingWorkspace材料驱动Prompt协议_v1.0.md` 负责。

实现时：

- coder full prompt 按 Prompt 协议从 Work Item / Final Compile / VerificationPlan / EvaluationContextPack 提取执行清单。
- coder delta prompt 除 Prompt 协议要求外，必须加入本轮 reviewer findings 和人工返修意见。
- code reviewer prompt 按 Prompt 协议从任务材料和 diff 提取审查清单。
- GroupFinalReview prompt 按 Prompt 协议从 completed units、handoff、ReviewRequest、完整 diff 提取整组审查清单。
- 平台固定 prompt 不新增任何语言、包管理器、构建工具、测试框架硬编码。

## 11. 实施建议

实现前先做当前代码事实审计，并在实现报告中列出：

- 已存在且无需重做的能力。
- 本 Delta 要修改的具体文件。
- 当前仍残留的 Testing / Tester / 单 WorkItem InternalPrReview 入口。
- `max_auto_rework` 当前来源与需要替换的硬编码点。
- WorkItemGroup 是否仍会重复 WorktreePrepare timeline。

建议先改后端状态机和 attempt 配置，再改前端配置页和展示，最后补 prompt 与测试。

## 12. 验收标准

1. 07-06 不作为全量待实现清单，代码实现报告不得声称“重新实现 07-06 全流程”。
2. CodeReview 配置页可设置自动返修次数，默认 2，范围 0 到 5。
3. 创建单 WorkItem attempt 和 WorkItemGroup attempt 时，`max_auto_rework` 来自配置，不再硬编码。
4. `request_changes` 在自动返修额度内直接触发 coder 自动返修。
5. 自动返修额度耗尽后，后续 `request_changes` 全部进入人工返修 gate。
6. 人工 gate 展示 reviewer findings，并允许用户补充人工返修意见后继续返修。
7. coder delta prompt 明确包含人工返修意见，并声明人工意见优先级最高。
8. 单 WorkItem CodeReview approve 后 ReviewRequest push 成功即 completed，不运行 InternalPrReview。
9. WorkItemGroup timeline 中 WorktreePrepare 只出现一次。
10. WorkItemGroup 全部 unit 完成并 push 后，只运行一次 GroupFinalReview。
11. 新链路 UI 不展示 Tester / Analyst / 单 WorkItem Internal Reviewer 配置。
12. Prompt 固定模板不引入 Rust、Node、Java 等技术栈硬编码；技术栈要求只来自任务材料或仓库事实。
