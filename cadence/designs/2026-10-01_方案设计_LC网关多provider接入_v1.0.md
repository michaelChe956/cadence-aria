# 方案设计：LC 网关多 Provider 接入（v1.0）

- **Change**：`lc-gateway-multi-provider`
- **日期**：2026-10-01
- **状态**：为后续 OpenSpec proposal 接地；本文件只写设计，不改产品代码
- **工作区**：`feat-b-0808-add-monorepo`
- **依赖**：`lc-root-initialization` Phase 2 的 root-cwd / target 合同落地后实施
- **范围**：让 Claude Code、Codex、Pi、Kimi Code 四家 provider 均能在 LC（逻辑代码库）会话中经统一 gateway 运行 Story、Design、Plan、Coding、Review；非 LC 单仓 direct 路径零改动
- **前置事实**：用户 2026-10-01 裁决；`cadence/designs/2026-09-30_方案设计_LC根初始化_v1.0.md` v1.3；`cadence/reports/2026-10-01_预研报告_LC根初始化provider实测_v1.0.md`

> 本文是设计接地，不是实现完成声明。真实 CLI 版本、OS sandbox、写边界和 resume 能力均以实施期 probe 与 E2E 证据为准；证据不足时保持 `Unknown` 并 fail-closed。

## 1. 目标与锁定边界

### 1.1 用户裁决

1. LC 根初始化 recipe 只由 Claude Code 执行，属于 `lc-root-initialization`，不在本 change；本 change 不新增初始化 provider，也不改变其五步 durable operation。
2. 后续 LC 会话的 story/design/plan/coding/review 四家均要支持；本 change 负责 Codex、Pi、Kimi 的 gateway 接入，并把 Claude 一并迁移到同一 capability/preflight/policy 口径。
3. root-cwd 模型固定：provider 进程 `working_directory` 是 manifest `provider_context_root` 的 canonical path；`target` 是独立的 member/checkout/worktree；`writable_roots` 是逻辑授权，不是 OS 隔离证明。
4. Codex/Kimi trust 是 recipe 前的独立前置门；本 change 只消费并在每次 LC session spawn 前复验，不把 trust 登记塞进本 change 的 recipe 步骤。
5. REQ-MTG-03 保持不变：不引入多 target 自动化、不按依赖自动拉起其它 target-attempt、不让一个 run 承载多个 target。

### 1.2 成功定义

某 provider/action 只有在以下事实全部成立时才可在 LC 路由放行：

- `ProviderName` 有显式 `ProviderRefType` 和 `ProviderDialect`，无静默回退；
- capability store 有该 provider、该 action 的生产证据，resume 请求另有 `Confirmed` 证据；
- authority root、policy、target、cwd、trust、配置和 capability 在 admission 与 spawn 前均一致；
- adapter 能把 action、role、tool policy、permission mode、target/write boundary 投影到真实 CLI/RPC/ACP 权限；
- tool-policy guard 和 provider-specific guard 在子进程创建前通过；
- Coding 有 D4 cross-target baseline，结束后有 target 写入、非 target 不变和越界尝试证据；
- Story/Design/Plan/Coding/Review 的真实 LC E2E 证据齐全，失败路径证明 provider 未 spawn。

CLI 能启动、MCP 能调用、模型自述“没有写文件”都不足以构成支持证据。

## 2. 当前源码事实与证据边界

### 2.1 Gateway 当前只认 Claude/Codex

`src/product/logical_codebase/provider_gateway.rs:143-156` 的 `ProviderRef::from_provider_name` 仅映射 `ClaudeCode`、`Codex`；`Pi`、`KimiCode`、`Fake` 返回 `UnsupportedCapability(PROVIDER_UNSUPPORTED_FOR_GATEWAY_LAUNCH)`。`ProviderRefType` 在 `:161-164` 也仅有两项。不能用 `_ => ClaudeCode` 修复，必须扩展显式 provider 和 dialect。

### 2.2 policy、capability、目标合同已有骨架

- `src/product/logical_codebase/policy.rs:130-195` 已有 `ProviderDialect::{ClaudeCodeCliV1,CodexCliV1}`、`SessionPolicyEnvelope`、action/target/readable_roots/writable_roots/config_digest/authority_root。
- `SessionPolicyEnvelope::new`（同文件 `:197-265`）已经强制 Planning/Review 无 writable root、Coding 恰一个等于 canonical target worktree 的 writable root；该不变量保留。
- `src/product/logical_codebase/provider_capability_store.rs:31-73,231-245` 当前 capability 只有版本、dialect、snapshot、三档 provenance、一个 resume 状态和 action 列表，bootstrap 只写 Claude 全 action 与 Codex 空 action。它不能表达 action×resume 三态，需要版本化扩展。
- `src/cross_cutting/provider_capabilities.rs:9-65` 已有 `Confirmed / Denied(reason) / Unknown` 三态；`provider_gateway.rs:166-207` 已将其映射到 gateway resume 判定。新矩阵继续复用三态，Unknown 不得当作 allow。

