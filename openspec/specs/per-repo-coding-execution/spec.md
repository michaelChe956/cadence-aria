# per-repo-coding-execution Specification

## Purpose
Coding 在目标仓独立 worktree 执行（主 checkout 不动），shared worktree 键升级为 `(project, issue, repository)`，attempt 冻结 target 快照；mixed-target WorkItemGroup 按 target 分流为独立 target-attempt（每 `(plan, target)` 至多一个，单 target 场景零变化，分流契约以 `multi-target-group-coding` 为唯一来源）；交付为推分支 + 人工评审（ReviewRequest），自动 PR 后置。

## Requirements

### Requirement: 单目标仓 worktree 执行（REQ-COD-01）
系统 SHALL 由 Cadence 在目标仓显式创建独立 worktree（`git worktree add`）并将 provider cwd 钉死到该 worktree；主 checkout 的 HEAD 与工作区不被修改；worktree 路径统一为 `<repo>/.worktrees/aria-issues/{issue}`（与现有实现一致）；「主 checkout 不被修改」限定为 HEAD 与工作区，不误述为零 Git 副作用（worktree 会变更 git 元数据）。

#### Scenario: 对 Work Item 启动 coding
- **WHEN** 对 Work Item 启动 coding
- **THEN** 系统 SHALL 先显式创建目标仓 worktree，再以该 worktree 作为 provider cwd 启动；主 checkout 的 HEAD/工作区不受影响

#### Scenario: worktree 隔离不依赖 provider 自觉
- **WHEN** provider 尝试越出 worktree 写入主 checkout 或其他路径
- **THEN** 系统 SHALL 通过 pre/post 越界检测与配置约束（best-effort）标记并阻断；不宣称 OS 级不可达；硬隔离需 provider/OS sandbox（后置）

### Requirement: attempt 冻结 target 快照（REQ-COD-02）
系统 SHALL 使 `CodingExecutionAttempt` 持久化不可变 `target_repository_id`/checkout/revision 与 policy digest；创建、恢复、重放一律使用冻结快照；旧 attempt 缺快照时 display-only 或人工恢复，不得从活 Work Item/`Issue.repo_id` 重新猜测。attempt 冻结快照含三层身份映射(`target_repository_id` 语义类型为 `LogicalRepositoryId`、`RepositoryCheckoutId`、`RepositoryRecord.id` + canonical_path + git_dir_identity + revision + policy_digest);旧 `issue-shared-worktree.json` 经 journal 迁移:验证 record.repository_id 一致性 → 原子写新 record `issues/{issue}/shared-worktrees/{repository_id}.json` → 保留 legacy tombstone/redirect → 仅在无活动引用时清除旧文件;锁 API 增加 repository 参数。

#### Scenario: 创建与恢复 coding attempt
- **WHEN** 创建、恢复或重放 coding attempt
- **THEN** 系统 SHALL 使用 attempt 内冻结的 target 快照；快照缺失时 fail-closed 拒绝启动或进入人工恢复

### Requirement: shared worktree 按三元键（REQ-COD-03）
系统 SHALL 使 Issue shared worktree 的存储、锁、获取/释放/迁移/删除接口全部升级为 `(project, issue, repository)` 键；新路径为 `issues/{issue}/shared-worktrees/{repository_id}.json`，其中 `repository_id` 的 ID 域为 `LogicalRepositoryId`（与 `target_repository_id` 语义类型一致；迁移时旧物理 `RepositoryRecord.id` SHALL 经 migration journal 唯一解析为 logical ID，不混用 ID 域）；锁文件随该 JSON 同址（如 `issues/{issue}/shared-worktrees/{repository_id}.json.lock`）。**锁迁移协议（二审 worker F-6 闭合）**：迁移者 SHALL 先持有 legacy 锁（`.issue-shared-worktree.json.lock`），再按固定全局顺序（如 repository_id 字典序）取得仓维新锁，避免并发迁移死锁；迁移期间通过 journal/redirect 对旧 API 提供可见性；活动 attempt 阻塞迁移直至释放；崩溃恢复与旧锁清理仅在「无活动引用 + redirect 已持久化」双条件满足时执行。旧 `issue-shared-worktree.json` 经 journal 迁移（验证 record.repository_id 一致性→原子写新 record→legacy tombstone/redirect→无活动引用时清除旧文件与旧锁）；同一仓库多个 Work Item 可共享同一仓库级 worktree 并串行，不同仓库可并行。

#### Scenario: 异仓并行
- **WHEN** 同一 Issue 的两个 Work Item 分属不同仓库
- **THEN** 两个仓库的 worktree 可并行创建与执行，互不阻塞

