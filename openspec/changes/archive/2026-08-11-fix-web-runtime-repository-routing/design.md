# Design: 修复 Web 运行时统一逻辑代码库分流

## Context

Plan 1 Task 13（commit `e3178083`）把 Web 层 runtime repository reader 改为「逻辑解析优先」，但**单仓（无 manifest、无 selection）场景未正确回退 legacy**，导致 29 个 it_web 集成测试回归（main 基线 313 passed/0 failed → 当前 29 failed）。单仓用户添加代码库后，创建/repair/删除 coding attempt、plan compile、lifecycle 等操作返回 HTTP 500。

回归根因评审（2026-08-11 回归根因评审，12 点现状核实）结论：根因不是「多了一条逻辑代码库路径」，而是「单仓路径被破坏」——Web 层多个 reader 在无逻辑代码库状态时，要么**无条件读 selection**（`coding/group.rs` 直接读 `codebase-selection.json`，文件不存在即 `ProductStoreError` → HTTP 500），要么**无差别降级**（`workspace_repository.rs`、`builder.rs`、`coding.rs` 的 `or_else` 对任意逻辑错误都回退物理仓库，破坏逻辑代码库 fail-closed）。

同时存在三处结构性不一致：

1. **新旧 selection schema 并存**：Plan 3 权威 `IssueCodebaseSelectionStore` 使用 `focus_repository_ids` 等新字段，而 Web 层三个本地 reader（`coding/group.rs:251-271`、`coding.rs:337-357`、`coding_ws_handler/context.rs:207-224`）仍手写读取旧 `focus` 字段；旧 migration 私有 schema（`migration_types.inc.rs:160-165`）是 `{ included, focus, selection_policy }`。新类型落盘的 selection 在旧 reader 反序列化失败。
2. **compile 与 planning resume 语义冲突**：compile（`compile_support.rs:41-63`）严格要求 manifest 与 selection 成对，任一单独存在即报错；planning resume（`socket.rs:61-88`）把 manifest 或 selection 任一缺失都视为传统单仓跳过校验。
3. **`workspace_repository_for_session` 未按 manifest/selection 路由**：Design/WorkItemPlan 始终只读 `issue.repo_id`；Story/WorkItem 逻辑解析失败即泛化回退物理 id。
4. **逻辑解析内部存在 legacy projection fallback**：`RepositoryStore::resolve_logical_repository_with_source`（`repository_store_parts/resolve.inc.rs:67-131`）在 manifest 缺失、target 不在 member_ids、member 缺失时内部回退 `resolve_legacy_projection_if_dual`。逻辑状态的 authority-only 解析必须与这条内部回退隔离（评审 B1）。

本 change 引入**统一的 Web 运行时 repository 路由分流**，以 `(manifest, selection)` 成对状态为唯一权威信号，统一全部入口的 legacy/logical/fail-closed 三态判定，并统一 selection schema，单仓兼容与逻辑代码库 fail-closed 两不误。

## Goals / Non-Goals

**Goals:**

- 引入唯一持久化路由判定 `RepositoryRouting`：`(manifest, selection)` 成对状态 → legacy / logical / fail-closed 三态，各 Web 入口复用同一判定，不再散点手写 `is_some`/`or_else`/文件读取。
- 修复代表回归：单仓（无 manifest、无 selection）group coding 创建/repair/删除、普通 WorkItem 创建、coding WS context、planning resume 全部按物理 `RepositoryRecord.id` 解析并返回 200/204，恢复 it_web 313 passed/0 failed。
- 统一 `IssueCodebaseSelection` schema：新权威类型为唯一事实来源，所有 Web reader 迁移到 `IssueCodebaseSelectionStore`，旧 JSON（migration 写入的 `{ included, focus, selection_policy }`）经 serde 兼容读取并重写。
- **strict authority-only resolver（B1）**：逻辑状态（manifest/member/target 相关）解析经 authority-only 路径（读 manifest + member + checkout 权威），不一致即 fail-closed，绝不进入 legacy projection fallback（`resolve_legacy_projection_if_dual`）。
- 收紧降级语义：逻辑解析仅在 `(None, None)` 时回退物理；任何逻辑状态不一致（manifest/selection 不成对、selection 失效、member 删除、target 不在有效 selection、snapshot 与 manifest 不一致）一律 fail-closed，带稳定错误码，不静默降级。
- planning resume 与 compile 语义对齐：不成对状态一致 fail-closed。
- 稳定错误码 + HTTP 映射（B3）：fail-closed 以稳定错误码（`repository_routing_*`）对外，映射 4xx 业务阻断，非 500。

