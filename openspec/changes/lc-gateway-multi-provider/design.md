# Design

## Context

现有 LC gateway 只显式映射 Claude Code 与 Codex；Pi/KimiCode 在 gateway 层不支持，Codex Coding 仍被 `danger-full-access` 硬门阻断。`session-policy-envelope` 已建立 canonical `working_directory`、独立 target、D1-D4、预算门与角色工具策略基础，但 capability 记录尚不能表达 action×resume×write_boundary，且不同 adapter 的真实权限投影不一致。本文依据 proposal 及现有 provider 实测报告设计实施边界；LC 根初始化 recipe 继续固定 Claude。

## Goals / Non-Goals

**Goals:**

- 为 Claude Code、Codex、Pi、Kimi Code 建立显式 ref/dialect/wire adapter 与统一 LC admission。
- 以 action×resume×write_boundary 三态证据控制 fresh/resume、权限投影和 target 写边界。
- 在 validate→spawn 间复验 authority、cwd、target、trust、policy、projection、availability 与 D4 freshness。
- 收紧 Codex，统一 Claude/Codex/Pi tool-policy guard，并使 Kimi 保留 ClientServicePolicy 独立控制面。
- 为五类 LC 会话和失败零 spawn 路径建立可追溯的真实 E2E 证据。

**Non-Goals:**

- 不修改 Claude-only 根初始化 recipe，不改变单仓 direct topology、既有单仓参数或输出契约。
- 不提供 provider 自动切换、无政策 fallback、隐式 StartCoding、多 target 自动编排或新增跨 target 状态机。
- 不把 logical writable_roots、MCP 成功、模型自述或 D4 post-check 当作 OS 写隔离证明。
- 不在本 change 修复 E2E 缺陷 #8 的 policy 正文发布通道；gateway 只消费并校验该前置事实。

## Decisions

### 1. 统一 gateway 边界与 provider 注册

保留 `LogicalCodebaseProviderGateway` 作为 LC 唯一 launch authority，扩展 `ProviderRefType`/`ProviderDialect` 与显式 `ProviderRef::from_provider_name`：ClaudeCode/ClaudeCodeCliV1、Codex/CodexCliV1（wire=`codex-app-server-rpc`）、Pi/PiRpcV1（wire=`pi-rpc`）、KimiCode/KimiAcpV1（wire=`kimi-acp`）。Fake 仅存在 test registry，未知值带 provider 名返回稳定 unsupported。同步 `ProviderAdapter::run` 与 streaming `start` 均接收 gateway 绑定的 validated projection；LC 禁止 legacy stream fallback。

### 2. Capability 版本化为逐 action 三态

将 capability durable 记录升级为版本化 action matrix，每格保存 launch、resume、write_boundary 三个 `Confirmed/Denied(reason)/Unknown`，并关联 exact version、gateway/wire dialect、projection digest、snapshot/probe artifact、provenance 和 trust。读取旧记录缺矩阵时返回 Unknown。fresh 只要求 launch；明确 resume 追加 resume=Confirmed；任何 Unknown/Denied 在 spawn 前失败关闭。能力记录不因 CLI 能启动、MCP 成功或 declared provenance 自动升级。

### 3. Projection 与 cwd/target 不变量

Gateway 从 envelope 生成不可伪造 `ProviderPolicyProjection`，冻结 action/role、tool policy、approval、sandbox、MCP/config、trust、canonical cwd、target、logical roots、provider version/dialect 和 digest。进程 cwd 始终是 manifest `provider_context_root`；target 是独立成员 worktree。Planning/Review logical writable roots 为空，Coding 仅允许单一 canonical target root。投影 digest 同时用于 provider_start 审计和 resume fingerprint。logical root 仅是授权声明，真实 boundary probe 是 Confirmed 的必要条件。

### 4. 四家权限与安全投影