### 2.3 Codex 当前危险模式由两处硬门保护

`src/cross_cutting/codex_provider/session.rs:54-70` 在无 tool policy 的 Coder/Executor 路径使用 `CODEX_DEFAULT_SANDBOX_MODE`；`src/cross_cutting/codex_provider/mod.rs:37` 当前该值为 `danger-full-access`。`src/product/logical_codebase/provider_gateway.rs:784-795,1105-1111` 和 `src/web/handlers/automation_gateway_preflight.rs:36-59` 已在 gateway/role-chain 层拒绝该配置。

本 change 不能因为要放行 Codex 就删掉硬门；正确做法是先把 Coding 投影收紧到可证明的受限模式，证据不足仍拒绝。

### 2.4 tool policy 和四家 adapter 事实

- `src/cross_cutting/streaming_provider/mod.rs:193-223`：`Orchestrator/WorkItemSplitter/Reviewer` 必须有 `DenyFileWriteBuiltins`；`Executor/Handoff` 必须无 policy；拒绝发生在 spawn 前。
- Claude：`src/cross_cutting/claude_code_provider/mod.rs:42-49,168-204` 将语义策略投影为 `--disallowedTools Edit,Write,NotebookEdit`。
- Pi：`src/cross_cutting/pi_provider/mod.rs:40-44,293-320` 将语义策略投影为 `--exclude-tools edit,write`。
- Codex：`src/cross_cutting/codex_provider/session.rs:54-70` 的策略会话已采用 read-only/on-request；Coder 仍使用 danger-full-access。
- Kimi：`src/cross_cutting/kimi_code_provider/client_services/policy.rs:40-75` 有独立 role policy，`src/cross_cutting/kimi_code_provider/mod.rs:204-220` 从 `input.working_dir` 启动 ACP；通用 `StreamingProviderInput.tool_policy` 不是 Kimi 的控制面。
- `src/cross_cutting/kimi_code_provider/client_services/terminal.rs:605-613` 当前 Unix `TerminalIsolation::Unavailable`，故不能声称 Kimi 已有 target-only OS 隔离。

### 2.5 预研的证明范围

预研报告 §1–§4 在非 Git、多成员 root fixture 中证明四家均能从根 `AGENTS.md` 引用 `.claude/rules` 并真实调用 MCP；Codex 需要 projects trust 与 `--skip-git-repo-check`，Kimi 需要 workspace trust。§6 明确未证明非 target 写边界、大 LC 成本和多版本兼容。该报告不能替代 write-boundary 或 resume evidence。

## 3. 总体架构

### 3.1 统一链路

`ProviderName → ProviderRefType + ProviderDialect → capability(action/resume) → LogicalCodebaseProviderAdmissionPreflight → LogicalCodebaseProviderGateway::validate → ValidatedSessionLaunchPolicy → revalidate_before_spawn → registry adapter → start/run`。

Gateway 同时覆盖：

- 流式栈 `StreamingProviderAdapter::start(StreamingProviderInput)`：Story、Design、Plan、Coding、Review、aggregate planning 的真实入口；
- 同步栈 `ProviderAdapter::run(AdapterInput)`：work-item split 等同步入口；
- LC 中不得使用裸 input、旧 `run_streaming` fallback 或 provider 之间的自动切换。

现有 `ProviderRegistry` 已登记 Claude/Codex/Pi/Kimi 名称，但 `LogicalCodebaseProviderGateway` 的真实 dialect 和 capability 仍只有两家；本 change 扩展 registry 的真实 adapter 装配，并让 task-run 的旧 Claude/Codex routing 不再成为 LC 的唯一入口。Fake 继续只在 test registry 中存在。

### 3.2 root-cwd / target / writable_roots

| 字段 | 来源 | 语义 | spawn 前复验 |
|---|---|---|---|
| `working_directory` | manifest `provider_context_root` canonical path | provider 进程 cwd、根规则/MCP/skills 发现根 | realpath、authority root、recipe receipt 一致 |
| `target` | member/checkout/worktree identity | 本次逻辑操作对象 | member、checkout、git-dir、worktree canonical identity |
| `readable_roots` | action policy | 逻辑可读声明 | canonical、authority 范围、无逃逸 |
| `writable_roots` | action policy | 只读为空；Coding 恰一个 target worktree | canonical target equality，不等于 OS 隔离 |
| `provider_projection` | gateway 按 dialect 生成 | sandbox/approval/tool/MCP/trust 的真实投影 | projection digest、版本、dialect、adapter 复验 |

LC 禁止用 `target.worktree` 替代 `working_directory`，也禁止把 aggregate root 自动加入 Coding writable roots；单仓将 cwd 与 target 映射到原目录，保持 direct 行为不变。

## 4. ProviderRefType、Dialect 与 capability