**Non-Goals:**

- 不改变逻辑代码库的产品形态（不套目录、不加按钮，纯自动分流）。
- 不做多成员 group coding 的新语义定义（shared worktree 模型下多目标 group 的执行语义属 Plan 4 WP5b，本 change 只保证单仓不回归 + 逻辑代码库 fail-closed；多目标且无唯一 target 的 group 操作返回明确业务阻断）。
- 不新增 provider 能力、不引入 OS 沙箱、不改变 `RepositoryRecord.id` 字节表示。
- 不把 `ProjectRecord` 字段或 `LogicalCodebaseFeature` 作为分流依据（评审 M1：二者均非持久化逻辑代码库状态信号）。
- 不修改 `resolve_logical_repository_with_source` 的 legacy fallback 语义本身；legacy 物理 ID 在 `(None, None)` 时经 `resolve_legacy_physical_repository_if_dual` 保持既有 dual-read 行为。

## Decisions

### 1. 统一 repository 路由状态机：`(manifest, selection)` 成对为唯一权威信号

在 product 层新增唯一路由判定模块 `RepositoryRouting`（`src/product/logical_codebase/repository_routing.rs`），返回显式三态类型，Web 各处不再直接读 JSON：

```rust
/// 稳定错误码（B3）：fail-closed 的机器可读分类；HTTP 映射见 §8（error.rs/support.rs）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepositoryRoutingErrorCode {
    /// (Some, None)：manifest 存在但 selection 缺失 → 数据不完整
    TargetMissing,
    /// (None, Some)：孤立 selection，无 manifest → 数据损坏
    OrphanedSelection,
    /// target 指向不存在/非 active 成员
    TargetUnknown,
    /// 多目标 group 无唯一 target / 无唯一 resolve
    TargetAmbiguous,
    /// manifest/member/checkout/snapshot 权威不一致
    Inconsistent,
    /// 成员删除/停用（tombstone）
    MemberRemoved,
    /// selection 已失效（invalidation）
    SelectionInvalidated,
}

pub enum RepositoryRouting {
    /// (None, None)：无 manifest 且无 selection → 物理 RepositoryRecord.id 解析（改动前行为）
    Legacy { repository_id: String },
    /// (Some, Some)：有 manifest 且有有效 selection → 逻辑解析（WorkItem target / attempt snapshot 决定具体成员）
    Logical { manifest: LogicalCodebaseManifest, selection: IssueCodebaseSelection },
    /// 其余一切不一致状态 → 明确错误，稳定错误码 + 可诊断 reason，绝不静默回退物理仓库
    FailClosed { code: RepositoryRoutingErrorCode, reason: String },
}
```

共享 API 命名/模块路径统一（评审 B6）：`RepositoryRouting` 模块是唯一入口——
- `RepositoryRouting::classify(manifest, selection) -> RepositoryRouting`：纯判定函数（不加载 store，便于单测）。
- `RepositoryRouting::load_for_issue(app_paths, project_id, issue_id) -> Result<RepositoryRouting, ProductStoreError>`：加载辅助，经 `LogicalCodebaseStore::load_manifest` + `IssueCodebaseSelectionStore::load` 后交 `classify`。
- 不使用 `route_repository_state` 等散点命名；`load_for_issue` 不再定义在 `workspace_repository.rs`。

基础状态机（与 compile `compile_support.rs:41-63` 的成对判断一致，作为共享判定基准）：

| manifest | issue selection | 结果 |
|---|---|---|
| 无 | 无 | legacy physical route |
| 有 | 有 | logical route；继续验证 selection、member、checkout、target |
| 有 | 无 | fail-closed：migration/数据不完整（`repository_routing_target_missing`） |
| 无 | 有 | fail-closed：孤立 selection/数据损坏（`repository_routing_inconsistent`） |

fail-closed 的触发集合（不只是不成对）：selection 被 `mark_invalidated`、member 删除/停用（tombstone）、target 逻辑 id 不在有效 selection、authority-only 校验 member active + checkout 与 repository projection 一致失败（见 §2）、attempt `target_snapshot` 与 manifest/checkout 不一致（见 §7）。

