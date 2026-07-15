---
name: rollback-coding-attempt
description: Use when the user asks to roll back / revert a Cadence Aria Coding Attempt to an earlier work-item (unit) boundary, wants to redo a work item from scratch, or a previous rollback left active_unit_id/status inconsistent and caused work_item_handoff_missing.
---

# Rollback Coding Attempt

## 目标

把一个 `CodingWorkspaceEngine`（`scope: work_item_group`）的 Coding Attempt，从当前状态（可能卡在 `waiting_for_human` / `blocked`，也可能只是想重新开始某个 work item）安全地回退到「某个 unit（work item）刚完成、下一个 unit 已激活但尚未进入 Coding」的状态，并清理对应 worktree 分支上的未提交变更。这个边界由 `attempt.stage = prepare_context` 和目标 Unit `status = running` 共同表达。回退动作只操作 `.aria/` 下的持久化 JSON/文本文件和目标 worktree，不涉及重新生成 Work Item Plan、不修改任何 work item 的 `exclusive_write_scopes`/`forbidden_write_scopes`（范围调整是另一件事，不属于本 skill）。

## 何时使用

- 用户提到具体的 `[Coding Attempt #coding_attempt_xxxx]`，并要求"回到 draft N 完成后、draft N+1 还没开始的状态"。
- 用户要求"从某个 work item 重新开始操作"，且明确提到要清理 worktree 分支的未提交变更。
- 用户想撤销一次失败/卡死的 Coding Attempt 尝试，但保留之前已完成 work item 的全部产物和提交记录。

不适用于：重新生成 Work Item Plan（那是 draft/compile 阶段的操作）、修改 work item 的写入范围字段（那是独立的编辑动作）、单纯重试当前 blocked gate（用 gate 自带的 `send_to_coder`/`retry` 动作即可，不需要回退）。

## 推荐输入

```text
帮我处理一下这个 coding attempt [Coding Attempt #coding_attempt_0001]。
目标：回到 work item draft3 审核完成后，work item draft4 还没有开始的状态。
记得当前分支【aria/issues/issue_0001】的未提交变更也需要清理，我从 draft4 重新开始操作。
```

把 `coding_attempt_0001`、`draft3`/`draft4`、`aria/issues/issue_0001` 替换成实际的 attempt id、边界 unit 序号和分支名即可。

## 需要先确认的参数

在动手前，必须从用户描述或 attempt 数据中确认以下三项，缺一不问清楚就不能删数据：

1. **attempt_id**：目标 Coding Attempt 的 id。
2. **边界 unit（保留）**：哪个 unit/work item 完成后的状态要保留（记为 `U_b`）。
3. **目标 unit（重置）**：哪个 unit/work item 要回退成"尚未开始"（记为 `U_t`，通常就是 `U_b` 的下一个 unit）。

如果用户只说了"draft4 还没开始"，`U_b` 默认推断为 draft4 依赖链上最后一个已 `completed` 的 unit，`U_t` 就是 draft4 对应的 unit；如果推断结果和 units 列表状态不吻合（比如 `U_t` 已经是 `completed`，或 `U_b` 和 `U_t` 之间还有别的 unit 处于非 `completed` 状态），必须停下来向用户确认，不能自行假设顺序。

## 前置：定位路径

```bash
# attempt 顶层记录
.aria/projects/{project_id}/issues/{issue_id}/coding-attempts/{attempt_id}.json

# attempt 明细目录
.aria/projects/{project_id}/issues/{issue_id}/coding-attempts/{attempt_id}/
  ├── units/                          # 每个 work item 对应一个 coding_unit_xxxx.json
  ├── timeline-nodes.json             # 所有阶段节点，按顺序排列
  ├── role-runs/                      # coding_role_run_xxxx.json
  ├── role-run-events/                # coding_role_run_xxxx.jsonl
  ├── artifacts/role-run-events/      # coding_role_run_xxxx/ 目录，含 prompt/output 原文
  ├── chat-entries/                   # coding_node_xxxx_{coder_output|code_review_report}.json
  ├── code-reviews/                   # code_review_xxxx.json
  ├── provider-raw/{coding,code_review}/
  ├── stage-gates/                    # coding_stage_gate_xxxx.json
  ├── blocked-gates/                  # 未处理的放在根目录，已处理的在 resolved/ 子目录
  ├── choice-gates/                   # 结构同 blocked-gates
  ├── context-notes/                  # coding_context_note_xxxx.json（人工补充说明，谨慎删除）
  └── rework-instructions/            # coding_rework_instruction_xxxx.json

# 对应的 worktree（真实代码所在处，不在主仓库里）
{attempt.worktree_path}   # 从 attempt 顶层 JSON 的 worktree_path 字段读取
```