### 4.1 显式映射矩阵

| ProviderName | ProviderRefType | ProviderDialect | wire dialect | LC 结论 |
|---|---|---|---|---|
| ClaudeCode | `ClaudeCode` | `ClaudeCodeCliV1` | `claude-code-cli` | 迁移到统一 gateway/capability 链 |
| Codex | `Codex` | 保留 `CodexCliV1`（旧记录兼容） | `codex-app-server-rpc` | 只允许非 danger-full-access 的受限 evidence |
| Pi | 新增 `Pi` | 新增 `PiRpcV1` | `pi-rpc` | adapter、版本、tool policy、write boundary 均需证据 |
| KimiCode | 新增 `KimiCode` | 新增 `KimiAcpV1` | `kimi-acp` | adapter、trust、MCP、role policy、write boundary 均需证据 |
| Fake | 不进 gateway | 不适用 | 不适用 | 仅测试 registry，不能绕过真实门禁 |

`ProviderRef::from_provider_name` 改为四家显式 match。未来 provider、Fake、未知值均返回含 provider 名的 `PROVIDER_UNSUPPORTED_FOR_GATEWAY_LAUNCH`；禁止静默换成 Claude 或 direct fallback。

保留 `CodexCliV1` 是为了不悄悄改变已有 envelope/capability 记录的序列化语义；wire dialect 单独记录。若实施期证明旧枚举不能表达 app-server RPC，必须新增 variant 并把旧记录降为 Unknown，不得复解释旧数据为新能力。

### 4.2 action×resume 三态记录

建议把 `ProviderCapabilityRecord` 版本化为以下逻辑结构（字段名可在 proposal/spec 阶段定稿）：

```text
ProviderCapabilityRecord {
  provider_type, exact_version, gateway_dialect, adapter_wire_dialect,
  capability_snapshot_ref,
  provenance: Declared | FixtureVerified | ProductionVerified,
  action_matrix: [
    { action, launch, resume, write_boundary, projection_digest }
  ],
  trust: Confirmed | Denied(reason) | Unknown,
  probed_at, probe_artifact_ref
}
```

约束：

1. `launch`、`resume`、`write_boundary` 都是 `Confirmed/Denied(reason)/Unknown`；Unknown 不能先试启动。
2. Planning/Review 的 logical writable roots 必须为空；即使如此，OS/provider 写边界仍要有证据，不能仅由空列表推导物理只读。
3. Coding 的 `write_boundary=Confirmed` 必须来自真实 target-only probe：target 可写，root、其它成员、其它 worktree、`.git`/`.aria` 写尝试被拒绝；MCP 成功、git status 无变化、模型自述均不足。
4. `resume=Confirmed` 必须证明原生恢复、tool/sandbox/config 投影和 fingerprint 全部可重建；不能从 provider 全局布尔值推导。
5. provenance 只描述证据来源，不替代 action 逐格证据；`ProductionVerified + Unknown` 仍不放行。
6. 旧记录缺少矩阵时按 Unknown 读取；不能把旧 `supported_actions` 自动升级为 ProductionVerified。

Capability source 提供两个判定：`require_launch_supported(provider, action)` 和 `require_resume_supported(provider, action)`。fresh launch 不因 resume 未确认而阻断；明确 resume 请求必须要求对应 `resume=Confirmed`。

## 5. policy envelope 与四家权限映射

### 5.1 ProviderPolicyProjection

Gateway 内部生成不可由调用方伪造的 `ProviderPolicyProjection`，至少冻结：provider/dialect/version、OS cwd、协议 cwd（若不同）、action/role、permission/approval、semantic tool policy、target/write roots、sandbox/boundary、trust ref、config/MCP ref/digest、projection digest。adapter 只接受 gateway 绑定的 projection。

### 5.2 Claude Code

- Planning/Review：`DenyFileWriteBuiltins` → `--disallowedTools Edit,Write,NotebookEdit`；logical writable roots 为空。
- Coding：不带 deny token；logical writable root 仅 target。当前 Claude adapter 没有被源码证明的 writable-roots native 参数，故 target-only 需要 provider/OS boundary probe；不能用 root cwd 代替 target。
- cwd=root，消费根 `AGENTS.md/.claude/rules`；target 独立传递。
- resume 沿 `--resume` 和 native session id；policy、target、cwd、tool projection、config digest 任一漂移都拒绝。

只读 action 不能仅靠 `--disallowedTools` 提供物理只读：Claude 的 terminal、MCP 等可绕过内建写工具限制。LC Planning/Review 还需由 provider 原生只读边界或产品支持的 OS launcher 阻断根、成员和元数据写入；未验证该投影时 `write_boundary=Unknown`，不得因 fixture 无变更而放行。

### 5.3 Codex：从 danger-full-access 收紧

#### 5.3.1 结论