Claude 使用 `--disallowedTools Edit,Write,NotebookEdit` 保护非编码 built-in 工具，Coding 不带 deny token，但须单独完成 boundary evidence；Claude headless MCP 工具 allowlist 必须进入 argv projection。Pi 使用 `--exclude-tools edit,write`，其 terminal/extension/MCP 仍是显式逃逸面，未验证只读边界即 Unknown。Kimi 不消费通用 tool policy，沿 ClientServicePolicy 生成 role policy，Aria-owned MCP bundle 与原生项目 MCP 自发现分开审计；当前 TerminalIsolation 不可用，未有 OS boundary 证据不得放行。四家均从 root cwd 发现规则/MCP，但 discovery evidence 不等于写权限 evidence。

Codex 永久拒绝 `danger-full-access`。Planning/Review 使用经版本和 wire 验证的 `read-only` + `on-request`；Coding 只有 `workspace-write`、process cwd=root、protocol cwd=target、trust、D4 和 target-only probe 均确认时才允许。若 protocol cwd 无法与 root discovery 同时满足，或 workspace-write 包含聚合根，则需要产品拥有的 OS sandbox launcher；native projection 与 launcher 均无证据时保持 Unknown/Denied，不回退。

### 5. Admission、复验与 role-chain

Admission 先验证 LC identity/manifest/authority、policy artifact、显式 provider、action capability、trust、role/tool policy、logical roots、projection/adapter、availability 与 boundary/D4。进入 spawn 前重新读取所有可漂移事实并比较 digest/identity；失败返回 PolicyDrift、ProviderUnavailable 或稳定 capability reason，且零 spawn。`automation_gateway_preflight` 复用同一判定源，聚合所有 role/provider/action 违规并附 capability/projection 引用；`gateway_required=false` 对 LC 无效。

Claude/Codex/Pi adapter 的 `start` 首行执行角色工具策略双向 guard，早于进程或协议 child；Kimi 对非空通用策略 fail-closed。作者/评审必须有 DenyFileWriteBuiltins，Executor/Handoff 不得有通用 deny，保持 D1-D4 的角色语义不变。

### 6. Resume 与失败关闭

能力矩阵分别描述 Claude `--resume`、Codex `thread/resume`、Pi `--session-id`/RPC、Kimi ACP `session/load`。resume fingerprint 包含 policy、authority、canonical cwd、target/git identity、version/dialect、capability snapshot、tool-policy digest、projection digest、trust digest、适用 Aria bundle digest 和 action-row evidence。任一漂移标记旧会话 superseded；明确 resume 不支持时零 spawn，不能静默 fresh。用户明确选择新会话后，重新走完整 fresh admission。

### 7. E2E 证据和 #8 边界

每家 provider 的 Story/Design/Plan/split/Coding/Review fresh 与 resume 作为独立矩阵格，记录实际 argv/RPC/ACP、cwd/target、审批和工具事件、projection/capability/trust digest、D4、快照与 target/越界写探针。失败路径必须证明 provider 未启动。缺陷 #8 的 policy 正文 authority-root 发布建议另立小 change 或归 lc-root-initialization 收尾；本 change 在 admission 中要求 artifact locator 可读且 digest 一致，缺失即等待/阻断，禁止启动时临时复制。

## Risks / Trade-offs

- 四家 CLI 版本和 OS sandbox 行为差异大，真实 probe 成本高；用逐格 Unknown 保证安全，但会暂时减少可用 action。
- root cwd 有利于规则/MCP 发现，却扩大 provider 物理写面；target-only 需要 native 或产品-owned launcher 证据，逻辑 writable root 不能替代。
- capability schema 迁移增加旧记录兼容成本；旧数据一律 Unknown，避免错误升级，需由实施任务提供重探测与可诊断等待。
- Kimi 原生 MCP 与 Aria 注入 bundle 并存，审计边界较弱；明确分层而不伪造统一控制面。
- #8 作为前置 change 可能延迟四家 E2E；保留 fail-closed 是正确的可交付状态，不用 fixture workaround 宣称通过。
- 多版本 CLI、OS launcher 和大 LC 预算仍属实施期证据；未验证项不能写入 Confirmed。