## 步骤

### Step 0：检查是否有活跃进程/活跃执行在占用这个 attempt

```bash
ps aux | grep "target/debug/aria" | grep -v grep
```

后端正在跑不是问题（本 skill 不需要重启服务，见下方"关于服务"），但要确认它当前**不是正在处理这个 attempt**（比如前端界面显示某个 role 正在执行，或最新 timeline node/role-run 的 `status` 是 `running` 且时间戳非常新）。

- 如果确实在跑：先让用户停止当前操作，或等待收敛到稳定的等待状态（`waiting_for_human`/`blocked`）再动手。
- 如果 timeline node/role-run 显示 `running` 但时间戳明显是"过去"的（比如超过合理的单轮执行时长，或已知用户在 UI 上点过 abort）：这很可能是**中止未清理干净的残留状态**，见下方「已知问题」，按残留状态处理，不要误判为活跃执行而拒绝操作。

### 关于服务：不需要启动或重启

本 skill 只操作磁盘上的 `.aria/` JSON 文件和目标 worktree，**不需要启动、也不需要重启前后端服务**。`CodingAttemptStore::get_attempt` 每次都是直接 `read_json` 读磁盘文件（`src/product/coding_attempt_store/attempt.rs`），没有内存缓存这个 attempt 的完整状态；后端进程里唯一的内存态是 `CodingRunRegistry`（`src/web/state.rs`），它只记录"这个 attempt_id 当前有没有一个活跃的 runner 在跑"（用于互斥锁和 WebSocket 命令通道），不缓存 attempt 的业务字段。只要 Step 0 确认了没有活跃 runner，直接改磁盘文件即可，用户下次在 UI 打开或刷新这个 attempt 时读到的就是最新状态。

如果用户或环境里没有起服务（比如只是要清理数据、不着急立刻验证），本 skill 全程不需要涉及 `cargo run`/`pnpm dev` 等命令。

### Step 1：核对边界一致性

```bash
BASE=.aria/projects/{project_id}/issues/{issue_id}/coding-attempts/{attempt_id}

jq '[.[] | {id, work_item_id, order_index, status, completed_at, completion_commit}]' \
  "$BASE"/units/coding_unit_*.json
```

确认：

- `U_b` 的 `status == completed`，记下它的 `completed_at`（记为 `boundary_time`）和 `completion_commit`（记为 `boundary_commit`）。
- `U_t` 的 `status` 是 `running` / `blocked` / `waiting_for_human` 之一（不是 `pending`，不是 `completed`）。
- `U_b` 和 `U_t` 之间没有其他 unit，或者中间的 unit 全部 `completed`。

如果 Attempt 的 `active_unit_id` 已指向 `U_t`，但 `U_t.status == pending`，这是非法恢复状态，不是“尚未开始 Coding”的正常表达。必须先按本 skill 的 Step 4/5 恢复为 `running / prepare_context`；不能让 Coder 在这个矛盾状态上继续运行，否则 Review 通过后的 commit/handoff 阶段会报 `work_item_handoff_missing`。

任何一条不满足，停下来向用户确认，不要继续删除。

### Step 2：确定要清理的时间线切点

```bash
jq '[.[] | {id, stage, status, agent_role, started_at, completed_at}]' "$BASE/timeline-nodes.json"
```

找到 `U_b` 完成对应的最后一个 timeline node（通常是 `stage=code_review, status=completed, summary="code review 通过"`，且其 `completed_at` 等于或接近 `boundary_time`）。这个 node 及之前的全部保留；这个 node **之后**的全部属于 `U_t`（及可能的失败重试），是要清理的对象。

同样的思路检查：