Codex Coding 首选 `workspace-write`，但 process cwd=root 时不能直接假设只写 target：workspace-write 通常把当前 workspace/cwd 纳入 writable set；只设置 `writable_roots=[target]` 可能仍使聚合根可写。因此 `writable_roots` 的逻辑声明不能单独构成安全保证。

采用两层门：

1. Planning/Review：`sandbox=read-only` + `approvalPolicy=on-request`，不授予 writable root。
2. Coding：只有在真实版本 probe 证明下列投影后放行：provider 进程 cwd 仍是 root；Codex thread protocol 的 `cwd` 明确指向 target；sandbox 是 `workspace-write`；实际 writable set 仅 target（加产品明确允许的临时目录）；root、其它成员、其它 worktree、`.git` 写尝试失败且 target 写成功。

若当前 Codex 版本不能将 protocol cwd 与 process cwd 分开，或 workspace-write 固定包含 root，则不能把 Coding 标记为 Confirmed。实施必须提供产品-owned sandbox launcher（例如基于受支持 OS sandbox 的 target allowlist）作为明确的 boundary 方案；native projection 和 launcher 均不可用时保持 capability denied/unknown，不回到 danger-full-access。

这里的 protocol cwd 是否保持 root discovery、是否影响 project config 与 AGENTS 祖先发现，必须由真实 app-server probe 验证；不能靠推测放行。

#### 5.3.2 映射表

| action | Codex projection | 放行条件 |
|---|---|---|
| PlanningReadOnly | `sandbox=read-only`、`approvalPolicy=on-request` | launch evidence + tool/approval wire evidence |
| ReviewReadOnly | 同上；fileChange/commandExecution 按策略拒绝 | launch evidence + reviewer evidence |
| CodingTargetWrite | `sandbox=workspace-write`；Auto→`never`、Supervised→`on-request`；protocol cwd=target | target-only probe、版本/dialect、D4 全部 Confirmed |

LC root 的 `--skip-git-repo-check` 和 projects trust 是初始化前置契约，不是 sandbox 替代；每次 spawn 仍复验 `[projects."<canonical_root>"] trust_level="trusted"`。Codex MCP 审批按既有 `session.rs` 分类：MCP 自发现按现有信任通道处理，策略 fileChange 拒绝，Coder 走既有 ApprovalBridge；未知形态不得静默不应答。

#### 5.3.3 REQ-ENV-05 修订

- `danger-full-access` 永久是 LC gateway 禁止模式，稳定错误仍为 `codex_danger_full_access_unsupported`，UI 选项、resume 和 permission mode 都不能覆盖。
- `read-only` 可用于 Planning/Review，但必须有版本、dialect、审批 wire、tool policy 证据。
- `workspace-write` 是 Coding 首选而非自动 allow；必须通过 target-only boundary probe、版本钉定和 D4。
- probe 未确认、参数未生效、协议 cwd≠target、trust 漂移或边界证据缺失时阻断；不得自动换 cwd、换 provider 或放宽 sandbox。

### 5.4 Pi

- Planning/Review：`DenyFileWriteBuiltins` → `--exclude-tools edit,write`；其余 terminal/extension/MCP 是显式信任逃逸面，不能伪称被该 token 禁止。还需独立验证覆盖这些通道的只读边界，否则 `write_boundary=Unknown`、阻断 LC 只读 action。
- Coding：不带 deny token，logical writable root 仅 target。当前 Pi adapter 没有已核实的 native writable-roots 参数，因此必须使用 provider/OS boundary launcher 并通过 target-only probe；无证据则 `write_boundary=Unknown`、路由阻断。
- cwd=root；Pi MCP 继续经已有 `pi-mcp-adapter`，不新增 prompt/env 注入。
- resume 使用 `--session-id`/Pi RPC session flow；只有版本、native session、tool projection、target/cwd boundary 全部可重建才 Confirmed。
- 单仓 direct Pi 不改。

### 5.5 Kimi Code

- Planning/Review：不读取通用 `ProviderToolPolicy`；沿 `ClientServicePolicy`：Reviewer 拒绝 Terminal/FsWrite，Orchestrator 拒绝 FsWrite，WorkItemSplitter/Handoff 全拒，读能力随 Auto/Supervised。对可写的其它 ACP 工具、MCP 和子进程仍需验证独立只读边界，否则 `write_boundary=Unknown`、阻断 LC 只读 action。
- Coding：Executor 沿既有 permission mode；logical writable root 仅 target，但当前 `TerminalIsolation::Unavailable` 不能证明 OS target-only。需要 provider/OS boundary launcher + target-only probe；无证据阻断。
- Aria-owned MCP bundle 继续遵守 REQ-ENV-08（allowlist、digest、脱敏、argv/审计、resume digest）；Kimi 原生项目 MCP 是 REQ-ENV-06 用户信任通道，两者不能混成一条来源。
- cwd=root；Kimi workspace-trust 必须已登记并每次复验。ACP `session/load` 只有 session id、bundle digest（若有）、version/dialect、policy/target/cwd fingerprint 全一致才允许。

