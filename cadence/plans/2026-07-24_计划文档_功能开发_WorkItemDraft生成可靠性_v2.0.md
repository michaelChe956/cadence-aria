# Work Item Draft Prompt 最小验证实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` or `superpowers:executing-plans` task-by-task. Steps use checkbox syntax.

**Goal:** 保留 Draft 校验失败可见性与 findings 保真，以两个已确认 Case 的完整生产 Prompt 调用本地 Claude Code，并由现有 Parser/Validator 测得首次输出通过率；不在产品中实现评估平台。

**Architecture:** 产品代码只保留既有 Prompt、Parser、Validator、一次有界修复和 Work Item Draft 失败反馈。临时入口只编排四个既有能力：`build_work_item_draft_invocation` 组装完整 Prompt、`claude -p` 首次生成 Draft、`parse_work_item_draft_output` 解析、`WorkItemDraftLocalValidator::validate` 判定。入口不定义或放宽任何格式/校验规则，不进入 `src/`、`tests/`、CI、产品 CLI 或版本控制；仅存在于 `/tmp` 并在本轮验证后丢弃。Case 与提醒规则仅以 Cadence 设计文档和已启用项目规则沉淀。

**Tech Stack:** Rust 2024 临时入口、本机 Claude Code `-p --output-format=json`、既有 Work Item Draft Prompt、Parser 与本地 Validator。

## 全局约束

- 不回退无效 Draft 不可接受门禁、失败告警、`validator_findings` 服务端合并或有效 Draft 的现有确认动作。
- 不新增评估模块、CLI、持久化报告、版本控制 30 场景语料或默认 CI 步骤。
- Prompt 成功仅指首次 Provider 输出经既有 `parse_work_item_draft_output` 成功解析、再由既有 `WorkItemDraftLocalValidator::validate` 返回零个 error finding；自动修复不计入成功率。
- 仅使用 Claude Code；Case A、Case B 各取得 10 个有效首次输出，且必须各为 10/10 `pass` 后停止自动调优并交由人工验证。
- 未达标时仅改变 Prompt 文案一个变量；不得放宽 Validator、Schema 或接受门禁。
- 临时入口只能输出 `case`、`run`、`pass` 或错误码汇总；不得写入、提交或打印完整 Prompt、Claude 原始 Draft、认证信息或目标仓库内容。
- 每次调用使用 `claude -p --no-session-persistence --permission-mode=plan --output-format=json <完整Prompt>`，单次上限 480 秒；实测 Case A/Case B 的成功耗时分别为约 228 秒/394 秒，480 秒为 Case B 保留约 86 秒余量。每次终端结果额外输出不含 Prompt/Draft 的 `elapsed_ms`，用于复核两案耗时。不得使用 `stream-json` stdin。最终成功率批次必须一次只运行一个 Claude Code 调用，避免并发资源竞争污染指标。
- Provider 启动失败、超时或非零退出单独标为 `provider_inconclusive`，不冒充 Prompt 失败，也不消耗 20 个有效样本；单个 Case 最多允许两次替补调用，连续两次或累计第三次同类中断即停止并报告。

---

## Task 1：撤回超出范围的正式评估功能

**Files:**

- Delete by reverting commits: `91ba6d0 feat: add explicit draft quality evaluation`、`21ff5e5 fix: harden draft evaluation reports`
- Preserve: Task 1–4 的 Prompt/Validator/反馈实现与测试

- [x] 先确认未提交的 Task 5 文件只属于评估模块，再恢复它们到当前 `HEAD`，不得触碰 `.superpowers/sdd/progress.md`、pycache、其他 OpenSpec change 或用户文件。
- [x] 按新到旧顺序 revert `21ff5e5`、`91ba6d0`，用 `git diff --check` 确认无空白错误。
- [x] 运行 `rg "work-item-draft-eval|work_item_draft_evaluation" src tests web`，确认评估运行时代码与 CLI 已不存在；保留普通 Draft Prompt、Validator 和反馈路径。
- [x] 运行 `cargo fmt --check` 与 `cargo check --locked`；提交 `revert: remove draft evaluation runtime`。

## Task 2：执行临时 Claude Code Prompt 试运行

**Files:**

- Create only at runtime: `/tmp/aria-draft-prompt-validation/src/main.rs` 与 `/tmp/aria-draft-prompt-validation/runner`（不提交，验证后丢弃）
- Do not modify: `src/`、`web/`、`tests/`、产品 CLI、CI、Schema、Parser、Validator

### 临时入口职责（不包含新的产品逻辑）

1. 仅携带本 Plan 已定义的 Case A 与 Case B Outline 数据，逐次调用当前分支的 `build_work_item_draft_invocation`；不得手工摘要、复用历史 Prompt 或增减 Prompt 段落。
2. 将返回的完整 Prompt 作为单个参数传给本机 `claude -p`，取得本次唯一的 `result` 文本；每次调用都使用新的无持久化会话，且 Claude 仅有 plan 权限。
3. 在内存中按原顺序调用 `parse_structured_output`、`parse_work_item_draft_output` 与 `WorkItemDraftLocalValidator::validate`。任何解析错误、禁用字段、Schema 错误或 Validator error finding 都是本次失败，必须保留准确错误码。
4. 只向终端输出运行序号和结果码；不保存完整 Prompt 或 Draft。入口不执行自动修复，也不修改任何仓库文件。

