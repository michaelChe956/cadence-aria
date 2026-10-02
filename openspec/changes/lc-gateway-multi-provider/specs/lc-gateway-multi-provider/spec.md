# Spec Delta

## Purpose

为 LC 的 Story、Design、Plan、Coding 与 Review 会话提供四家 provider 一致的证据驱动准入、权限投影、恢复与审计契约；证据不足时在进程启动前失败关闭。

## ADDED Requirements

### Requirement: 显式四家 provider gateway 映射

系统 SHALL 为 Claude Code、Codex、Pi、Kimi Code 显式解析 ProviderRefType、ProviderDialect 与 wire dialect，并为每个 provider/action 使用 durable capability 证据；Fake、未知值及未来 provider SHALL NOT 回落到 Claude、legacy direct path 或其它 provider。每个 action 行 SHALL 独立记录 launch、resume、write_boundary 的 Confirmed、Denied(reason) 或 Unknown 状态，以及 exact version、dialect、projection digest、证据引用与 provenance。旧记录缺少此矩阵时 SHALL 解释为 Unknown，不得由旧 supported_actions 或 provenance 自动升级。

#### Scenario: 显式 provider 映射
- **WHEN** LC 请求由 Claude Code、Codex、Pi 或 Kimi Code 启动
- **THEN** 系统 SHALL 使用该 provider 的显式 ref、dialect、wire dialect 和 adapter；未知 provider 与 Fake SHALL 在 spawn 前返回 provider_unsupported_for_gateway_launch

#### Scenario: 未知 capability 阻断
- **WHEN** provider/action 的 launch 或 write_boundary 为 Unknown/Denied，或证据版本与当前 CLI 不符
- **THEN** 系统 SHALL 在 provider spawn 前拒绝并返回对应诊断，不写入成功 capability、不切换 provider

#### Scenario: Fresh 与 resume capability 分离
- **WHEN** provider/action 的 launch 已 Confirmed、resume 为 Unknown，且用户请求 fresh session
- **THEN** fresh launch MAY 继续校验其余门禁后启动；当用户明确请求 resume 时系统 SHALL 因 resume 非 Confirmed 在 spawn 前拒绝，且不得静默改为 fresh

### Requirement: LC admission 与 spawn 前复验

每个 LC provider 会话 SHALL 使用 manifest `provider_context_root` canonical path 作为进程 `working_directory`，并将成员 checkout/worktree 作为独立 target。系统 SHALL 在 validate 与真实 CLI/RPC/ACP child 启动之间重新读取并比较 authority、policy、target identity、cwd、trust、capability、provider projection、adapter availability 与适用的 D4 freshness；任一漂移 SHALL 在 spawn 前拒绝。LC 入口 SHALL 覆盖 Story、Design、Plan/split、Coding、Review 的同步与流式栈，且 SHALL NOT 使用裸 provider input、无政策 legacy bridge、备用 cwd 或 provider fallback。

#### Scenario: validate 后 authority 或投影漂移
- **WHEN** validate 成功后、provider spawn 前 policy/authority/cwd/target/trust/capability/projection/availability 任一冻结事实发生变化
- **THEN** 系统 SHALL 返回可审计的 PolicyDrift 或 ProviderUnavailable，真实 provider SHALL 零启动

#### Scenario: LC Coding target 独立于 root cwd
- **WHEN** LC Coding 会话以成员 worktree 为 target 请求启动
- **THEN** provider 进程 cwd SHALL 仍为 canonical provider_context_root，envelope 的唯一 writable root SHALL 恰为该 target；target 不得替代 cwd，聚合根不得成为 Coding writable root

#### Scenario: 同步与流式入口
- **WHEN** LC 任一规划、split、编码或评审入口启动真实 provider
- **THEN** 同步与流式入口 SHALL 经同一 validated gateway policy；不支持或材料缺失时在启动前失败关闭

### Requirement: 可验证的 provider policy projection

系统 SHALL 从不可由调用方伪造的 gateway projection 将 action、role、语义工具策略、permission/approval、sandbox、MCP/config、trust、cwd、target 与 writable_roots 映射至各 provider 实际 CLI/RPC/ACP 权限，并记录精确 projection digest。逻辑 writable_roots SHALL NOT 被解释为 OS 隔离证明；只有真实 provider/OS boundary evidence 验证 target 可写且 root、非 target、`.git`、`.aria` 等受保护位置不可写后，write_boundary 才可为 Confirmed。

#### Scenario: Projection 与逻辑写根一致
- **WHEN** gateway 为任一 provider/action 生成 projection
- **THEN** adapter SHALL 仅消费与 envelope、provider 版本、dialect、role 和 target 绑定的 projection；审计 SHALL 能关联其 digest 与证据

#### Scenario: Boundary 证据缺失
- **WHEN** target-only 或只读 boundary probe 未证明 provider/OS 的真实权限
- **THEN** 系统 SHALL 将 write_boundary 保持 Unknown 或 Denied 并阻断该 LC action，不得以空 writable_roots、MCP 成功、git status 或模型自述替代证据

### Requirement: Codex 仅受限安全模式

LC gateway SHALL 永久禁止 Codex `danger-full-access`。Planning/Review 仅可使用经当前版本验证的 `read-only` sandbox 与 `on-request` approval；Coding 仅可在 `workspace-write`、process cwd=root、protocol cwd=target、实际写边界 target-only（以及明确允许的临时目录）、trust 与 D4 均得到当前版本证据时放行。任何证据缺失、配置漂移或协议 cwd 不兼容 SHALL fail-closed；UI、resume、permission mode 不得覆盖此硬门。