### 5.6 writable_roots 的共同语义

审计必须拆分：

1. **逻辑授权**：envelope 是否只允许 target；
2. **真实投影**：adapter/launcher 是否把它变成 provider/OS 权限；
3. **实际证据**：target 可写、非 target 不可写、越界尝试和 pre/post snapshot。

缺少第 2 或第 3 层时只能记录 Unknown。D4 baseline 是检测/交付门，不是 OS 预防隔离；不能把 git status 单独写成“保证只写 target”。

## 6. resume 与 fail-closed 策略

### 6.1 当前/目标矩阵

| provider | 原生恢复形态 | LC 允许条件 |
|---|---|---|
| Claude Code | `--resume`，native session id | action resume Confirmed；policy/target/cwd/tool/config projection 全一致 |
| Codex | `thread/resume`，应答必须确认 thread id | 同上，且 sandbox/approval/protocol cwd 不漂移 |
| Pi | `--session-id` + RPC session flow | 版本、session、tool policy、target/cwd boundary probe Confirmed |
| Kimi | ACP `session/load` | trust、bundle（若注入）、policy/target/cwd、version/dialect 全一致 |

“现有命令带 resume 参数”只是实现线索，不是 LC resume 证据。各家实际支持状态由 action×resume probe 写入 capability store。

### 6.2 fingerprint 扩展

现有 `SessionResumeFingerprint::from_envelope`（`provider_gateway.rs:223-251`）覆盖 policy、action、target、provider version/dialect、capability snapshot；root-cwd 前置 change 已要求把 canonical `working_directory` 纳入 digest。本 change 再纳入或引用：semantic tool-policy digest、provider projection digest、Kimi Aria bundle digest（仅注入通道）、Codex/Kimi trust record digest、action-row evidence snapshot。

任一 policy revision/digest、authority root、cwd、target/git-dir/worktree、provider version/dialect、capability snapshot、tool projection、MCP bundle、trust 或 boundary 发生漂移，均不得续接旧 session。

### 6.3 失败语义

- 明确 resume 且 `resume != Confirmed`：返回 `provider_gateway_resume_not_supported` 或具体 capability reason，零 spawn；产品提供用户明确的“启动新会话”操作，不把 resume 静默变 fresh。
- fingerprint 不一致：沿现有 `resume_or_start` 给出 `StartNew` 决策并写入 superseded 审计；该决策本身不得创建新进程。只有用户明确请求新会话，且新会话重新完整 validate/revalidate 后才能 spawn。
- native handshake 无 session id、id 空/错、MCP digest 漂移、tool-policy audit 缺失：终止子进程，旧会话 superseded/failed，不冒充 provider 确认。
- validate→spawn 间版本、dialect、sandbox、trust、policy、target 或 availability 变化：`PolicyDrift`/`ProviderUnavailable`，零 spawn。

## 7. spawn 前复验与两个 adapter 栈

### 7.1 admission/preflight（不 spawn）

`LogicalCodebaseProviderAdmissionPreflight::check` 和 `automation_gateway_preflight` 共同检查：

1. LC manifest、authority root、root initialization readiness；
2. policy artifact/revision/digest、config reference；
3. ProviderRefType/Dialect 显式映射；
4. provider/action launch evidence，resume 请求另查 resume evidence；
5. Codex projects trust、Kimi workspace trust；
6. role→action 映射及 tool-policy 合法性；
7. logical writable roots 不变量；
8. provider projection、版本、adapter 存在；
9. availability；
10. Coding 的 write-boundary evidence 与 D4 baseline；只读 action 的原生或 OS 写边界 evidence。D4 在 admission 前采集，spawn 前复验其 freshness。

缺材料应落 durable waiting/preparation，不应先让 provider 失败后才告知用户。

### 7.2 gateway validate → revalidate → spawn

顺序固定为：load policy → resolve target → load capability → enforce Codex route policy → 构造 envelope → 生成 projection/fingerprint → Coding 采集 D4 → admission（检查该 baseline）→ 重新读取并逐维复验 policy/authority/cwd/target/capability/trust/tool/config/availability/D4 freshness → registry lookup → adapter guard → spawn → `provider_start` audit。

任何失败都发生在 `ProcessManager::spawn` 或真实 RPC/ACP child 启动前；adapter 不得自找备用 policy、备用 cwd 或备用 provider。

### 7.3 同步栈对齐

`LogicalCodebaseProviderGateway` 已持有 sync adapter 与 streaming registry。实施需要为 Pi/Kimi 注册同步 adapter，或实现一个只接受 validated projection 的同步 bridge；不允许继续使用无政策 `run_streaming` bridge。`AdapterInput.worktree_path` 在 LC 只表示 target，应增加/复用独立 `working_directory`；非 LC legacy input 映射旧目录。LC 固定 `allow_legacy_stream_fallback=false`。