判定实现要点（评审 M1）：
- 不做裸 `Path::exists("manifest.json")`；使用 `LogicalCodebaseStore::load_manifest(project_id)` 与 `IssueCodebaseSelectionStore::load(project_id, issue_id)`。
- 不使用 `ProjectRecord` 字段或 `LogicalCodebaseFeature`（内存注入值，非持久化状态）。

### 2. strict authority-only resolver（B1）

逻辑状态（有 manifest）下的成员解析必须经 **authority-only 路径**，不得进入 legacy projection fallback。

现状：`RepositoryStore::resolve_logical_repository_with_source`（`resolve.inc.rs:67-131`）在 manifest 缺失、`logical_id` 不在 `manifest.member_ids`、member 缺失三种情况下内部回退 `resolve_legacy_projection_if_dual`（`resolve.inc.rs:137+`）。逻辑状态（`RepositoryRouting::Logical` 或 `FailClosed` 判定之后）调用它会把「逻辑状态异常」误判为「legacy 窗口」，破坏 fail-closed。

strict resolver 语义：
- 入口：`RepositoryRouting` 的 `Logical` 分支与各 Web resolver 在逻辑状态下的解析一律经 `RepositoryStore::resolve_logical_repository_strict(project_id, logical_id)`（新增，位于 `resolve.inc.rs` 或 `repository_routing.rs`）。
- 读取权威：`LogicalCodebaseStore::load_manifest(project_id)`（必须存在，否则 `repository_routing_inconsistent`）、`load_member(project_id, logical_id)`（必须存在且 `status == Active`，否则 `repository_routing_target_unknown` / `repository_routing_inconsistent`）、`load_checkout(project_id, checkout_id)`（必须存在且与 member/physical repository 投影一致，否则 `repository_routing_inconsistent`）。
- 一致性校验：member 与 checkout 与 repository projection 的交叉校验同 `resolve_logical_repository_with_source` 的 authority 校验块，但**任何一项不一致即 fail-closed**（返回 `RepositoryRoutingErrorCode::Inconsistent` / `MemberRemoved` / `TargetUnknown`），**不调用 `resolve_legacy_projection_if_dual`**。
- 绝不回退物理投影：strict 路径不存在 `resolve_legacy_projection_if_dual` / `resolve_legacy_physical_repository_if_dual` 的调用。legacy 物理解析只允许在 `(None, None)` 的 `Legacy` 分支经 `resolve_legacy_physical_repository_if_dual` / `issue.repo_id` 进行。
- 单成员边界（评审 M2）：manifest 存在但成员数为一的「已迁移单成员 logical codebase」仍走 strict authority-only，不以物理成员数作为 legacy/logical 分流条件。

### 3. 统一入口列表与每入口现状→目标

统一替换的消费点（评审「建议的修复方案调整」§3 所列 + 评审 High 补充）共 8 个：

| 入口 | 文件:行 | 现状 | 目标 |
|---|---|---|---|
| `workspace_repository_for_session` | `src/product/workspace_repository.rs:9-113` | Story/WorkItem 逻辑解析失败泛化回退物理；Design/WorkItemPlan 始终 `issue.repo_id` | 全部经 `RepositoryRouting`；Design/WorkItemPlan 也按 manifest/selection 走 logical；仅 `(None,None)` 回退物理；**唯一 resolve 接口** |
| group coding 创建 | `src/web/handlers/coding/group.rs:65-89,246-272` | `resolve_issue_selection_repository` 无条件读 `codebase-selection.json` 旧 `focus`，无文件即 500（代表回归 B1） | 统一分流：`(None,None)` 单仓按物理解析创建（200）；有 manifest+selection 走 logical（strict resolver）；多目标无唯一 target 返回明确业务阻断（§5，评审 B2） |
| delete / repair group attempt | `src/web/handlers/coding.rs:300-358` | schema-v2 group 走 `resolve_issue_selection_repository`（旧 `focus`，无 fallback）；普通 WorkItem 有物理 fallback | 统一分流：`(None,None)` 单仓删除走 legacy（删除返回 204 NO_CONTENT）；snapshot 优先；逻辑失败 fail-closed（H3） |
| coding WS context | `src/web/coding_ws_handler/context.rs:102-224` | snapshot 优先不回退；schema-v2 group 先读旧 selection 失败回退 legacy；非 group target 逻辑失败回退物理 | 统一分流；读权威 selection（`IssueCodebaseSelectionStore`）；`(None,None)` legacy；逻辑失败 fail-closed（含 snapshot 字段级 validator，§7） |
| `resolve_work_item_repository` | `src/web/handlers/coding.rs:265-298` | target Some/None 逻辑失败均回退物理 | 仅 `(None,None)` 回退物理；有 manifest 时 target 解析失败 fail-closed（H1） |
| `coding_evaluation_context/builder.rs` | `src/product/coding_evaluation_context/builder.rs:202-223,308-327` | 任何逻辑错误均回退 `issue.repo_id`（`or_else` 泛化回退） | 与 `resolve_work_item_repository` 同一语义收紧：仅 `(None,None)` 回退，逻辑失败 fail-closed（评审 High 补充） |
| compile | `src/product/workspace_engine/draft_batch/compile_support.rs:41-63` | 已严格要求 manifest/selection 成对（参考实现） | 保持成对判断，抽取共享判定避免与 resume 漂移（Task 11） |
| planning resume | `src/web/workspace_ws_handler/socket.rs:61-88` | manifest 或 selection 任一缺失即 `Ok(None)` 跳过校验（宽松） | 与 compile 对齐：不成对 fail-closed（H5） |

