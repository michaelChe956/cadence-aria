# web-runtime-repository-routing Specification

## Purpose

（主规范沿用途不变，本 delta 把分流从「legacy/logical/fail-closed 三态」收为「logical/fail-closed 两态」）Web 层所有 runtime repository 解析统一经 `(manifest, selection)` 成对分流，只剩 `Logical` 与 `FailClosed` 两种结果；不存在「issue 未归属逻辑代码库就回退物理仓」的变体；稳定错误码 + 4xx 映射不变。

## REMOVED Requirements

### Requirement: 统一分流状态机（REQ-ROUTE-01）

**Reason**: 本条定义的三态（`Legacy` / `Logical` / `FailClosed`）中的 `Legacy` 即单仓语义（issue 未归属 LC → 回退物理仓）。技术方案 §3.1.1 要求 `Legacy` 变体整个移除、`(None, None)` 落 `FailClosed`，本条随之被两态版本取代。

**Migration**: 由本次 ADDED 的「两态分流状态机（REQ-ROUTE-01）」承接（ID 不变）；32 处 `RepositoryRouting::Legacy` 消费点按该条改写。

### Requirement: 全入口对齐（REQ-ROUTE-02）

**Reason**: 本条中「issue 创建时单仓 `repo_id` 物理存在性校验保持 legacy 语义」「无 manifest/无 selection 的单仓创建 group coding attempt 返回 200」两条场景要求保留单仓路径，与去掉单仓入口直接冲突（技术方案 §3.1.1 / §3.1.2）。

**Migration**: 由本次 ADDED 的「全入口对齐（无单仓分支）（REQ-ROUTE-02）」承接（ID 不变）；入口清单不变，两条单仓场景改为 fail-closed 语义。

### Requirement: 单仓回退语义（REQ-ROUTE-03）

**Reason**: 本条的核心是「无逻辑状态时可回退物理 `repository_id`」，这正是要被删除的单仓语义；技术方案 §3.1.1 第 4 点要求老记录返回 `issue_not_bound_to_logical_codebase` 而非静默降级。

**Migration**: 由本次 ADDED 的「无逻辑归属不回退物理仓（REQ-ROUTE-03）」承接（ID 不变）。

### Requirement: 自动化 target 的载体判定与单仓语义（REQ-ROUTE-C5-TARGET）

**Reason**: 本条把「issue 无 LC 归属 → 单仓（Single-Repository）载体」写成载体判定的一支，并要求保留 `RepositoryRouting::Legacy` 分支名（技术方案 §3.1.1 要求删除），8 条场景中有 7 条描述单仓载体行为，全部随入口删除而失效。

**Migration**: 由本次 ADDED 的「自动化 target 的载体判定（REQ-ROUTE-C5-TARGET）」承接（ID 不变）；保留 authority 冲突 fail-closed 与逻辑载体既有约束两条实质内容。

## ADDED Requirements

### Requirement: 两态分流状态机（REQ-ROUTE-01）

系统 SHALL 使 Web 层 runtime repository 解析统一基于 `(manifest, selection)` 成对状态分流，结果 MUST 只有两种：`Logical`（`(Some, Some)`）与 `FailClosed`（其余一切）。`RepositoryRouting` SHALL 只包含这两个变体，MUST NOT 保留单仓语义的 `Legacy` 变体，MUST NOT 保留永不出现的第三臂。

`(None, None)`（issue 未持久归属任何逻辑代码库）SHALL 落 `FailClosed` 并返回稳定错误码，MUST NOT 回退物理仓。

#### Scenario: 双有状态映射逻辑

- **WHEN** 状态为 `(Some, Some)`
- **THEN** 系统 SHALL 判定为 `Logical`，依据 WorkItem target / attempt snapshot 解析具体成员

#### Scenario: 不一致即失败关闭

- **WHEN** 状态为 `(Some, None)`、`(None, Some)`、selection 失效、member 删除/停用、target 不在有效 selection 或 snapshot 与 manifest/checkout 不一致
- **THEN** 系统 SHALL 判定为 `FailClosed`（稳定错误码 + 明确错误），MUST NOT 回退物理仓库、MUST NOT 猜测成员