### 本次测试案例

#### Case A：后端登录会话 Draft（正常基线）

- **Outline**：`outline_backend_session`；无上游依赖。
- **目标**：提供登录会话过期检测与刷新相关 API。
- **专有写入范围**：`src/product/session.rs`、`src/web/session_handlers.rs`；禁止修改 `web/**`。
- **验证**：必需检查使用可信命令 `cargo test --locked --lib session`。
- **首次输出通过条件**：入口的 Parser 成功，Draft 的逻辑身份、标题和 kind 与 Outline 一致；至少一个专有写入范围；必需验证检查原样写入 `verification_plan`；既有 Validator 返回零个 error finding。

#### Case B：紧凑时长格式化函数（仓库现有 `draft_001` 失败回归）

- **Outline**：`outline_implement_compact_duration`；逻辑 Work Item 为 `wi_implement_compact_duration`；该 Draft 在当前 Work Item Workspace 中已是 `validation_failed`，正是截图里没有“确认”按钮的案例。
- **目标**：在 `src/formatCompactDuration.mjs` 提供无副作用的 ESM 命名导出 `formatCompactDuration(totalSeconds)`；`0`、`3599`、`3600`、`86400` 分别格式化为 `00:00`、`59:59`、`01:00:00`、`24:00:00`。
- **边界**：只允许修改 `src/formatCompactDuration.mjs`；禁止修改 `test/formatCompactDuration.test.mjs` 与 `package.json`；负数、浮点数、非安全整数和非数值不在本项契约内。
- **当前失败事实**：旧 Draft 把 `check_red_esm_boundary`、`check_green_esm_boundary`（验证检查 ID）写进任务的 `done_when_refs`，而 `done_when_refs` 只能引用 acceptance criterion ID，触发 `unknown_done_when_ref`；同时三个 `required=true` 的验证检查只有 `manual_instruction`、没有 command，触发 `missing_required_verification_command`。
- **首次输出通过条件**：入口的 Parser 成功；所有 `done_when_refs` 只引用本 Draft 声明的 acceptance criterion ID；若当前 Outline 有可信命令目录，每个 `required=true` 验证检查都提供目录中的非空 command；若目录为空，所有人工检查必须 `required=false` 且存在 `operational_gate` blocker；`verification_plan` 与 canonical contract 的验证检查完全一致；既有 Validator 返回零个 error finding。

- [x] 操作者已确认：以 Case B 替换原定的第二个通用案例；Case B 的历史失败码必须在本轮首次输出统计中归零。
- [x] 先以每个 Case 的一次 dry-run 确认当前代码可组装完整 Prompt；再执行一次真实 Claude Code 调用，确认结果能由既有 Parser/Validator 得到 `pass` 或准确错误码。dry-run 不计入 10 个有效样本；这次真实调用计入第 1 次。
- [ ] 每个案例取得 10 个有效首次输出；每次仅记录首次输出是否通过既有 Parser/Validator、失败码、调用序号与耗时，不保存 Prompt 或模型原文。
- [ ] 每个 Case 必须 10/10 `pass`，两个 Case 均须全数通过。自动修复结果单独记录但不计入首次通过率。
- [ ] 达标后停止自动化操作并向操作者报告聚合结果，进入人工验证；未达标时只修改一个 Prompt 文案变量，先运行现有确定性测试，再对相同案例重跑。

## Task 3：沉淀测试基线与提醒规则

**Files:**

- Create: `cadence/designs/2026-07-24_技术方案_WorkItemDraftPrompt测试基线_v1.0.md`
- Create: `cadence/project-rules/work-item-draft-prompt-validation.md`
- Modify: `cadence/project-rules/README.md`
- Modify: `openspec/changes/improve-work-item-draft-generation-reliability/{design.md,tasks.md,specs/work-item-draft-generation-reliability/spec.md}`

- [x] 固化 Case A、Case B 的 Outline、首次输出 Parser/Validator 判定、480 秒上限、Provider 中断口径与每案 10/10 门槛；不得记录完整 Prompt 或 Draft。
- [x] 将规则列为已启用项目规则：变更 Draft Prompt、Canonical Contract 投影或 Provider 输出结构约束时，交付前必须向操作者提示并获取真实 Claude Code 试运行授权。
- [x] 明确提醒不自动调用 Provider，不创建 CI、Hook、产品 CLI、持久化报告或版本控制语料；同步 OpenSpec 的任务、设计与验收门槛。
- [x] 运行文档一致性检索，确认不再残留旧的单案样本数或旧通过门槛。

## Task 4：交付前验证

- [ ] 运行与本次保留功能相关的 Rust/前端定向回归；记录 Task 4 已知的无关 `web_coding_attempt_api` 404 与完整 Rust/Clippy 的既有阻塞，不将其归因于本次回退。
- [ ] 更新 OpenSpec task 状态和本计划，仅在 Task 1 的 revert 与 Task 2 的新鲜人工授权证据齐全时勾选。