**compile helper 的定位（评审 High）**：`resolve_logical_work_item_plan_repository_targets`（`compile_support.rs:41-63`）返回 `Option<BTreeMap<LogicalRepositoryId, String>>`——它是 **target map**（compile 校验用：所有 unit target 到物理仓库名的映射），**不是唯一 resolve 接口**；本 change 的**唯一 resolve 接口是 `workspace_repository_for_session`**（经 `RepositoryRouting`）。所有 resolve 统一走 `RepositoryRouting`；compile 只把成对判定抽取为共享判定（`RepositoryRouting::classify` 复用），不承担入口级唯一解析职责。

lifecycle 纳入边界：lifecycle 涉及 repository 解析的路径（planning resume / followups 经 `workspace_repository_for_session`、聚合视图按 `target_repository_id` 分组经 `resolve_logical_repository`）随上述入口纳入统一分流；issue 创建时对单仓 `repo_id` 的物理存在性校验保持 legacy 语义，不作为分流入口。

### 4. 单仓回退语义：仅 `(None,None)` 回退物理

回退物理 `repository_id`（或 `issue.repo_id`）的**唯一**条件是基础路由判定为 `Legacy`，即 `(None, None)`。不得以「logical resolve 返回 Err」作为回退条件（评审 H1）。

- `(None, None)` 且 WorkItem 有物理 `repository_id` → 用物理 `repository_id` 解析（改动前行为）。
- 有 manifest 且 target 解析失败 → 返回 blocker（稳定错误码），**不得**回退物理仓库。
- 已有 `target_snapshot`、`target_repository_id` 或 selection 的 logical operation 却没有 manifest → 视为迁移不完整/数据损坏，fail-closed 并引导修复（评审 M3），不依赖 `issue.repo_id` 继续执行。

入口级 target 优先级（评审「建议的修复方案调整」§2）：
1. attempt 有 `target_snapshot`：使用 snapshot，并验证逻辑 authority；snapshot 存在但 authority 无法验证时失败关闭，不转 legacy（H4）。
2. WorkItem 有 `target_repository_id`：按该 target resolve（strict authority-only）。
3. group attempt：按 §5 的 group target 解析算法得到唯一成员；仅当有效 focus 唯一时才可用 selection focus 兜底；多目标返回明确业务阻断，不选择首成员、不用 issue repo 猜测（B2）。
4. Story：使用 `focus_repository_id`，并验证属于有效 selection。
5. Design/WorkItemPlan：走逻辑分流（有 manifest+selection 时按逻辑解析；`(None,None)` 回退物理），不伪装成 issue 的单物理仓库。
6. 仅当基础状态是 legacy 时，使用 `work_item.repository_id`、`story.repository_id` 或 `issue.repo_id`。

评审 M2 边界：manifest 存在但成员数为一的「已迁移单成员 logical codebase」仍走 logical authority，不以物理成员数作为 legacy/logical 分流条件。

### 5. group target 唯一规则（B2）

group coding 创建/删除/repair 的 target 解析算法（评审 B2，不选首成员、不猜 focus）：