## 8. tool-policy guard 对齐

| AdapterRole | Claude/Codex/Pi | Kimi | LC action |
|---|---|---|---|
| Orchestrator | 必须 `DenyFileWriteBuiltins` | client policy 拒绝 FsWrite | PlanningReadOnly |
| WorkItemSplitter | 必须 `DenyFileWriteBuiltins` | client services 全拒 | PlanningReadOnly |
| Reviewer | 必须 `DenyFileWriteBuiltins` | Terminal/FsWrite 拒绝 | ReviewReadOnly |
| Executor/Coder | 必须无通用 policy | 既有 Executor policy | CodingTargetWrite |
| Handoff | 必须无 policy | client services 全拒 | 仅既有调用，不新增写权 |

要求：

1. Claude/Codex/Pi 的 `start` 首行调用 `validate_tool_policy_for_role`，早于 process spawn、RPC handshake、Pi extension 启动。
2. Kimi 对非空通用 policy fail-closed，正常输入由 role/permission mode 派生 client policy；不得因“统一字段”绕过 Kimi 独立矩阵。
3. LC 不得借 legacy author/reviewer 例外绕过 gateway；legacy 例外仍必须有 engine builder、策略和 audit sink。
4. Coder/aggregate initialization 继续无通用 deny policy；本 change 不把 Coder 收紧成 Reviewer。
5. tool-policy digest、version、wire dialect 和 `provider_start` audit 在 resume 时精确比对；sink 缺失或写入失败沿既有 kill 链终止。

## 9. automation gateway preflight 同步放行规则

当前 `src/web/handlers/automation_gateway_preflight.rs:36-59` 的静态判定只覆盖两家旧矩阵。改为“静态映射 + durable capability + 当前 projection”同源判定：

- 每个 LC role 先经 `ProviderRef::from_provider_name`；
- role 派生 action；
- 查询 action launch/write-boundary/trust；resume/continuation 另查 resume；
- Codex 仅非 danger-full-access 且受限模式 probe confirmed 才放行；
- Pi/Kimi 仅 adapter/dialect/projection/evidence 全具备才放行；
- 不支持、未知、版本冲突、trust 缺失、write boundary unknown、tool policy 非法均列角色违规。

`AutomationRoleChainViolation` 至少携带 `role/provider/action/reason_code/capability_snapshot_ref/projection_ref`。稳定 reason code 建议包括：

- `provider_unsupported_for_gateway_launch`
- `provider_capability_action_unknown`
- `provider_resume_unsupported`
- `provider_trust_missing`
- `codex_danger_full_access_unsupported`
- `codex_target_boundary_unverified`
- `provider_write_boundary_unverified`
- `provider_adapter_dialect_mismatch`

一次 422 必须列出所有违规角色，不因首个错误短路。SingleRepository carrier 继续跳过 LC gateway predicates；Fake 仅 test-provider-enabled 时经 test registry 放行；`gateway_required=false` 不能成为 LC fallback 开关。

## 10. E2E 验收与写边界证据门

### 10.1 统一 fixture

使用非 Git canonical LC root，至少两个真实 Git member、一个 target worktree、一个非 target worktree；root recipe 已由 `lc-root-initialization` 完成 rules/policy/MCP/readiness。每家 provider 先跑 fresh，再按其 capability 跑 resume。

### 10.2 五类会话证据矩阵

| 阶段 | action | 必须证明 |
|---|---|---|
| Story | PlanningReadOnly | cwd=root；root rules；真实 provider 输出；只读边界阻断内建工具及 terminal/MCP 写；root/成员 snapshot 不变 |
| Design | PlanningReadOnly | 同上；MCP 结构化调用；目标成员只读；无 spawn 前置门绕过 |
| Plan | PlanningReadOnly | split/plan review 均经 gateway；target 与 cwd 分离；tool policy 正确 |
| Coding | CodingTargetWrite | target worktree 写成功；root/其它成员/其它 worktree 写失败；D4 baseline 无非目标变化 |
| Review | ReviewReadOnly | Review provider 经 gateway；只读边界阻断内建工具及 terminal/MCP 写；resume/fresh 语义正确 |

矩阵要逐家记录 Claude/Codex/Pi/Kimi 的：实际 argv/RPC/ACP、版本、dialect、provider session id、tool calls、approval events、policy/projection digest、trust key、pre/post snapshot、D4 baseline、错误/等待码。

### 10.3 写边界证据门

Coding 支持的硬门为：