#### Scenario: 同仓串行
- **WHEN** 同一 Issue 同一仓库的多个 Work Item
- **THEN** 复用同一仓库级 shared worktree，由该仓库锁串行化

### Requirement: mixed-target group 按 target 分流（REQ-COD-04）

系统 SHALL 使多成员 Issue 下创建 mixed-target WorkItemGroup 时按 target 分流：为每个目标仓库建立独立的 target-attempt（每 `(plan, target)` 至多一个），MUST NOT 以单一 attempt 承载多 target；「group 内存在多 target」本身 SHALL NOT 构成拒绝理由。分流 SHALL 在创建、恢复、replay 三处及全部路由消费面一致适用。无唯一 target 归属（units 无 `target_repository_id` 且 issue codebase selection focus 不唯一）SHALL 保持 fail-closed 拒绝并返回稳定错误码（TargetMissing 语义）；「group 内多 target」不再返回多目标歧义拒绝（TargetAmbiguous 在 group attempt 路由面退役，selection focus 面等其他消费点语义不变）。同 target 的 group 行为 SHALL 与分流引入前完全一致。分流的完整契约（per-`(plan, target)` 唯一性、单 target 零变化、per-target 冻结快照、手动推进、聚合视图、增殖审计）以 `multi-target-group-coding` capability 为唯一来源，本 requirement 只锁 per-repo-coding-execution 侧的分流义务与拒绝语义变更。

#### Scenario: 尝试 mixed-target group

- **WHEN** 用户对涉及多个目标仓库的 Work Item 建立同一 group
- **THEN** 系统为每个目标仓库分别建立独立 target-attempt（各自冻结该 target 快照、承载该 target 的 units），不再返回多目标歧义拒绝

#### Scenario: 无唯一 target 归属仍 fail-closed

- **WHEN** group 的 units 均无 `target_repository_id` 且 issue codebase selection focus 不唯一
- **THEN** 创建被拒绝并返回稳定错误码，不静默选择任一 target、不创建任何 attempt

#### Scenario: 同 target 的 group 行为零变化

- **WHEN** group 全部 units 属同一目标仓库
- **THEN** 建组、路由与执行行为与分流引入前完全一致：单 attempt、单快照、既有收敛路径不变

### Requirement: 跨仓只读证据检索（REQ-COD-05）
系统 SHALL 使 Coder 可经受控检索接口获取其他成员仓的只读证据（改 A 的接口时查 B 的调用点），证据带 ACL/snapshot/token 预算/审计；Coder 不持有聚合根；跨仓调用点检索为启发式符号/字符串近似（CodeGraph 非 Java 语义级），非精确调用图；「无法写入其他仓」为 best-effort 表述，不宣称 OS 级强制。

#### Scenario: coding 中检索跨仓证据
- **WHEN** coding 过程中 Coder 需要了解其他仓的调用关系
- **THEN** 通过受控检索接口获取只读证据注入上下文；证据记录来源 snapshot 与预算；Coder 的写入配置目标仅为目标 worktree

### Requirement: 每仓独立交付（推分支 + 人工评审）（REQ-COD-06）
系统 SHALL 使每仓独立 branch/commit 并持久化 `ReviewRequest`（GitBranchOnly，含分支/推送状态，无外部 PR URL）。目标仓库已配置交付远端时 SHALL 推送该分支；目标仓库**未配置**交付远端时，系统 SHALL 跳过推送并以本地交付收口，推送状态记为 `skipped_not_configured`。「未配置」的唯一判据是该仓库不存在名为交付远端的远端配置，MUST NOT 以远端不可达、鉴权失败、被拒或无法判定远端状态代替；这些情形 SHALL 保持既有语义（推送被拒记为失败，无法判定则停等人工恢复），MUST NOT 降级为本地交付。

Issue 完成 = 所有必需 Work Item 完成，完成级别为「目标仓分支已推送，或目标仓未配置远端而本地交付，且 ReviewRequest 已生成」；partial failure 显式呈现，不伪装全局成功；自动创建 GitHub/GitLab PR 不在本 change 范围。本地交付 SHALL 在 API 与界面上与「已推送」可区分，MUST NOT 呈现为已推送。Issue 级交付聚合的既有取值 `all_pushed` 名称保持不变，语义为全部必需 Work Item 已交付（含本地交付），不新增取值。

#### Scenario: 多仓 Issue 各仓交付
- **WHEN** 多仓 Issue 各仓 coding 完成
- **THEN** 各仓 SHALL 产出独立分支/提交/推送（或本地交付）与 ReviewRequest 状态；某仓失败时显式呈现 partial failure，不标记 Issue 为已完成