1. 收集权威计划 `authoritative.units` 中**所有** unit 对应 WorkItem 的 `target_repository_id` 集合 `T = { target(unit_i) | unit_i ∈ units }`。
2. `|unique(T)| == 1`：唯一 target，`resolve_logical_repository_strict(project_id, t)` → 该成员。
3. `|unique(T)| == 0`（所有 unit 无 target）：仅当权威 selection 的 `focus_repository_ids` 恰为单元素时兜底解析；否则 `repository_routing_target_missing`。
4. `|unique(T)| >= 2`：**明确 blocker**——返回 `repository_routing_ambiguous`（HTTP 409），**不得**选择首成员、**不得**用 `focus` 或 `issue.repo_id` 猜测。
5. 任何成员不在有效 selection / 非 active → `repository_routing_target_unknown` / `repository_routing_inconsistent`（fail-closed）。

算法置于 `RepositoryRouting` 模块或 `resolve_group_repository` 辅助（Task 6 落地），供 group 创建（Task 6）与 delete/repair（Task 8）复用，避免两处语义漂移。

### 6. selection schema 统一：权威类型为唯一事实

- `IssueCodebaseSelection`（`focus_repository_ids` 等字段，`src/product/logical_codebase/issue_selection.rs:20-39`）为唯一事实来源。
- 删除 Web 层三个本地 `IssueCodebaseSelection` 反序列化结构（`coding/group.rs:251-269`、`coding.rs:337-355`、`coding_ws_handler/context.rs:207-224`），统一只经 `IssueCodebaseSelectionStore::load` 消费 selection。
- 旧 JSON 兼容：migration executor（`migration_executor.inc.rs:545`）写入的旧格式 `{ included, focus, selection_policy: "explicit" }` 与新权威格式字段不同；`IssueCodebaseSelectionStore::load` 增加旧格式兼容读取（versioned/legacy decoder，将 `included`→`included_repository_ids`、`focus`→`focus_repository_ids`、String `selection_policy`→枚举），读取成功后一次性迁移重写为新格式（评审 B3 三选一：一次性迁移/兼容读取后重写/versioned decoder，本设计选「兼容读取 + 迁移期重写」）。
- **migration executor 幂等兼容（B4）**：`migration_executor.inc.rs:540-560` 的 `write_issue_selection` 用旧结构 `IssueCodebaseSelection { included, focus, selection_policy }` 读 `codebase-selection.json`。Task 2 重写为新格式后，该 reader 必须以版本标记/兼容读取容忍新格式（视为已迁移、跳过重写），不得在重跑迁移时对新格式文件报 `IdentityMismatch`。
- `schema_version` 字段保留，用于区分新旧格式，防止 schema 分叉。
- schema 解析错误映射为受控业务错误（4xx/409/422），不以 HTTP 500 暴露。

### 7. snapshot 字段级 validator（B5）

attempt `target_snapshot` 存在时，进行字段级一致性校验（`RepositoryRouting` 模块或 `snapshot_validator` 辅助，Task 7 落地）：

- **checkout 一致性**：snapshot 的 checkout id/路径与权威 `load_checkout` 一致。
- **path 一致性**：snapshot 的 repository path 与权威 checkout 的物理投影路径一致。
- **git identity 一致性**：snapshot 的 git identity（逻辑/物理 id 映射）与权威 member/checkout 一致。
- **membership revision 一致性**：snapshot 记录的成员/selection revision 与当前 manifest/selection 一致；不一致（如成员已删除、revision 已变更）→ fail-closed（`repository_routing_inconsistent`），**不转 legacy**（评审 H4）。

validator 只做校验与 fail-closed 判定，不做降级选择。

### 8. 失败处理：稳定错误码 + HTTP 映射，不静默降级

fail-closed 一律以稳定错误码对外（评审 B3，替代自由字符串 `reason`；`reason` 仅作为可诊断详情保留）：

| 状态/错误 | 稳定错误码（`ApiError.code`） | HTTP status | 说明 |
|---|---|---|---|
| `(Some, None)` manifest 无 selection | `repository_routing_target_missing` | 422 UNPROCESSABLE_ENTITY | 与 compile 对齐 |
| `(None, Some)` 孤立 selection | `repository_routing_inconsistent` | 409 CONFLICT | 数据损坏 |
| selection 已失效（invalidation） | `repository_routing_inconsistent` | 409 CONFLICT | 复用 `mark_invalidated` 语义 |
| member 删除/停用 | `repository_routing_inconsistent` | 409 CONFLICT | 复用 compile `target_member_removed` 语义 |
| target 不在有效 selection / 非 active | `repository_routing_target_unknown` | 404 NOT_FOUND | |
| 多目标 group 无唯一 target | `repository_routing_ambiguous` | 409 CONFLICT | 不选首成员、不猜 focus |
| manifest/member/checkout/snapshot 权威不一致 | `repository_routing_inconsistent` | 409 CONFLICT | strict resolver / snapshot validator |
| schema 解析失败 | 受控 4xx（validation/conflict） | 409/422 | 不 500 |