1. **预防证据**：provider native sandbox 或 Aria-owned OS boundary launcher 明确列出 target writable root；无 boundary 不能以逻辑 writable root 代替。
2. **正向证据**：在 target 写入一个受控文件并观察真实文件变化；记录 provider 事件和 digest。
3. **负向证据**：尝试 root、非 target member、非 target worktree、`.git`、`.aria` 和越界 symlink 写入；全部被 provider/OS 拒绝或以可审计错误返回。
4. **D4 证据**：run 前后采集全部 member main checkout HEAD+porcelain；缺失、HEAD/status 非预期变化则阻断交付。
5. **复验证据**：validate→spawn 间重查 target/cwd/boundary；漂移零 spawn。
6. **清理证据**：测试 fixture 清除受控探针文件，不覆盖用户文件；未知变更 fail-closed。

只完成 pre/post snapshot 而无法证明预防边界时，action 只能是 Unknown，不能称“支持 coding”。

### 10.4 失败 E2E

必须验证：缺 root readiness、缺 trust、capability Unknown、Codex danger-full-access、Pi/Kimi boundary Unknown、错误 role policy、resume fingerprint 漂移、版本/dialect 漂移、target/git-dir 漂移、D4 非目标变化、adapter spawn 失败。每个 case 都要有“provider spawn 次数为零”或真实进程未创建证据。

## 11. 阶段切分与估时

总量按真实接入点重估为 **14–20 人日**；其中 6–12 人日只适用于复用已验证 sandbox、只改路由、不含四家写边界和完整 E2E，不能作为本 change 的完整估算。

| 阶段 | 工作包 | 主要文件/边界 | 估时 |
|---|---|---|---:|
| Phase 0 | 依赖门与能力契约冻结 | `lc-root-initialization` Phase2；root-cwd/target、trust、readiness、REQ-ENV-05/08/09 | 0.5–1 日 |
| Phase 1 | ProviderRef/Dialect、capability matrix、store migration | `provider_gateway.rs`、`policy.rs`、`provider_capability_store.rs`、`provider_capabilities.rs`、相关 resolver/tests | 2–3 日 |
| Phase 2 | 四家 projection 与 registry/同步流式接线 | `provider_registry.rs`、`provider_gateway.rs`、四家 adapter、`session_launch.rs`、sync routing | 3–4 日 |
| Phase 3 | Codex 受限 sandbox + Pi/Kimi/Claude write boundary | Codex session projection、provider launcher/boundary seam、provider-specific probes；若 native 不足则 OS launcher | 3–5 日 |
| Phase 4 | admission、automation role-chain、tool-policy/resume | `provider_admission_preflight.rs`、`automation_gateway_preflight.rs`、`streaming_provider/mod.rs`、audit | 2–3 日 |
| Phase 5 | 四家 × 五阶段 E2E、写边界、D4、版本矩阵 | `tests/it_web`、provider tests、真实 CLI report | 3–4 日 |
| **合计** | **可交付完整 change** | 依赖外部 CLI/OS 现场不计入开发日，但证据缺失必须阻断 | **14–20 日** |

每阶段内部按独占文件切片实施；Phase 3 的 boundary probe 未确认时不得把 Phase 5 的 provider action 标为支持。

### 11.1 推荐实施顺序

1. 先落地 `lc-root-initialization` Phase 2：canonical root、独立 cwd/target、trust/readiness、root recipe receipt。
2. 先扩展 capability/dialect 与 preflight，再扩 adapter；防止 adapter 已能启动但路由仍无可诊断能力记录。
3. 先完成 Planning/Review read-only 四家，再完成 Coding target write；Coding 的每家 provider 单独过 boundary evidence gate。
4. Codex 先将默认 danger-full-access 从 LC 路由永久拒绝，完成 workspace-write/protocol cwd 或 OS launcher 证据后再写入 action capability。
5. 最后跑完整五阶段 E2E 和单仓对照；单仓 command/cwd/direct/tool policy 必须零变化。

## 12. 非目标

- 不实现 LC 根初始化 recipe，不修改“recipe 只由 Claude Code 执行”的裁决。
- 不修改传统单仓初始化、direct provider topology、cwd、GitFinalize、单仓 capability 或既有 provider fallback 语义。
- 不新增多 target 自动化、跨 attempt 依赖编排、隐式 StartCoding。
- 不通过 prompt、环境变量、sidecar、复制规则或成员 thin pointer 伪造 provider 配置发现；保留 root-native discovery 与 REQ-ENV-08 Kimi bundle 的既有边界。
- 不把 logical writable roots、git status、MCP 调用成功或模型文字声明当成 OS 级隔离。
- 不支持 capability Unknown 时“先启动再审计”、不自动换 cwd、换 provider、降级到 danger-full-access 或裸 input。
- 不在本 change 解决 C6、旧 per-member digest 静默迁移、任意外部目录/嵌套 monorepo 自动发现、大 LC 成本预算制定。

## 13. 开放问题

这些问题必须在实施期以证据收口，不得默认为 allow：

