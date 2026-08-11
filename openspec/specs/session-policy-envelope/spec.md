# session-policy-envelope Specification

## Purpose

为逻辑代码库流程的每次 provider run 冻结集中政策/目标/配置快照；真实 adapter 只接受 validated launch policy；路由级 fail-closed；禁止无政策 fallback（experimental + supervised）。

## Requirements

### Requirement: 两层政策结构（REQ-ENV-01）
系统 SHALL 持久化 `AggregatePolicyArtifact`（集中政策正文 + digest + revision）作为事实来源；每次 provider run 生成不可变 `SessionPolicyEnvelope`（policy_id/revision/digest、action、target、read/write roots、provider dialect、config artifact 引用）。本契约覆盖逻辑代码库流程的**全部真实 provider 启动入口**：同步栈 `ProviderAdapter::run(AdapterInput)`（含 work-item split 引擎直接调用）与流式栈 `StreamingProviderAdapter::start(StreamingProviderInput)`（含聚合初始化、聚合规划 provider_drive、coding provider_stream、review）；所有入口须经统一 `LogicalCodebaseProviderGateway` 构造 `ValidatedSessionLaunchPolicy` 后启动。

#### Scenario: 启动逻辑代码库 provider run
- **WHEN** 逻辑代码库流程启动任何真实 provider run（规划/编码/评审/初始化）
- **THEN** envelope SHALL 由 resolver 从 policy artifact 解析并校验，缺省或不一致时 fail-closed 拒绝启动

### Requirement: 适配器只接受 validated launch policy（REQ-ENV-02）
系统 SHALL 使逻辑代码库流程的真实 provider adapter 只接受经 `LogicalCodebaseProviderGateway` 构造的 `ValidatedSessionLaunchPolicy` 启动；关闭真实 provider 的无政策 fallback（legacy `run_streaming` 默认 bridge、coding retry `allow_legacy_stream_fallback: true`）与裸 `StreamingProviderInput`/`AdapterInput` 直接启动，逻辑代码库调用固定 `allow_legacy_stream_fallback=false`；Fake/测试路径经 registry 分层或编译期构造限制隔离，不依赖运行时 `if provider != Fake`。

#### Scenario: 尝试无政策启动
- **WHEN** 逻辑代码库流程试图以裸 input 或 legacy fallback 启动真实 provider
- **THEN** 系统 SHALL 拒绝并返回 fail-closed 错误；Fake/测试路径被类型或 registry 层隔离

### Requirement: action 与写根约束（REQ-ENV-03）
系统 SHALL 使 envelope 区分 `PlanningReadOnly` / `CodingTargetWrite` / `ReviewReadOnly`；`PlanningReadOnly`/`ReviewReadOnly` 无 writable root，`CodingTargetWrite` 的 single writable root 恰为当前 target worktree。root 约束为「配置目标 + best-effort」（pre/post 越界检测 + 启动拒绝），不宣称 OS 级不可写。

#### Scenario: coding target 与 envelope 不一致
- **WHEN** coding run 的 target 与 envelope 不一致
- **THEN** 启动 SHALL 被拒绝（fail-closed）；spawn 前复验 canonical path/git-dir/worktree identity 防 TOCTOU

### Requirement: resume 一致性（REQ-ENV-04）
系统 SHALL 仅当 policy digest、target identity、provider version/dialect、capability snapshot 全部一致时允许 resume；政策被升级/撤销、target 变更或 provider 版本变化时新建会话并使旧会话 superseded；对 planning/coding/review 三类均适用。

#### Scenario: 续接旧 provider session
- **WHEN** 尝试续接旧 provider session 且指纹不一致
- **THEN** 系统 SHALL 拒绝 resume 并启动新会话，旧会话标记 superseded

### Requirement: 禁止降级（REQ-ENV-05）
系统 SHALL 使 provider 能力不满足（无只读模式/无单写支持/版本不符）时路由级 fail-closed（阻塞、不降级）。Codex 当前硬编码 `danger-full-access`（`codex_provider/mod.rs:31`/`session.rs:77,101`），SHALL 经 gateway 路由级阻断（不限 UI 隐藏选项）；在完成受限写配置（首选稳定路径 `sandbox_mode = "workspace-write"`，Beta permission profile 列为后续演进）并通过版本钉定/越界测试前保持 unsupported。capability 持久化 exact version / adapter dialect / evidence 三态（`declared`/`fixture_verified`/`production_verified`），`declared` 级路由标注未验证；路由级 fail-closed 不代表端到端 coding 已达 fail-closed supported。

#### Scenario: 选择 coding provider
- **WHEN** 路由选择 coding provider 且能力不满足
- **THEN** 路由 SHALL 阻塞并返回能力缺失错误，不回落到 `danger-full-access` 或自动宽松模式

### Requirement: 配置来源隔离（REQ-ENV-06）
系统 SHALL 使逻辑代码库流程的真实 provider 使用 Aria-owned、权限受控的 settings/MCP bundle；审计 user/project/local/env/子仓 MCP 合并优先级，隔离未批准的配置与凭据；记录最终 argv 与配置 digest 到 run 审计；托管 settings（managed-settings）优先级高于 Cadence 注入时列为已知 gap。

#### Scenario: 真实 provider 启动
- **WHEN** 逻辑代码库流程启动真实 provider
- **THEN** 使用经校验的 Aria 配置产物，最终参数与配置 digest 写入 run 审计记录；未批准配置/凭据不注入；检测 `/status` Setting sources 是否含 managed settings，含时在 run 审计显式标注「managed-settings 活跃，Aria 注入可能被覆盖」

### Requirement: 每仓最小指针（REQ-ENV-07）
系统 SHALL 使本 change 向每个成员仓发布极薄 AGENTS.md/CLAUDE.md 最小指针（logical codebase ID、repo ID、canonical policy locator、「未加载集中政策前禁止写」声明），经独立 worktree/branch 受控发布并生成 ReviewRequest（非自动 PR），可回滚，处理仓内已有文件的合并；envelope 为权威政策输入，pointer 仅负责发现；pointer 未发布前，现有「完整读取目标根 AGENTS.md/CLAUDE.md」语义先修正为「读取 envelope 校验的聚合政策」（见 project-rule-aware-prompts MODIFIED）。

#### Scenario: 指针受控发布
- **WHEN** 发布每仓最小指针
- **THEN** 使用独立 branch + ReviewRequest（非污染主 checkout、非自动 PR）；已有 CLAUDE.md/AGENTS.md 时按无冲突策略合并或明确冲突

#### Scenario: 指针缺失
- **WHEN** 目标仓最小指针缺失或与政策不一致
- **THEN** coding 由 envelope 校验的聚合政策驱动；指针缺失时按配置策略阻塞或标记，不将指针当作政策正文执行