映射实现：`src/web/error.rs` 的 `ApiError::into_response` match 增加上述稳定错误码 → 4xx；`src/web/handlers/support.rs` 的 `product_store_api_error` 对 routing 相关 `ProductStoreError`（`Ambiguous`/`Conflict`/`IdentityMismatch` 的 routing kinds）映射到稳定错误码。所有 fail-closed 错误都带可诊断信息（`kind`/`id`/`reason`），供回归矩阵定位。

### 9. 验收门禁（本 change 的 gate）

1. `cargo test --locked --test it_web` 恢复 **313 passed / 0 failed**（基线 commit：main `e3178083` 对应 313 passed/0 failed）。新增测试计数规则：验收标准为「基线 313 + 本 change 新增用例」全部通过且 0 failed，禁止通过删除/修改既有用例降低计数维持 313。
2. `cargo test --locked --lib` ≥ **1964 passed / 0 failed**（基线 1964 + 新增）；计数规则同 it_web。
3. 单仓流程端到端（具体端点 + 预期 status）：添加代码库 → coding attempt 创建（`POST /api/projects/{p}/issues/{i}/work-item-plans/{plan}/coding-attempts`）→ **200 OK**；删除（`DELETE /api/projects/{p}/issues/{i}/coding-attempts/{attempt_id}`）→ **204 NO_CONTENT**；repair（对存在 paused/incomplete 既有 attempt 的计划重新 POST，走 plan-repair 恢复路径）→ **200 OK**，无 500。
4. 逻辑代码库流程不回归：有 manifest 的不一致状态 fail-closed，返回稳定错误码（`repository_routing_*`）与 **4xx** 状态，不 500、不静默降级。
5. `openspec validate 2026-08-11-fix-web-runtime-repository-routing --strict` valid。

## Risks / Trade-offs

- [统一判定抽取共享模块改动面大，可能触碰逻辑代码库路径] → 以 compile 的成对判断为基准提取共享判定，逻辑路径行为保持；每个 task 先失败测试再实现，it_web/lib 全量门禁兜底。
- [strict authority-only resolver 移除逻辑状态的 legacy projection fallback，可能暴露既有逻辑状态数据问题] → 这正是 B1 的目标（逻辑状态异常必须显式 fail-closed），以可诊断稳定错误码引导修复；legacy 物理路径仅在 `(None, None)` 可用，语义不变。
- [旧 selection JSON 兼容读取可能误判新格式] → 以 `schema_version` 字段区分新旧格式，缺失/旧格式走 legacy decoder，解析后重写为新格式；migration executor 对新格式幂等（B4）。
- [多成员 group coding 语义未定义] → 本 change 只保证单仓不回归 + fail-closed；多目标 group 返回明确业务阻断（`repository_routing_ambiguous`），不隐式选择成员，语义定义留给 Plan 4 WP5b。
- [`(Some, None)` 等不一致状态从「静默跳过」改为「fail-closed」可能暴露既有数据问题] → 这正是本 change 的目标（不静默降级），以可诊断错误引导修复；resume 与 compile 语义对齐避免入口不一致。
- [既有 8 个 `.inc.rs` EOF 空白告警导致 `git diff --check e3178083..HEAD` 失败] → 与本次路由判定无因果关系（评审 L1），本 change 不引入新告警。

## Migration Plan

1. 合入顺序：本 change 是回归修复，独立于 Plan 4，优先合入（先于多成员 group coding 语义）。
2. 旧 selection JSON（migration 写入的 `{ included, focus, selection_policy }`）：`IssueCodebaseSelectionStore::load` 兼容读取并重写为新格式，存量用户无感迁移；不删除旧文件直至重写成功。
3. migration executor 幂等（B4）：`write_issue_selection` 对已重写的新格式文件容忍（版本标记/兼容读取），重跑迁移不报 `IdentityMismatch`。
4. 回滚：若单仓回归未完全恢复，回退到统一分流前的各入口实现；逻辑代码库路径不受影响（fail-closed 语义保持）。
5. 交付物：本 design + `cadence/plans/2026-08-11_计划文档_修复Web运行时统一分流_v1.0.md` 实施计划；实施完成按 it_web 313/0、lib ≥1964 门禁验收。