- `role-runs/*.json`：`started_at > boundary_time` 的要删。
- `role-run-events/*.jsonl`、`artifacts/role-run-events/<role_run_id>/`：id 对应到上面被删的 role-run 的一并删除。额外检查有没有「孤儿」目录——存在于 `artifacts/role-run-events/` 但在 `role-runs/*.json` 里找不到对应记录的目录，这类通常是失败重试/中断留下的残留，同样按时间判断是否属于 `U_t` 之后。
- `chat-entries/coding_node_{id}_*.json`：`{id}` 属于被删 timeline node 集合的要删。
- `code-reviews/*.json`：`role_run_id` 属于被删 role-run 集合的要删。
- `provider-raw/coding/*.txt`、`provider-raw/code_review/*.txt`：被删 role-run 的 `raw_provider_output_refs` 或被删 code-review 的 `raw_provider_output_ref` 指向的文件要删。
- `stage-gates/*.json`：`created_at > boundary_time` 的要删。
- `blocked-gates/*.json`（根目录下未处理的，不是 `resolved/` 子目录）：`created_at > boundary_time` 的要删——这些正是导致要回退的失败 gate 本身。
- `choice-gates/*.json`（同样只看未处理的）：同上规则。
- `rework-instructions/*.json`：`created_at > boundary_time` 的要删——这类是 review round 自动生成的返修指令（不是人工手写内容，可以直接按时间判断删除，不需要额外向用户确认内容）。
- `context-notes/*.json`：这类是**人工通过 gate 手动补充的说明**，`created_at > boundary_time` 的删除前必须先列出内容给用户看一眼，除非用户已经明确说"全部按边界清理"，否则不要静默删除人工写的内容。

在真正执行 `rm` 之前，先把上面每一类要删除的文件名列出来，做一次目视核对（尤其确认没有把 `boundary_time` 之前的文件误删）。

**已知问题：中止（abort）后状态可能未完全同步。** 如果 `U_t` 这次尝试是被用户在 UI 上点了 abort 中止的，检查对应 role-run 的 `role-run-events/<id>.jsonl` 最后一条事件——如果是 `"event_type":"aborted"`，但该 role-run 的 `role-runs/<id>.json` 里 `status` 字段仍显示 `running`、`completed_at` 仍是 `null`，且对应的 timeline node 状态也仍是 `running`：这是已知的状态不同步现象（abort 事件写入了 jsonl 日志，但没有回写 role-run/timeline-node 的 JSON 记录）。不影响本次回退——这类"卡在 running 但实际已中止"的记录同样按 `started_at > boundary_time` 判断，一并归入要清理的对象即可，不需要为了让状态"看起来正确"而单独修复它。如果用户后续单独反馈这个现象，说明这是 `src/product/coding_workspace_engine/handoffs.rs::handle_abort` 或相关 abort 命令处理路径本身的 bug，值得独立报告，不属于本次回退操作的范围。

### Step 3：执行清理

```bash
BASE=.aria/projects/{project_id}/issues/{issue_id}/coding-attempts/{attempt_id}

# timeline-nodes.json：只保留边界之前的（示例用 id 而非时间比较，实际按 Step 2 定位的切点 id 替换）
jq '[.[] | select(<保留条件>)]' "$BASE/timeline-nodes.json" > /tmp/timeline-nodes.json.new
mv /tmp/timeline-nodes.json.new "$BASE/timeline-nodes.json"

# 逐类删除 Step 2 中列出的具体文件（role-runs / role-run-events / artifacts / chat-entries /
# code-reviews / provider-raw / stage-gates / blocked-gates / choice-gates / rework-instructions）
rm -f  "$BASE"/role-runs/coding_role_run_XXXX.json ...
rm -f  "$BASE"/role-run-events/coding_role_run_XXXX.jsonl ...
rm -rf "$BASE"/artifacts/role-run-events/coding_role_run_XXXX ...
rm -f  "$BASE"/chat-entries/coding_node_XXXX_*.json ...
rm -f  "$BASE"/code-reviews/code_review_XXXX.json ...
rm -f  "$BASE"/provider-raw/coding/coder_output_XXXX.txt ...
rm -f  "$BASE"/provider-raw/code_review/code_review_XXXX.txt ...
rm -f  "$BASE"/stage-gates/coding_stage_gate_XXXX.json ...
rm -f  "$BASE"/blocked-gates/coding_blocked_gate_XXXX.json ...
rm -f  "$BASE"/rework-instructions/coding_rework_instruction_XXXX.json ...
```

