# web-runtime-repository-routing Specification

## Purpose

Web 层所有 runtime repository 解析统一经 `(manifest, selection)` 成对分流，legacy/logical/fail-closed 三态；逻辑状态解析仅走 authority-only 路径（不回退 legacy projection）；稳定错误码 + 4xx 映射；单仓兼容与逻辑代码库 fail-closed 两不误。修复 e3178083 引入的 29 个 it_web 回归。

## ADDED Requirements

### Requirement: 统一分流状态机（REQ-ROUTE-01）
系统 SHALL 使 Web 层 runtime repository 解析统一基于 `(manifest, selection)` 成对状态分流：`(None, None)` → legacy（物理 `RepositoryRecord.id` 解析）；`(Some, Some)` → logical（依据 WorkItem target / attempt snapshot 解析具体成员）；`(Some, None)`、`(None, Some)`、selection 失效、member 删除/停用、target 不在有效 selection、snapshot 与 manifest/checkout 不一致 → fail-closed（稳定错误码 + 明确错误，绝不回退物理仓库或猜测成员）。逻辑状态（manifest/member/target 相关）解析 SHALL 经 authority-only 路径（读 manifest + member + checkout 权威），不一致即 fail-closed，不得进入 legacy projection fallback（`resolve_legacy_projection_if_dual`）。

#### Scenario: 单仓 legacy 分流
- **WHEN** 单仓（无 manifest、无 selection）执行 runtime repository 解析
- **THEN** 系统 SHALL 按物理 `RepositoryRecord.id` 解析（改动前行为），不读取 selection、不报 500

#### Scenario: 逻辑代码库 logical 分流
- **WHEN** 有 manifest + 有 selection（单成员或多成员）
- **THEN** 系统 SHALL 依据 WorkItem `target_repository_id` / attempt snapshot 解析具体成员；多成员且无唯一 target 的 group 操作 SHALL 返回明确业务阻断，不得选择首成员或用 issue repo 猜测

#### Scenario: 逻辑状态 authority-only 解析
- **WHEN** 逻辑状态（有 manifest）下解析成员，且 manifest/member/checkout 任一不一致（如 member 缺失、非 active、checkout 与投影不一致）
- **THEN** 系统 SHALL 经 authority-only 路径 fail-closed（稳定错误码），SHALL NOT 回退 legacy projection fallback（`resolve_legacy_projection_if_dual`），SHALL NOT 静默降级物理仓库

#### Scenario: 状态不一致 fail-closed
- **WHEN** manifest 与 selection 不成对、selection 失效、member 删除/停用、target 不在有效 selection、snapshot 与 manifest/checkout 不一致
- **THEN** 系统 SHALL fail-closed（返回稳定错误码 + 明确错误），不得静默降级为物理仓库

### Requirement: 全入口对齐（REQ-ROUTE-02）
系统 SHALL 使以下 Web runtime repository 解析入口全部经统一分流：`workspace_repository_for_session`、group coding 创建（`coding/group.rs`）、group/attempt 删除（`coding.rs`）、coding WebSocket context（`coding_ws_handler/context.rs`）、`resolve_work_item_repository`、`coding_evaluation_context/builder.rs`、planning resume（`socket.rs`）；compile（`compile_support.rs`）保持既有成对判断并抽取共享判定。各入口对同一 `(manifest, selection)` 状态 SHALL 产生一致结果。lifecycle 涉及 repository 解析的路径（经 `workspace_repository_for_session` 与聚合视图的 `resolve_logical_repository`）随上述入口纳入统一分流；issue 创建时对单仓 `repo_id` 的物理存在性校验保持 legacy 语义，不作为分流入口。

#### Scenario: group coding 创建
- **WHEN** 无 manifest、无 selection 的单仓创建 group coding attempt
- **THEN** 系统 SHALL 按物理仓库解析并创建成功（200），不因 selection 缺失报 500

#### Scenario: 删除 coding attempt
- **WHEN** 单仓（无 selection）删除 schema-v2 group attempt
- **THEN** 系统 SHALL 按物理仓库路由删除（204 NO_CONTENT），不强制 selection

#### Scenario: evaluation context 与 resolve_work_item_repository 同一语义
- **WHEN** `coding_evaluation_context/builder.rs` 或 `resolve_work_item_repository` 在逻辑状态（有 manifest/selection）下解析 repository
- **THEN** 系统 SHALL 仅 `(None, None)` 回退物理 `issue.repo_id`；逻辑状态解析失败 SHALL fail-closed，不得泛化回退物理仓库

#### Scenario: planning resume 与 compile 语义一致
- **WHEN** manifest 与 selection 不成对（如仅有 manifest）
- **THEN** planning resume 与 compile SHALL 返回一致的 fail-closed 结果（不得一个跳过、一个报错）

### Requirement: 单仓回退语义（REQ-ROUTE-03）
系统 SHALL 使逻辑解析仅在「无逻辑代码库状态」时回退物理 `repository_id`；逻辑状态存在（manifest/selection 有任一）时的解析失败 SHALL 为 fail-closed，不得因「任何错误都回退物理仓库」掩盖逻辑状态异常。