#### Scenario: 逻辑解析仅读权威

- **WHEN** 解析逻辑状态（manifest/member/target 相关）
- **THEN** 系统 SHALL 经 authority-only 路径解析（读 manifest + member + checkout 权威），不一致即 fail-closed，MUST NOT 进入 legacy projection fallback（`resolve_legacy_projection_if_dual`）

#### Scenario: 无逻辑归属即失败关闭

- **WHEN** issue 无 manifest 且无 selection
- **THEN** 系统 SHALL 返回 `FailClosed`：无归属用 `repository_routing_target_missing` 语义，老记录字段缺失用 `issue_not_bound_to_logical_codebase`；MUST NOT 按物理 `RepositoryRecord.id` 解析、MUST NOT 返回成功

#### Scenario: 逻辑代码库 logical 分流

- **WHEN** 有 manifest + 有 selection（**单成员或多成员，同一路径**）
- **THEN** 系统 SHALL 依据 WorkItem `target_repository_id` / attempt snapshot 解析具体成员；多成员且无唯一 target 的 group 操作 SHALL 返回明确业务阻断，不得选择首成员或用 issue repo 猜测；单成员 LC SHALL 走完全相同的解析路径，MUST NOT 因其成员数为 1 而短路

#### Scenario: 逻辑状态 authority-only 解析

- **WHEN** 逻辑状态（有 manifest）下解析成员，且 manifest/member/checkout 任一不一致（member 缺失、非 active、checkout 与投影不一致）
- **THEN** 系统 SHALL 经 authority-only 路径 fail-closed（稳定错误码），MUST NOT 回退 legacy projection fallback，MUST NOT 静默降级物理仓库

#### Scenario: 枚举无第三态

- **WHEN** 检查 `RepositoryRouting` 的定义与全部 match 臂
- **THEN** 枚举 SHALL 只有 `Logical` / `FailClosed` 两个变体且全部 match 穷尽；MUST NOT 存在以单仓语义命名的残留臂

### Requirement: 全入口对齐（无单仓分支）（REQ-ROUTE-02）

系统 SHALL 使以下 Web runtime repository 解析入口全部经统一分流：`workspace_repository_for_session`、group coding 创建（`coding/group.rs`）、group/attempt 删除（`coding.rs`）、coding WebSocket context（`coding_ws_handler/context.rs`）、`resolve_work_item_repository`、`coding_evaluation_context/builder.rs`、planning resume（`socket.rs`）。任一入口 MUST NOT 自带单仓分支或回退物理仓。

#### Scenario: 各入口结果一致

- **WHEN** 各入口处理同一 `(manifest, selection)` 状态
- **THEN** 各入口 SHALL 产生一致结果（同为 `Logical` 或同一稳定错误码的 `FailClosed`）

#### Scenario: 编译入口抽取判定

- **WHEN** 解析入口为 compile（`compile_support.rs`）
- **THEN** compile SHALL 复用共享的成对判定，MUST NOT 自带第三态分支

#### Scenario: 生命周期纳入分流

- **WHEN** lifecycle 路径解析 repository
- **THEN** lifecycle 涉及 repository 解析的路径（经 `workspace_repository_for_session` 与聚合视图的 `resolve_logical_repository`）SHALL 随上述入口纳入统一分流

#### Scenario: 创建 issue 不再走单仓校验

- **WHEN** 用户创建 issue
- **THEN** 系统 SHALL 要求携带逻辑代码库归属；MUST NOT 提供「无 LC、仅带物理 `repo_id`」的创建路径，MUST NOT 保留单仓 `repo_id` 物理存在性校验作为创建前门

#### Scenario: 缺失归属的 group coding 创建被拒

- **WHEN** issue 无所属逻辑代码库（无 manifest、无 selection）
- **THEN** 创建 group coding attempt SHALL fail-closed 并返回稳定错误码；MUST NOT 按物理仓库解析并返回成功

#### Scenario: 删除 coding attempt

- **WHEN** 删除 schema-v2 group attempt，其 issue 归属某逻辑代码库（单成员或多成员）
- **THEN** 系统 SHALL 经该 LC 的权威解析定位成员与 worktree 后删除；缺失归属时 SHALL fail-closed，MUST NOT 回退物理仓库删除

