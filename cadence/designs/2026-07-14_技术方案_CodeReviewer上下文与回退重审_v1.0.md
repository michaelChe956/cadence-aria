# CodeReviewer 上下文修复与 Run #17 回退重审技术方案

## 目标

修复单 Work Item Code Review 的平台上下文与证据生命周期问题，将 `coding_attempt_0001` 安全回退到 `Coder · Codex · Run #14` 完成、`Code Reviewer · Run #17` 尚未开始的状态，并使用修复后的平台重新执行一次全新的 Code Review。

本方案必须满足以下不变量：

- 不修改业务 worktree 中 Coder Run #14 已完成的代码内容。
- 不删除或改写 Coder Run #14 的输出、验证日志和业务 handoff 文件。
- 不运行 E2E、Playwright 或浏览器自动化测试。
- CodeReviewer 与 GroupFinalReview 继续禁止提出 E2E/Playwright 相关 findings。
- 回退前必须生成可恢复备份并记录业务 diff 指纹。
- 新 Reviewer 必须使用全新 Provider 会话，不能复用 Run #17 会话。

## 已确认问题

### 1. Reviewer 未获得正式 Work Item 上下文

Coder 通过 `coding_execution_context` 从 `LifecycleWorkItemRecord`、Verification Plan 和 Workspace artifact fallback 生成完整 Work Item markdown。

CodeReviewer 当前通过 `work_item_markdown_for_attempt` 只读取 Work Item Workspace 的 artifact version。当前 `workspace_session_0009` 没有 artifact version，因此 Reviewer Prompt 错误写入“未找到 Work Item markdown”，尽管正式 Work Item JSON 已存在。

EvaluationContextPack 同样只从 artifact version 填充 `work_item.raw_markdown_or_sections`，因此字段为空，Reviewer 无法稳定取得：

- Planned Implementation Context
- Exclusive Write Scopes
- Forbidden Write Scopes
- Verification Plan
- Planned Handoff Summary
- Work Item 与 Story/Design 的追踪关系

### 2. 单 Work Item Review 使用累计分支 diff

CodeReviewer 当前相对 `attempt.base_branch=main` 生成 diff。对于 WorkItemGroup，这会把已完成并提交的 Unit 1–5 与当前 Unit 6 混在一起。

单 Work Item Review 应只审查当前 Unit 的增量：

- 当前 Unit 存在前序完成提交时，以当前业务 worktree `HEAD` 或当前 Unit 开始时的基线提交为审查基线。
- 对当前 `coding_unit_0006`，基线为 `640d63d`，即 Work Item 5 完成提交。
- GroupFinalReview 仍使用相对 `main` 的完整分支 diff，不改变整组最终审查语义。

### 3. Review 前 handoff 与 commit 状态被错误解释

当前 WorkItemGroup 生命周期在 Code Review approve 后才执行：

1. 提交当前 Unit 代码。
2. 生成并注册平台侧 Unit handoff。
3. 标记当前 Unit 完成。

因此 Code Review 运行前：

- `completion_commit=null` 是正常状态。
- 平台 Attempt Store 中尚无正式 Unit handoff 是正常状态。
- Coder 在业务 worktree 内写入的 handoff/验证日志可以作为辅助证据，但不能被误认为已注册的平台最终 handoff。

Reviewer Prompt 与 EvaluationContextPack 必须显式表达这一生命周期，不能要求 Coder 在 approve 前完成只会在 approve 后发生的提交和 handoff 注册。

## 方案选择

### 方案 A：只回退 Run #17 后重新 Review

优点是修改最少。缺点是 Reviewer 仍会缺失 Work Item 上下文、使用累计 diff，并继续产生 handoff/commit 假阳性，重复失败概率高。

不采用。

### 方案 B：只修 Prompt 文案并回退

可以消除 handoff/commit 假阳性，但正式 Work Item 数据和 Unit 增量 diff 仍未进入 Reviewer，审查范围仍不可靠。

不采用。

### 方案 C：统一 Reviewer 上下文、Unit diff 与证据生命周期后回退重审

同时修复上下文来源、审查 diff 基线和 Review 前证据语义，再安全回退 Run #17 并启动全新 Reviewer。

采用本方案。

## 平台设计

### 1. 统一 Work Item 上下文生成

提取可复用的 Work Item 上下文构建入口，使 Coder、CodeReviewer、EvaluationContextPack 共享同一来源优先级：

1. 从 `LifecycleWorkItemRecord` 编译正式 Work Item 内容。
2. 加载关联 Verification Plan。
3. 在正式字段缺失或存在补充需要时合并 Workspace artifact snapshot。
4. 保留 Source Draft Supplement 与 Story/Design 追踪信息。

CodeReviewer 不再单独依赖 artifact version。即使 Work Item Workspace 没有 artifact version，也必须从正式 Work Item JSON 得到完整上下文。

EvaluationContextPack 的 `work_item.raw_markdown_or_sections` 使用相同编译结果，避免 Prompt 主上下文和 EvaluationContextPack 相互矛盾。

### 2. 单 Unit 审查 diff 基线

新增明确的 Review diff 选择逻辑：

- `scope=work_item_group` 且处于单 Unit Code Review：使用当前 Unit 开始基线到工作树当前状态的 diff。
- 若当前 Unit 基线不可解析，返回明确平台错误或 context warning，不静默回退到 `main` 累计 diff。
- 单 Work Item Attempt 保持现有相对 base branch 的语义。
- GroupFinalReview 保持完整分支 diff。

当前 Attempt 的 Unit 6 基线必须解析为 `640d63d`。