#### Scenario: 无逻辑状态才回退
- **WHEN** `(None, None)` 且 WorkItem 有物理 `repository_id`
- **THEN** 系统 SHALL 用物理 `repository_id` 解析
- **WHEN** 有 manifest 且 target 解析失败
- **THEN** 系统 SHALL 返回 blocker（稳定错误码），不得回退物理仓库

### Requirement: IssueCodebaseSelection schema 统一（REQ-ROUTE-04）
系统 SHALL 使新权威 `IssueCodebaseSelection`（`focus_repository_ids` 等字段）成为唯一事实来源；所有 Web reader（group、delete、coding WS context、workspace_repository）SHALL 迁移到权威类型，不再各自反序列化旧 `focus` 字段；旧 JSON 经 serde 兼容读取并重写为新格式；migration executor 的旧 schema reader 对重写后的新格式 SHALL 幂等兼容（版本标记/兼容读取），重跑迁移不得报 `IdentityMismatch`。

#### Scenario: Web reader 使用权威类型
- **WHEN** group / delete / coding WS / workspace_repository 读取 Issue selection
- **THEN** 系统 SHALL 使用 `IssueCodebaseSelectionStore` 的权威类型，旧 JSON 可兼容读取并重写，不产生 schema 分叉

#### Scenario: migration executor 幂等
- **WHEN** selection 文件已重写为新格式后重跑 identity migration
- **THEN** migration executor 的 `write_issue_selection` SHALL 容忍新格式（视为已迁移、跳过重写），不报 `IdentityMismatch`

### Requirement: group target 唯一规则（REQ-ROUTE-05）
系统 SHALL 使 group coding 创建/删除/repair 的 target 解析基于「所有 units 的 `target_repository_id` 集合唯一」规则：集合恰有一个唯一 target 时经 authority-only 解析该成员；集合为空时仅当权威 selection 的 `focus_repository_ids` 恰为单元素才兜底；集合存在多个不同 target 时 SHALL 返回明确 blocker（稳定错误码 `repository_routing_ambiguous`），不得选择首成员、不得用 focus 或 `issue.repo_id` 猜测。

#### Scenario: 多目标 group 无唯一 target 阻断
- **WHEN** 有 manifest + 有 selection 的 group 操作，且所有 units 的 `target_repository_id` 集合存在多个不同值
- **THEN** 系统 SHALL 返回明确业务阻断（`repository_routing_ambiguous`，4xx），不得选择首成员或用 issue repo 猜测

#### Scenario: 唯一 target 解析
- **WHEN** 所有 units 的 `target_repository_id` 集合恰有一个唯一值
- **THEN** 系统 SHALL 经 authority-only 路径解析该唯一成员；成员不存在/非 active 时 SHALL fail-closed（`repository_routing_target_unknown` / `repository_routing_inconsistent`）

### Requirement: 稳定错误码与 HTTP 映射（REQ-ROUTE-06）
系统 SHALL 使 fail-closed 一律以稳定错误码对外并映射 4xx 业务阻断（非 500）：`repository_routing_target_missing`（422，`(Some, None)` 数据不完整）、`repository_routing_inconsistent`（409，孤立 selection / selection 失效 / member 删除停用 / 权威不一致）、`repository_routing_target_unknown`（404，target 不在有效 selection 或非 active）、`repository_routing_ambiguous`（409，多目标无唯一 target）。错误 SHALL 带可诊断信息（kind/id/reason）。映射 SHALL 统一实现于 `src/web/error.rs` 与 `src/web/handlers/support.rs`。

#### Scenario: fail-closed 返回 4xx 稳定错误码
- **WHEN** 任意入口在逻辑状态不一致时 fail-closed
- **THEN** 系统 SHALL 返回稳定错误码（`repository_routing_*`）与对应 4xx HTTP status，SHALL NOT 返回 500，SHALL NOT 静默降级为物理仓库

#### Scenario: 多目标 group 映射 409
- **WHEN** group 操作存在多个不同 target
- **THEN** 系统 SHALL 返回 `repository_routing_ambiguous` 且 HTTP 409 CONFLICT

## 验收（本 capability 的 gate）

- `cargo test --locked --test it_web` 恢复 **313 passed / 0 failed**（基线 commit：main `e3178083`）；新增测试计数规则：验收标准为「基线 313 + 本 change 新增用例」全部通过且 0 failed，禁止通过删除/修改既有用例降低计数维持 313。
- `cargo test --locked --lib` ≥ **1964 passed / 0 failed**（基线 1964 + 新增）；计数规则同 it_web。
- 单仓端到端：创建 coding attempt（`POST /api/projects/{p}/issues/{i}/work-item-plans/{plan}/coding-attempts`）→ **200**；删除（`DELETE /api/projects/{p}/issues/{i}/coding-attempts/{attempt_id}`）→ **204**；repair（对 paused/incomplete 既有 attempt 的计划重 POST，走 plan-repair 恢复路径）→ **200**；无 500。
- 逻辑代码库 fail-closed 保持（不一致状态返回 `repository_routing_*` 稳定错误码 + 4xx，不静默降级）。
- `openspec validate 2026-08-11-fix-web-runtime-repository-routing --strict` valid。