### Step 4：恢复目标 unit（`U_t`）为当前活动 Unit

`.aria/.../coding-attempts/{attempt_id}/units/coding_unit_{U_t}.json`：

```json
{
  "status": "running",
  "started_at": "<boundary_time>",
  "completed_at": null,
  "handoff_ref": null,
  "completion_commit": null,
  "summary": "进入下一个 Work Item",
  "updated_at": "<boundary_time>"
}
```

其余字段（`id`、`work_item_id`、`order_index`、`created_at`）不动。如果原始 `started_at` 能证明它就是 `U_b` 完成后首次激活 `U_t` 的时间，也可以保留该值；否则使用 `boundary_time`。

**不要把 `U_t` 写成 `pending`。** 在 Work Item Group 中，前一个 Unit 完成后，下一个 Unit 会先被激活为 `running`，然后 Attempt 才进入 `prepare_context`。这里的“尚未开始 Coding”由 `stage = prepare_context` 表达，而不是由 `status = pending` 表达。

### Step 5：重置 attempt 顶层字段

`.aria/.../coding-attempts/{attempt_id}.json`：

```json
{
  "status": "running",
  "stage": "prepare_context",
  "rework_count": 0,
  "current_work_item_id": "<U_t 对应的 work_item_id>",
  "active_unit_id": "<U_t 的 unit id>",
  "head_commit": "<boundary_commit，不变>",
  "provider_conversations": [],
  "updated_at": "<boundary_time，即 U_b 完成的时间>"
}
```

写入后必须满足以下不变量：

- `active_unit_id` 和 `current_work_item_id` 同时非空。
- `active_unit_id` 指向的 Unit 正好是 `U_t`。
- `U_t.work_item_id == current_work_item_id`。
- `U_t.status == running`，且所有 units 中只有一个活动状态 Unit。
- `attempt.stage == prepare_context`。

如果无法同时满足，停止恢复并还原备份；禁止保留“Attempt 指向 `U_t`，但 `U_t` 为 pending”的半恢复状态。

三个需要跟用户明确说明、不能自行悄悄决定的点：

- **`rework_count` 是否清零**：这个计数器是 attempt 级全局计数，不按 unit 分别计数。如果 `U_b` 之前的 unit 已经消耗过若干次 rework（历史真实值不是 0），保留历史值可能导致 `U_t` 一启动就因为「历史计数已经 ≥ `max_auto_rework`」直接撞上限，连正常返修一次的机会都没有。默认建议清零并向用户说明这个权衡；如果用户要求严格保留历史值，按用户要求执行。
- **`provider_conversations` 如何过滤**：不要整体清空，而是只移除 `last_node_id` 落在被删 node 集合里的条目；如果某个角色（比如 coder）的会话是从 `U_b` 之前就建立、且从未在 `U_t` 的失败尝试里更新过 `last_node_id`，理论上应该保留——但要先确认这种跨 unit 复用会话的场景是否真实存在（一般每个新 unit 会用全新会话，此时清空是安全的）。
- **`head_commit` 一定等于 `boundary_commit`**：不要凭空设成别的值；如果发现两者不一致，说明 worktree 还有别的问题，停下来排查而不是强行覆盖。

### Step 6：清理 worktree 未提交变更

```bash
cd {worktree_path}
git status --short --branch -uall
```

- 如果已经是 `nothing to commit, working tree clean` 且 `HEAD == boundary_commit`：不需要任何操作，直接汇报确认结果即可。
- 如果有未提交变更：先 `git diff --stat` 给用户看一眼改了什么，说明这是不可逆操作，需要用户确认丢弃（`git restore --source=HEAD --staged --worktree .`）还是先暂存（`git stash -u`）留着以后看。**未经确认不要直接丢弃**，即便用户在本次任务开头已经说了"清理未提交变更"——如果 diff 内容和预期（`U_t` 失败尝试留下的修改）不一致，要额外提醒。
- 如果 `HEAD != boundary_commit`：不要自动 `git reset --hard`。先查清楚多出来的提交是什么（`git log boundary_commit..HEAD`），可能是别的 unit 误提交或用户手动操作留下的，跟用户确认后再决定怎么处理。