### 3. Review 前证据状态

EvaluationContextPack 增加或派生审查阶段语义，使 Reviewer 能区分：

- Coder completion report 与验证日志证据。
- Coder 写入业务 worktree 的 draft handoff。
- approve 后由平台生成并注册的 final Unit handoff。
- approve 后生成的 Unit completion commit。

CodeReviewer 协议调整为：

- Review 前 `completion_commit` 为空不得成为 finding。
- Review 前平台 final handoff 缺失不得成为 finding。
- Coder completion report 中缺失必需验证证据仍可成为 finding。
- 已提供但相互矛盾的测试证据仍可成为 finding。
- GroupFinalReview 阶段仍要求所有 completed units 具备正式 handoff 与 completion commit。

### 4. Reviewer 非 E2E 边界

保留现有 Reviewer 与 GroupFinalReview 非 E2E 协议，不修改其允许的单元测试、非浏览器集成测试、编译、构建、类型检查和静态检查范围。

## 回归测试设计

只新增和运行非 E2E 测试。

### Work Item 上下文

- Work Item JSON 存在但 Workspace 无 artifact version 时，CodeReviewer Prompt 仍包含 Planned Implementation Context、Exclusive/Forbidden Write Scopes 和 Verification Plan。
- EvaluationContextPack 的 `work_item.raw_markdown_or_sections` 与 CodeReviewer 主上下文一致。
- Workspace artifact 存在补充内容时，按既有合并规则保留补充快照。

### Unit diff

- WorkItemGroup 当前 Unit 只包含上一 Unit completion commit 之后的增量。
- 已完成 Unit 的文件不会再次出现在当前 Unit Code Review diff 中。
- GroupFinalReview 仍包含从 `main` 开始的完整整组 diff。
- 基线缺失时产生明确诊断，不静默扩大审查范围。

### 证据生命周期

- Code Review 前 `completion_commit=null` 不生成 evidence warning/finding 要求。
- Code Review 前 final handoff 缺失不被描述为 Coder 缺陷。
- Coder required verification evidence 真正缺失时仍能被识别。
- GroupFinalReview 对 completed unit 缺失 handoff/commit 仍保持阻断。

### 非 E2E 边界

- CodeReviewer 与 GroupFinalReview Prompt 均继续包含 E2E/Playwright 禁止协议。
- 不执行任何 Playwright、浏览器环境或 E2E 命令。

## Run #17 安全回退设计

### 回退锚点

目标状态为：

- 保留 `coding_role_run_0030`，Coder Run #14，状态 `completed`。
- 保留 `coding_node_0031`，状态 `completed`。
- 保留 `coding_node_0031_coder_output`。
- 保留 `provider-raw/coding/coder_output_0014.txt` 与 Role Run 事件。
- 保留业务 worktree 当前 13 个修改文件与所有验证日志。
- Unit 6 保持 `running`，尚未 Review approve、尚未生成 completion commit。

回退内容包括：

- `coding_role_run_0031`。
- `coding_node_0032`。
- `code_review_0015`。
- `coding_node_0032_code_review_report`。
- `provider-raw/code_review/code_review_0015.txt`。
- `role-run-events/coding_role_run_0031.jsonl` 及其 artifacts。
- `coding_blocked_gate_0010`。
- Attempt 因 Run #17 产生的 `waiting_for_human`、返修上限 Gate 与更新时间变化。

不回退 Reviewer Run #16 及其后由用户明确授权、Coder Run #14 完成的测试契约迁移。

### 备份与校验

回退前必须：

1. 使用 rollback skill 生成带时间戳的 Attempt 备份。
2. 记录业务 worktree `git status --short`、`git diff --stat` 与 diff 内容哈希。
3. 记录 Coder Run #14 原始输出、验证日志和 handoff 文件哈希。
4. 确认当前没有运行中的 Coder、Reviewer 或相关 Provider 进程。

回退后必须重新计算上述哈希，业务代码指纹必须完全一致。

## 重新 Review

平台修复验证完成并由新后端加载后：

1. 将 Attempt 恢复到 Coder Run #14 完成后的可 Review 状态。
2. 启动一个新的 CodeReviewer Role Run。
3. 禁止复用 Run #17 的 Provider session `29f21c07-793e-4c7c-ba24-b3396ba6c4e6`。
4. 检查新 Prompt：
   - 包含 Work Item 6 正式上下文。
   - 包含 Exclusive/Forbidden Write Scopes。
   - 包含六条 Verification Plan 命令。
   - diff 基线为 Unit 6，而不是 `main`。
   - 不把 pre-review handoff/commit 缺失列为否决理由。
   - 保留非 E2E Reviewer 协议。
5. 对比新 Reviewer 的 raw output、持久化 Review 和 Role Run 状态。

重新 Review 不预设必须 approve。`command_index` 与 `stderr_summary` 若仍与 Design/Work Item 约束冲突，Reviewer 可以基于完整、正确的上下文给出真实 finding。

## 完成标准

- 平台回归测试通过，且未运行任何 E2E/Playwright。
- CodeReviewer Prompt 不再错误显示 Work Item 缺失。
- Unit 6 Code Review 不再携带 Unit 1–5 的累计 diff。
- Review 前 handoff/commit 空值不再形成假阳性 finding。
- Run #17 已从当前活动历史回退，Coder Run #14 及业务 diff 完整保留。
- 新 Reviewer 使用全新 Provider session 完成一次审查。
- 最终汇报新 Review 是真实 approve、request_changes、blocked，还是 Provider/解析失败，并提供三层证据。