#### Scenario: danger-full-access 请求
- **WHEN** LC Codex projection 或既有配置请求 danger-full-access
- **THEN** 系统 SHALL 返回稳定错误 `codex_danger_full_access_unsupported` 并在 spawn 前拒绝

#### Scenario: Codex Coding 受限权限未证实
- **WHEN** workspace-write 的实际可写集合、protocol cwd=target、trust、D4 或版本证据不完整
- **THEN** 系统 SHALL 拒绝 Codex Coding，不得回退 danger-full-access、换 cwd 或换 provider

#### Scenario: Codex Planning/Review
- **WHEN** LC Codex 执行 Planning 或 Review
- **THEN** 仅在当前版本确认 read-only + on-request、工具/审批 wire 与 boundary evidence 后启动

### Requirement: Provider resume 与冻结指纹

系统 SHALL 仅在 action 对应 resume=Confirmed，provider 原生会话恢复成功，且冻结 fingerprint 与当前 policy、authority、canonical cwd、target/git-dir/worktree、provider exact version/dialect、capability evidence、语义 tool policy、provider projection、trust、适用的 Aria 注入 MCP bundle digest 全部一致时恢复旧 session。Kimi 原生项目 MCP 自发现不属于 Aria 注入 bundle。resume 不支持或任一 fingerprint 不一致 SHALL 不续接旧 session、不把明确 resume 静默改成 fresh；只有用户明确选择新会话后，系统才能重新执行完整 fresh admission。

#### Scenario: 原生恢复形态
- **WHEN** 用户请求恢复 Claude Code `--resume`、Codex `thread/resume`、Pi session-id/RPC 或 Kimi ACP `session/load`
- **THEN** 仅在对应 action resume=Confirmed 且原生 session id、版本/dialect、projection 与全部冻结指纹一致时恢复

#### Scenario: Resume capability 未确认
- **WHEN** 用户请求 resume 而对应 capability 为 Unknown 或 Denied
- **THEN** 系统 SHALL 在子进程/协议 child 启动前拒绝，返回 resume capability reason，旧会话不被伪称恢复

#### Scenario: Fingerprint 漂移
- **WHEN** 旧会话的 policy、cwd、target、trust、capability、工具策略、projection 或适用 MCP bundle 与当前事实不一致
- **THEN** 系统 SHALL 标记旧会话 superseded 并要求明确新会话操作；fingerprint 判定自身 SHALL NOT 创建新 provider 进程

### Requirement: 按角色守卫 tool policy 并同步 preflight

Claude、Codex、Pi adapter SHALL 在任何进程 spawn、RPC handshake 或 Pi extension 启动前执行双向 `validate_tool_policy_for_role` 语义守卫：Orchestrator/WorkItemSplitter/Reviewer 必须带 `DenyFileWriteBuiltins`，Executor/Handoff 不得带通用 deny policy。Kimi SHALL 拒绝非空通用 ProviderToolPolicy，并只使用其既有 ClientServicePolicy。LC automation role-chain preflight SHALL 与 gateway 使用同一 provider 映射、action capability、trust、projection、Codex 模式、write-boundary、adapter 与 tool-policy 判定；聚合列出全部违规 role/provider/action/reason 和 capability/projection 引用。`gateway_required=false` SHALL NOT 绕过 LC gateway。

#### Scenario: 工具策略角色不匹配
- **WHEN** Claude/Codex/Pi 的角色策略缺失、非法或与角色相反，或 Kimi 收到非空通用 tool policy
- **THEN** adapter SHALL 在 provider child 启动前拒绝并给出稳定原因

#### Scenario: Role-chain 有多个违规项
- **WHEN** automation role-chain 中一个或多个 provider/action 缺少映射、capability、trust、projection、adapter、boundary evidence 或策略不合法
- **THEN** preflight SHALL 一次返回所有可确定违规项及其引用，并 SHALL NOT 启动任何 provider

#### Scenario: 禁用 gateway 标志的 LC 请求
- **WHEN** LC role-chain 带 `gateway_required=false` 请求运行
- **THEN** 系统 SHALL 仍执行完整 gateway/preflight 门禁，不得直连绕过

### Requirement: 四家真实 LC 会话验收证据

四家 provider 的 LC 支持结论 SHALL 由真实会话 E2E 按 Story、Design、Plan/split、Coding、Review 分别建立；fresh 与 resume 分别验收，不得由 CLI 可启动或根级规则/MCP 发现结果推导。每条证据 SHALL 记录 exact CLI 版本、实际 argv/RPC/ACP、canonical cwd 与 target、角色/action、policy/capability/projection/trust digest、工具调用与审批结果、D4 baseline、target 正向写、root/非 target/`.git`/`.aria` 越界负向写、pre/post snapshot。失败场景 SHALL 有零 spawn 证据；某格缺证据即保持 Unknown/Denied，不计为支持。

#### Scenario: 单 provider/action E2E 通过
- **WHEN** 某 provider/action 完成真实 LC fresh E2E（resume 另格）并产生完整全链证据
- **THEN** 系统 SHALL 将该 action 对应 capability 标记为有证据的状态，且报告可从结果追溯所有冻结事实与边界探针

#### Scenario: E2E 越界或证据缺失
- **WHEN** target 写失败、保护位置写成功、审计/快照缺失或失败路径发生 spawn
- **THEN** 该 capability SHALL NOT 标记 Confirmed，且 LC 路由 SHALL 保持阻断
