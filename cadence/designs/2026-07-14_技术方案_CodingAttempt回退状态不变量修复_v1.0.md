# Coding Attempt 回退状态不变量修复

## 背景

`coding_attempt_0001` 被回退到 Work Item 6 尚未 Coding 的边界时，`active_unit_id` 和 `current_work_item_id` 仍指向 Unit 6，但 Unit 6 被错误写成 `pending`。平台允许 Coder 和 Reviewer 继续执行，直到 Review 通过后的收尾阶段调用 `get_active_coding_unit`，因为没有状态为 `running`、`waiting_for_human` 或 `blocked` 的 Unit，最终报出 `work_item_handoff_missing`。

## 根因

`rollback-coding-attempt` skill 的 Step 4 把目标 Unit 重置为 `pending`，Step 5 又把同一个 Unit 写入 Attempt 的 `active_unit_id`。这两个步骤互相矛盾。

对于 Work Item Group：

- `active_unit_id` 非空表示存在当前活动 Unit。
- 回退到“目标 Work Item 尚未开始 Coding”时，目标 Unit 已经由前一个 Unit 的完成流程激活，其状态必须是 `running`。
- “尚未开始 Coding”由 Attempt 的 `stage = prepare_context` 表达，不由 Unit 的 `status = pending` 表达。

## 当前数据修复

保留 Work Item 6 已完成的代码和已经通过的 Code Review，不删除节点、不重跑 Coder/Reviewer：

1. 将 `coding_unit_0006.status` 从 `pending` 修正为 `running`。
2. 恢复其原始激活时间、`summary = 进入下一个 Work Item`，保持 `completed_at`、`handoff_ref`、`completion_commit` 为空。
3. Attempt 保持 `running / review_request`，继续指向 Unit 6。
4. 用户再次点击【开始 Coding】后，runner 将跳过 Coding/Review，直接提交当前四个合法文件、生成 handoff、完成 Unit 6，并激活 Unit 7。

## Skill 修复

### 文档规则

修改 `.codex/skills/rollback-coding-attempt/SKILL.md`：

- Step 4 的目标 Unit 固定恢复为 `running`，而不是 `pending`。
- 明确 `prepare_context + running Unit` 才是“尚未开始 Coding”的合法组合。
- Step 5 禁止制造“Attempt 指向活动 Unit，但 Unit 为 pending”的状态。
- Step 7 增加强制状态不变量校验。

### 可执行校验

新增 `scripts/validate-rollback-state.sh`，输入 Attempt JSON 和 units 目录，验证：

- `active_unit_id` 与 `current_work_item_id` 均非空。
- `active_unit_id` 指向的 Unit 文件存在。
- Unit id、work_item_id 与 Attempt 完全匹配。
- 当前 Unit 状态为 `running`，`started_at` 非空，完成和 handoff 字段为空。
- units 中恰好只有一个活动状态 Unit。
- Attempt 为 `running / prepare_context`。

该脚本只校验回退到“目标 Unit 尚未 Coding”后的标准状态；不用于校验当前已经走到 `review_request` 的临时补救状态。

## 测试

新增 shell 回归测试覆盖：

1. 活动 Unit 为 `pending` 时失败，并返回 `active_unit_not_running`。
2. 合法的 `running / prepare_context` 状态通过。
3. Attempt 与 Unit 的 work_item_id 不一致时失败。
4. 存在多个活动 Unit 时失败。

同时运行 skill frontmatter/目录快速校验、Shell 语法检查，以及当前 Attempt 的独立状态断言。

## 非目标

- 不修改平台 Rust 状态机代码。
- 不重新执行 Work Item 6 Coding 或 Code Review。
- 不手工创建 handoff 或提交业务代码；这些仍由平台 runner 完成。
- 不改动 Work Item 6 的写入范围和 Verification Plan。
