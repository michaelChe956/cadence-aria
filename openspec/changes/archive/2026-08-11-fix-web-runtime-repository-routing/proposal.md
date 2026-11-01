# Proposal: 修复 Web 运行时统一逻辑代码库分流（回归修复）

## Why

Plan 1 Task 13（commit e3178083）把 Web 层 runtime repository reader 改为「逻辑解析优先」，但**单仓（无 manifest、无 selection）场景未正确回退 legacy**，导致 29 个 it_web 集成测试回归（main 基线 313 passed/0 failed → 当前 29 failed）。单仓用户添加代码库后，创建/repair/删除 coding attempt、plan compile、lifecycle 等操作返回 HTTP 500。

根因不是「多了一个逻辑代码库路径」，而是「单仓路径被破坏」——Web 层多个 reader 在无逻辑代码库状态时，要么无条件读 selection（失败即 500）、要么无差别降级（破坏逻辑代码库 fail-closed）。

## What Changes

引入**统一的 Web 运行时 repository 路由分流**，以 `(manifest, selection)` 成对状态为唯一权威信号：

- 无 manifest + 无 selection → **legacy**（物理 `RepositoryRecord.id` 解析，改动前行为）
- 有 manifest + 有 selection → **logical**（依据 WorkItem target / attempt snapshot 解析具体成员）
- 有 manifest + 无 selection（或反之、selection 失效、member 删除、target 不在 selection）→ **fail-closed**（稳定错误码 + 4xx，绝不静默降级物理仓库）

统一对齐的入口（修复范围）：
- `workspace_repository_for_session`（规划/执行会话仓库解析，唯一 resolve 接口）
- group coding 创建（`coding/group.rs`）、删除（`coding.rs`）
- coding WebSocket context（`coding_ws_handler/context.rs`）
- `resolve_work_item_repository`（普通 WorkItem）
- `coding_evaluation_context/builder.rs`（evaluation context 的 repository 解析，与 `resolve_work_item_repository` 同一语义收紧：仅 `(None, None)` 回退物理）
- compile（`compile_support.rs`，已有成对判断，保持并抽取共享判定）
- planning resume（`socket.rs`，与 compile 语义对齐）

lifecycle 纳入边界：lifecycle 涉及 repository 解析的路径（planning resume / followups 经 `workspace_repository_for_session`、聚合视图按 `target_repository_id` 分组经 `resolve_logical_repository`）随上述入口纳入统一分流；issue 创建时对单仓 `repo_id` 的物理存在性校验保持 legacy 语义，不作为分流入口。

同时统一新旧 `IssueCodebaseSelection` schema：
- 新权威类型（`focus_repository_ids` 等字段）为唯一事实来源
- 迁移所有 Web reader 的本地旧 `focus` 反序列化到权威类型
- 旧 JSON（`codebase-selection.json` 含 `focus`）经 serde 兼容读取并重写为新格式
- migration executor 的旧 schema reader 对重写后的新格式保持幂等兼容（版本标记/兼容读取）

## Capabilities

### New Capabilities

- `web-runtime-repository-routing`：Web 层所有 runtime repository 解析统一经 `(manifest, selection)` 成对分流；legacy/logical/fail-closed 三态；逻辑状态解析仅走 authority-only 路径（不回退 legacy projection）；单仓兼容 + 逻辑代码库安全两不误。

### Modified Capabilities

- `target-aware-work-item`：`target_repository_id` 解析在单仓（无逻辑状态）时回退物理 `repository_id` 的语义明确化（仅无逻辑状态回退，逻辑状态失败即 blocker）。
- `logical-codebase-aggregate-planning`：planning resume 与 compile 的「manifest/selection 不成对」语义对齐（统一 fail-closed）。

## Non-goals

- 不改变逻辑代码库的产品形态（不套目录、不加按钮，纯自动分流）。
- 不做多成员 group coding 的新语义定义（shared worktree 模型下多目标 group 的执行语义属 Plan 4 WP5b，本 change 只保证单仓不回归 + 逻辑代码库 fail-closed；多目标且无唯一 target 的 group 操作返回明确业务阻断）。
- 不新增 provider 能力。

## Impact

- 后端：workspace_repository、coding handlers、coding WS context、compile、planning resume、coding_evaluation_context、IssueCodebaseSelection store、web 错误映射（`src/web/error.rs`、`src/web/handlers/support.rs`）、migration executor（selection 幂等兼容）。
- 测试：it_web 全量恢复（基线 313 + 本 change 新增用例全部通过/0 failed）；1964+ lib 测试保持绿（基线 + 新增）；新增统一分流契约测试（含错误码/HTTP 映射）。
- 交付节奏：本 change 是回归修复，独立于 Plan 4，优先合入。

## 验收标准（本 change 的 gate）

1. `cargo test --locked --test it_web` 恢复 **313 passed / 0 failed**（基线 commit：main `e3178083` 对应 313 passed/0 failed）。新增测试计数规则：验收标准为「基线 313 + 本 change 新增用例」全部通过且 0 failed，禁止通过删除/修改既有用例降低计数维持 313。
2. `cargo test --locked --lib` ≥ **1964 passed / 0 failed**（基线 1964 + 新增）；计数规则同 it_web（基线 + 新增，0 failed）。
3. 单仓流程端到端（具体端点 + 预期 status）：添加代码库 → coding attempt 创建（`POST /api/projects/{p}/issues/{i}/work-item-plans/{plan}/coding-attempts`）→ **200 OK**；删除（`DELETE /api/projects/{p}/issues/{i}/coding-attempts/{attempt_id}`）→ **204 NO_CONTENT**；repair（对存在 paused/incomplete 既有 attempt 的计划重新 POST，走 plan-repair 恢复路径）→ **200 OK**；全程无 500。
4. 逻辑代码库流程不回归：有 manifest 的不一致状态 fail-closed，返回稳定错误码（`repository_routing_*`）与 **4xx** 状态（409/422/404），不 500、不静默降级。
5. `openspec validate 2026-08-11-fix-web-runtime-repository-routing --strict` valid。