#### Scenario: evaluation context 与 resolve_work_item_repository 同一语义

- **WHEN** `coding_evaluation_context/builder.rs` 或 `resolve_work_item_repository` 解析 repository
- **THEN** 系统 SHALL 与统一分流同语义；MUST NOT 在任何路径上回退 `issue.repo_id`，解析失败 SHALL fail-closed

#### Scenario: planning resume 与 compile 语义一致

- **WHEN** manifest 与 selection 不成对（如仅有 manifest）
- **THEN** planning resume 与 compile SHALL 返回一致的 fail-closed 结果（不得一个跳过、一个报错）

### Requirement: 无逻辑归属不回退物理仓（REQ-ROUTE-03）

系统 SHALL 只经 issue 持久归属的逻辑代码库解析 repository。任何解析失败（无归属、selection 失效、成员不一致、snapshot 漂移）SHALL 为 fail-closed，MUST NOT 回退物理仓库。durable 记录中 `logical_codebase_id` 缺失 SHALL 返回稳定错误码 `issue_not_bound_to_logical_codebase`，MUST NOT 补默认值或推导为单仓。

#### Scenario: 无归属不回退

- **WHEN** issue 未持久归属任何逻辑代码库
- **THEN** 系统 SHALL 返回 `issue_not_bound_to_logical_codebase` 语义的 fail-closed；MUST NOT 使用物理 `repository_id` 继续解析

#### Scenario: 有归属但解析失败也 fail-closed

- **WHEN** issue 有 LC 归属，但 target 解析失败或 member 不一致
- **THEN** 系统 SHALL 返回 blocker（稳定错误码），MUST NOT 回退物理仓库

#### Scenario: 老记录报错而非静默回退

- **WHEN** 读到 `logical_codebase_id` 缺失的老 durable 记录
- **THEN** 系统 SHALL 返回明确错误 `issue_not_bound_to_logical_codebase`；MUST NOT 补默认值、MUST NOT 推导为单仓

### Requirement: 自动化 target 的载体判定（REQ-ROUTE-C5-TARGET）

自动化 target 投影、Enable、prepare、Enrolled advance 与自动 StartCoding SHALL 以唯一 authority 解析 issue 的逻辑代码库载体：所有 issue 都归属且只归属一个逻辑代码库，因此载体判定 MUST NOT 产生「单仓（Single-Repository）」载体。issue 无归属、其 LC 不存在/已软删、成员与 target 不一致时 SHALL fail-closed 并沿用既有 `repository_routing_*` 稳定错误码与 4xx 映射。

#### Scenario: 权威冲突失败关闭

- **WHEN** 出现 authority 冲突（如成员来源身份解析到同一 git 根但记录不同）
- **THEN** authority 冲突 SHALL fail-closed 并沿用既有 `repository_routing_*` 稳定错误码与 4xx 映射

#### Scenario: 载体唯一为逻辑代码库

- **WHEN** issue 归属某逻辑代码库（单成员或多成员）
- **THEN** 自动化 target 投影 SHALL 解析为该 LC 下的 target（含 logical codebase 与 logical repository 身份）；MUST NOT 产生单仓 target，MUST NOT 携带与该 LC 无关的物理仓身份

#### Scenario: 无归属即拒绝投影与 Enable

- **WHEN** issue 未持久归属任何逻辑代码库
- **THEN** target 投影与 Enable SHALL 返回明确 4xx（`issue_not_bound_to_logical_codebase` 语义）并指明该 issue 未绑定 LC，MUST NOT 写 enrollment

#### Scenario: LC 已删除或成员不一致

- **WHEN** issue 声称归属的 LC 记录不存在、已软删，或其成员与 target 不一致
- **THEN** 系统 SHALL 返回 `repository_routing_inconsistent` / `repository_routing_target_unknown` 类稳定错误码，MUST NOT 自动改绑到其他 LC

#### Scenario: 逻辑载体约束不变

- **WHEN** 处理逻辑代码库载体
- **THEN** 逻辑代码库载体保持既有 gateway、authority-only 与单 target 约束不变