### Step 7：一致性校验

```bash
BASE=.aria/projects/{project_id}/issues/{issue_id}/coding-attempts/{attempt_id}

jq empty .aria/projects/{project_id}/issues/{issue_id}/coding-attempts/{attempt_id}.json && echo OK
jq empty "$BASE/units/coding_unit_{U_t}.json" && echo OK
jq empty "$BASE/timeline-nodes.json" && echo OK

# 强制状态不变量校验；必须输出 rollback_state_ok 才能继续
bash .codex/skills/rollback-coding-attempt/scripts/validate-rollback-state.sh \
  .aria/projects/{project_id}/issues/{issue_id}/coding-attempts/{attempt_id}.json \
  "$BASE/units"

# role-runs / role-run-events / artifacts 三者数量必须一致
ls "$BASE"/role-runs/ | wc -l
ls "$BASE"/role-run-events/ | wc -l
ls "$BASE"/artifacts/role-run-events/ | wc -l

# timeline 最后一个 node 应该是 U_b 完成对应的那个 completed 节点
jq 'length as $l | .[$l-1]' "$BASE/timeline-nodes.json"

# 不应该再有未处理的 blocked/choice gate 属于 U_t
ls "$BASE"/blocked-gates/*.json 2>/dev/null || echo "OK: none"
ls "$BASE"/choice-gates/*.json 2>/dev/null || echo "OK: none"
```

不需要重启任何服务（原因见前面「关于服务」一节）。如果后端进程当前在跑且用户马上要在 UI 里操作，这一步做完就可以直接让用户刷新页面 / 重新打开这个 attempt。

### Step 8：汇报

向用户说明：

- 删除了哪些类别、各删了多少条（role-run/timeline node/gate/rework-instruction 等计数）。
- 保留了哪些历史记录（尤其是 context-notes 里跨边界保留的部分，如果有）。
- `rework_count`、`provider_conversations` 这两处做了什么决定，以及原因。
- worktree 最终状态（clean / HEAD commit）。
- attempt 现在停在什么状态，用户下一步应该在 UI 里做什么（比如"刷新页面后点击开始编码重新启动 `U_t`"）。
- `validate-rollback-state.sh` 是否输出 `rollback_state_ok`；没有通过就不能宣称恢复完成。
- 如果这次遇到了「已知问题：abort 后状态未同步」，明确告知用户这是产品本身的 bug（不是回退操作导致的），值得单独跟进，但已经在清理过程中一并处理，不影响本次回退结果。

## 安全约束

- 不删除任何早于 `boundary_time` 的记录，不动 `U_b` 及之前的 units、role-runs、code-reviews。
- 不修改任何 work item 的 `exclusive_write_scopes`/`forbidden_write_scopes`——那是独立的编辑动作，不属于回退。
- 不在没有向用户展示 diff 的情况下丢弃 worktree 里的未提交变更（即便不做备份，仍必须让用户看清楚要丢弃的具体内容再确认，`git restore`/`git reset` 不可逆）。
- 遇到边界判断（Step 1）不一致、`head_commit`/`HEAD` 不匹配、或 worktree 有意料之外的改动，停下来问用户，不自行假设。
- 禁止出现 `active_unit_id` 指向 `pending` Unit；`prepare_context + running Unit` 才是目标 Work Item 尚未 Coding 的合法恢复状态。
- **如果回退过程中还需要人工修订 Verification Plan，`commands[].source` 必须严格使用当前数据模型支持的枚举值；当前 `VerificationCommandSource` 只允许 `"provider"`，禁止写入自造值 `"amendment"`。** `jq empty` 只能验证 JSON 语法，不能证明枚举可反序列化；写入后必须确认后端能够重新读取该 Verification Plan。
- 每次删除前先列出具体文件名做目视核对，不要用宽泛的 glob（如 `rm -rf role-runs/*`）一次性清空整个目录——本 skill 默认不做备份，删除前的目视核对是唯一的安全网，务必执行到位。
- 不需要为了"看起来更完整"而额外备份/重启服务；如果用户明确要求本次要备份，按用户要求单独执行，不是默认行为。