1. **Codex protocol cwd 与 root discovery 的兼容性**：进程 cwd=root、thread cwd=target 时，AGENTS/rules、MCP、project config 的实际发现集合是否符合 root-cwd 合同。
2. **Codex writable_roots wire 形态**：当前 app-server 版本是否接受显式 writable roots；若不接受，是否能由 protocol cwd=target 得到 target-only effective roots；两者都不成立时采用哪一种受支持 OS launcher。
3. **四家 OS boundary 可用矩阵**：Linux Landlock/bwrap/namespace、macOS sandbox、Windows 等环境的受支持实现、失败码与部署前置；没有边界能力的主机必须呈现 waiting/unsupported，而不是放宽权限。
4. **Pi/Codex/Kimi 同步 adapter 的输出合同**：是否复用现有 `ProviderAdapter` 结构化输出，还是新增 validated bridge；不得恢复无政策 legacy bridge。
5. **多版本 CLI 兼容窗口**：版本、wire dialect、tool policy token、MCP/trust 记录和 resume 语义的最小支持矩阵。
6. **Kimi 原生 MCP 与 Aria bundle 的同时存在**：需要明确审计字段如何区分“用户信任自发现”和“Aria 注入”，以及 resume 时只对注入 digest 做漂移拒绝。
7. **大 LC 成本预算**：root-native discovery 的启动时延、上下文和 token 上限；超预算是否进入可操作等待面。
8. **读-only provider 的非 target 读取范围**：REQ-ENV-09 保护 built-in 写工具，不限制用户信任的 terminal/MCP 读取；产品是否需要额外的 read scope，需另立契约，不能在本 change 偷加。

## 14. OpenSpec proposal 接口与验收条款建议

后续 `openspec/changes/lc-gateway-multi-provider/` proposal/spec/tasks 应至少修改/引用：

- `session-policy-envelope`：REQ-ENV-01/02/03/04/05/06/08/09，新增四家 dialect、working_directory、projection digest、action×resume evidence、Codex workspace-write gate；保留 Kimi native discovery 例外与 tool-policy 矩阵。
- `logical-codebase-registration`：REQ-REG-12 的 capability/preflight 等待项扩展为四家，trust 事实来自前置 change，LC 缺 capability 零 spawn。
- `logical-codebase-aggregate-planning`：Story/Design/Plan/Review 的 root cwd、target 分离、gateway 入口一致性。
- `multi-target-group-coding`：只引用 target-attempt 的单目标授权、D4 baseline、REQ-MTG-03 无自动多 target 编排，不增加新的自动化。
- 可新增 `lc-gateway-multi-provider` capability，或明确作为上述 capability 的交叉修改；不能把设计文档中的“支持”直接写成已验证事实。

建议验收 MUST：

1. 四家 ProviderRef/Dialect 显式映射，未知 provider 和 Fake 不进入真实 LC gateway。
2. capability store 对每家 provider/action/resume/write boundary 保存三态证据；Unknown/Denied 一律在 spawn 前阻断。
3. Codex danger-full-access 在 gateway 和 automation preflight 均稳定拒绝；workspace-write 只有 target-only probe 通过才放行。
4. Pi/Codex/Claude 通用 tool guard 与 Kimi 独立 client policy 均在 spawn 前生效，策略/Executor 非法组合零 spawn。
5. Story/Design/Plan/Review 四家 read-only E2E、Coding 四家 target-write E2E 和每家 resume/fresh 证据齐全；写边界门包括正向 target 写、负向非 target 写、D4 baseline。
6. 单仓四家 direct 对照的 cwd、argv、tool policy、状态和输出契约无变化。

## 15. 自审记录

- **占位检查**：正文无 `TODO`、`TBD` 或空白方案；需要实施期证据的项目均列入开放问题、capability Unknown 语义和阶段验收门。
- **一致性检查**：Codex danger-full-access 仍在 route/preflight 双硬门；Codex workspace-write 不是自动放行；root-cwd 的 process cwd、独立 target、logical writable roots 三者未混用；trust 前置仍归 `lc-root-initialization`；REQ-MTG-03 未被扩展。
- **范围检查**：只设计 LC gateway 接入，不改代码、不改单仓行为、不实现 root recipe、不引入 provider fallback、不引入多 target 自动化。
- **证据边界检查**：将预研报告仅用于规则/MCP discovery 结论；明确写边界、大 LC 成本、多版本 CLI、Codex protocol cwd、Pi/Kimi OS isolation 均未提前宣称通过。
- **接地点检查**：已引用 `provider_gateway.rs`、`policy.rs`、`provider_capability_store.rs`、`provider_capabilities.rs`、`streaming_provider/mod.rs`、四家 adapter、`provider_admission_preflight.rs`、`automation_gateway_preflight.rs`、`multi-target-group-coding` 相关契约的现存符号与路径。
- **最终结论**：文档可作为后续 OpenSpec proposal 的接地输入；实施前必须先满足 `lc-root-initialization` Phase 2 依赖，且任何 provider/action 的“支持”都以真实 capability、write-boundary、resume 和 E2E 证据闭环为准。
