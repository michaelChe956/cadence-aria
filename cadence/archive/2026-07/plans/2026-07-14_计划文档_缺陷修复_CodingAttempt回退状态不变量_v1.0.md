# Coding Attempt 回退状态不变量修复 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 修正 `coding_attempt_0001` 的活动 Unit 状态，并让 `rollback-coding-attempt` skill 永久避免 `active_unit_id` 指向 `pending` Unit。

**Architecture:** 当前运行数据只修正 Unit 6 的状态，不触碰业务 diff 和已通过 Review。Skill 使用低自由度的明确状态契约，并附带一个可执行 Shell validator，在回退完成后阻止不一致数据进入 UI。

**Tech Stack:** Markdown Skill、Bash、jq、Cadence Aria `.aria` JSON。

## Global Constraints

- 不丢弃或修改 Work Item 6 当前四个业务文件的 diff。
- 不删除 Work Item 6 的 Coding/Review 节点和已通过 Review。
- 不手工提交业务 worktree；由现有 runner 在恢复后完成。
- 不暂存或覆盖 `.superpowers/sdd/final-review-fix-report.md`。
- `.aria` 运行数据保持 Git ignore，不强制加入版本控制。

---

### Task 1: 建立回归测试

**Files:**

- Create: `.codex/skills/rollback-coding-attempt/tests/validate-rollback-state.sh`
- Expected missing implementation: `.codex/skills/rollback-coding-attempt/scripts/validate-rollback-state.sh`

**Interfaces:**

- Consumes: 临时 Attempt JSON 和 `units/coding_unit_*.json` fixture。
- Produces: 对合法状态和三种不一致状态的确定性退出码断言。

- [ ] **Step 1: 写测试脚本**

  测试构造合法的 `running / prepare_context` 状态、活动 Unit 为 `pending`、work_item_id 不匹配、多个活动 Unit 四种 fixture，并调用 validator。

- [ ] **Step 2: 运行测试确认 RED**

  Run: `bash .codex/skills/rollback-coding-attempt/tests/validate-rollback-state.sh`

  Expected: FAIL，因为 `scripts/validate-rollback-state.sh` 尚不存在。

### Task 2: 修正当前 Attempt 数据

**Files:**

- Modify: `.aria/projects/project_0001/issues/issue_0001/coding-attempts/coding_attempt_0001/units/coding_unit_0006.json`
- Verify: `.aria/projects/project_0001/issues/issue_0001/coding-attempts/coding_attempt_0001.json`

**Interfaces:**

- Consumes: Work Item 6 已通过的 Review 和未提交业务 diff。
- Produces: runner 可识别的当前活动 Unit，使其能完成 commit、handoff 和下一 Unit 激活。

- [ ] **Step 1: 备份当前 Attempt 与业务 diff**

  把 Attempt 顶层 JSON、明细目录和 `git diff` 保存到 `/tmp`，记录备份路径。

- [ ] **Step 2: 修正 Unit 6**

  设置：

  ```json
  {
    "status": "running",
    "started_at": "2026-07-13T15:32:18.549045142+00:00",
    "completed_at": null,
    "handoff_ref": null,
    "completion_commit": null,
    "summary": "进入下一个 Work Item",
    "updated_at": "2026-07-13T15:32:18.549045142+00:00"
  }
  ```

- [ ] **Step 3: 验证补救状态**

  断言 Attempt 为 `running / review_request`，指向 Unit 6；Unit 6 为 `running`；业务 worktree 仍只有四个预期文件且 diff 通过 `git diff --check`。

### Task 3: 实现 Skill 状态校验

**Files:**

- Create: `.codex/skills/rollback-coding-attempt/scripts/validate-rollback-state.sh`
- Modify: `.codex/skills/rollback-coding-attempt/SKILL.md`

**Interfaces:**

- Consumes: 回退后的 Attempt JSON 和 units 目录。
- Produces: `rollback_state_ok` 或明确的状态不变量错误码。

- [ ] **Step 1: 实现 validator**

  使用 Bash 和 jq 检查 `running / prepare_context`、指针匹配、目标 Unit 为 `running`、只有一个活动 Unit、完成字段为空。

- [ ] **Step 2: 修正 Skill Step 4、5、7**

  把目标 Unit 标准状态改为 `running`，说明“未开始 Coding”由 `prepare_context` 表达，并要求执行 validator 后才能汇报成功。

- [ ] **Step 3: 运行测试确认 GREEN**

  Run: `bash .codex/skills/rollback-coding-attempt/tests/validate-rollback-state.sh`

  Expected: PASS，并输出 `rollback_skill_state_tests_ok`。

### Task 4: 验证、提交和推送

**Files:**

- Verify: `.codex/skills/rollback-coding-attempt/**`
- Preserve: `.superpowers/sdd/final-review-fix-report.md`

**Interfaces:**

- Consumes: Task 1–3 的结果。
- Produces: 远端 `feat-b-0709` 上可复用的安全回退 skill。

- [ ] **Step 1: 验证 skill**

  运行 Shell 语法检查、回归测试和 `quick_validate.py`。

- [ ] **Step 2: 验证当前运行数据**

  重新读取 Attempt、Unit 6、Review 13 和业务 diff，确认没有丢失。

- [ ] **Step 3: 提交并推送**

  只提交 skill、测试、设计和计划文档，推送 `feat-b-0709`；`.aria` 运行数据和用户 `.superpowers` 文件不进入提交。