#### Scenario: 交付完成级别
- **WHEN** 判定 Work Item/Issue 完成
- **THEN** 以「目标仓分支已推送，或未配置远端而本地交付，且 ReviewRequest 已生成」为完成级别；未推送、推送失败的 item 不得视为已交付

#### Scenario: 未配置远端的仓库按本地交付收口
- **WHEN** 某目标仓未配置交付远端，其 Work Item 的 coding 与评审通过后进入评审请求阶段
- **THEN** 系统 SHALL 不尝试推送、持久化 ReviewRequest（推送状态 `skipped_not_configured`，无推送错误），其提示 SHALL 说明为本地分支与提交且未配置远端，MUST NOT 声称基于远端发起审查；评审请求阶段 SHALL 标记为完成而非失败，attempt 继续进入后续阶段门

#### Scenario: 已配置远端的失败不降级为本地交付
- **WHEN** 目标仓已配置交付远端，但推送被拒、网络或鉴权失败，或远端状态无法判定
- **THEN** 系统 SHALL 保持既有语义：被拒记为推送失败并保留重推路径，无法判定则停等人工恢复；MUST NOT 记为 `skipped_not_configured`，该 item 不得视为已交付

#### Scenario: 推送中途中断的无远端仓库可收敛
- **WHEN** 某未配置远端的目标仓其评审请求操作已记录「推送已开始」后因进程中断而中止，随后经显式恢复重新进入该阶段
- **THEN** 系统 SHALL 在探测远端状态或推送之前先判定远端未配置，并收敛为本地交付，MUST NOT 因探测一个不存在的远端而再次失败

#### Scenario: 本地交付不自动补推
- **WHEN** 某 Work Item 已按本地交付收口，之后该仓库补配了远端
- **THEN** 系统 MUST NOT 自动推送，也 MUST NOT 重开该 ReviewRequest；既有重推操作仅对推送状态为失败的记录开放

#### Scenario: 本地交付计入 Issue 完成
- **WHEN** Issue 下所有必需 Work Item 均为 Completed，且各自最新 ReviewRequest 的推送状态为 `pushed` 或 `skipped_not_configured`
- **THEN** 交付聚合 SHALL 为全部已交付（`all_pushed`），Issue 可标记完成；其中本地交付的 Work Item 在交付面板中 SHALL 以「本地交付(无远端)」区别于「已推送」

### Requirement: 上游交付注入 coder prompt（REQ-COD-07）

对 work_item_group、存在冻结 target 快照且存在跨仓依赖的 attempt，系统 SHALL 在 coder 的 fresh、delta、rework、teaching 各实际 prompt 组装中追加运行期上游交付清单段（`ResolvedUpstreamDelivery`：上游逻辑 WI/revision、owner attempt、unit run、handoff revision 与 SHA、owner worktree 绝对路径、provided_contracts/capabilities），该段 SHALL 在实际 prompt 记录/claim 前追加。单仓与无依赖 attempt 的 prompt MUST NOT 出现该段。注入信息为运行期 DTO，MUST NOT 写入 immutable handoff、envelope、projection 或 renderer 哈希，MUST NOT 新增 REST/持久字段。

#### Scenario: 下游 prompt 含真实上游交付

- **WHEN** 上游 unit/run 已完成并发布 handoff，而其 attempt 尚待人工确认，下游 coder 构建 fresh/delta/rework/teaching 任一 prompt
- **THEN** 该 prompt 含上游路径、commit SHA 与契约标识（如 CT-001），且不要求上游已最终确认

#### Scenario: 单仓无注入段

- **WHEN** legacy 单仓或无依赖的 group target 构建 coder prompt
- **THEN** prompt 不含上游交付清单段，渲染路径、执行上下文哈希与既有行为一致

### Requirement: 上游不可读走环境阻塞门（REQ-COD-08）

coder 无法读取注入的上游 source_path 时，SHALL 产出 `defect_class=operational_blocker`、`recommended_route=operational_gate` 的 finding（建议 reason_code=`upstream_source_unavailable`，repair_target=null，保留读取命令与路径 evidence），并按环境阻塞门（REQ-CRO-09 的 `coding_operational_blocker`）落门与通知；MUST NOT 落人工输出分诊门，MUST NOT 以契约替身通过替代真实集成验收。

#### Scenario: 上游目录不可读落环境阻塞门

- **WHEN** 注入的上游 worktree 路径不存在或不可读，coder 报告运行环境类 finding
- **THEN** 该 finding 不因 plan 未预声明 blocker_rules 被判无效，attempt 落 `coding_operational_blocker` 门并进入平台通知，用户可再试/说明情况/终止
- **AND** 该 finding 不落 `coding_output_human_triage`
