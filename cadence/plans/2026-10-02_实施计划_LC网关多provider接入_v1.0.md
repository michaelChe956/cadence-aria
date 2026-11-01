# LC 网关多 Provider 接入 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: 使用 `subagent-driven-development`（推荐）或 `executing-plans` 逐任务实施。步骤使用 checkbox（`- [ ]`）跟踪。本计划只展开已批准的 OpenSpec change；执行前由 Main 并行安排接地审查与一致性裁决。计划编写阶段不改代码、不运行产品检查、不 commit。

**Goal：** 为 LC 的 Story、Design、Plan/split、Coding、Review 交付 Claude Code、Codex、Pi、Kimi Code 四家真实 provider 的统一 gateway、逐 action 三态能力、权限投影、原生 resume 和全链验收证据；缺材料时在 provider 启动前失败关闭，单仓 direct 与 Claude-only 根 recipe 保持原契约。

**Architecture：** 保留 `LogicalCodebaseProviderGateway` 作为 LC 唯一 launch authority，在现有 opaque validated policy 上绑定 provider projection 和运行审计上下文；同步入口使用只接受 validated input 的 streaming-to-sync bridge，四家共用真实 adapter，不另写 Pi/Kimi 裸 CLI adapter。正常 LC 会话消费新 capability matrix；凭据约束的根 recipe 保持独立 evidence 判定。projection 与 native/产品拥有的写边界、trust、cwd/target、政策正文和 D4 在 validate→spawn 间复验，#8 的政策发布由并行 change 提供，本 change 只读消费。

**Tech Stack：** Rust 2024、Tokio、serde、sha2、现有 JSON/file-lock store、Axum、真实 Claude stream-json / Codex app-server JSON-RPC / Pi RPC / Kimi ACP；复用现有 bubblewrap 和 Kimi client-service 路径，无新增 Cargo 外部依赖。命令遵循 `cadence/project-rules/build-test-commands.md`。

**Spec：** `openspec/changes/lc-gateway-multi-provider/proposal.md`、`design.md`、`tasks.md`、`specs/lc-gateway-multi-provider/spec.md`；权威设计 `cadence/designs/2026-10-01_方案设计_LC网关多provider接入_v1.0.md`。#8 外部依赖为 `openspec/changes/aggregate-policy-root-publication/` 的 proposal 与两份 spec；该 change 并行推进，尚不能把其交付写成已完成。

## §0 依赖与共享文件权威表（唯一事实源）

> 本节是本计划唯一有效的依赖、共享文件 owner 与切片前置事实源。除本节外，E 表、Task Files、Interfaces、Step、修订对照表和自审文字不得重新列依赖边或推导新的 owner 顺序；需要说明前置关系时统一写“依赖与汇合见 §0”。历史 round1–3 对照表保留用于追溯，但其中的依赖描述全部作废，以本节为准。

### §0.1 依赖边清单

`D` 是批次符号，代表 Task 4/5/6a 的 provider projection、Codex projection 与 boundary launcher 输出，不是新增任务节点。以下是唯一任务依赖边清单；共享文件 owner 边另见 §0.3，不能与任务依赖混写。

| 编号 | 唯一依赖边 | 验算含义 |
|---|---|---|
| E1 | `1a → 2a` | 1a 映射、三态与纯 DTO 形状先于2a durable DTO/roundtrip。 |
| E2 | `2a → 2c` | 2c shape validator 消费2a store shape。 |
| E3 | `2c → 4` | Task4 provider projection 消费2c shape。 |
| E4 | `2c → 5` | Task5 Codex projection 消费2c shape。 |
| E5 | `2c → 6a` | Task6a boundary launcher 消费2c shape。 |
| E6 | `4 → 6c` | Task4 provider mapping 完成后，6c 执行对应真实 probe。 |
| E7 | `5 → 6c` | Task5 Codex mapping 完成后，6c 执行 Codex probe。 |
| E8 | `6a → 6c` | Task6a launcher/mount 保护完成后，6c 执行 boundary probe。 |
| E9 | `6c → 2d` | 真实 evidence 经校验后，2d 才写 durable Confirmed。 |
| E10 | `2d → 3` | durable action matrix 完成后，3 才提供 admission/revalidate。 |
| E11 | `3 → 7` | 3 完成后，7 才接入统一 tool-policy guard。 |
| E12 | `3 → 8` | 3 完成后，8 才聚合 role-chain 预检。 |
| E13 | `3 → 9` | 3 完成后，9 才实现 resume/native 决策。 |
| E14 | `7 → 10` | guard 完成后，Task10 才执行 provider matrix。 |
| E15 | `7 → 11` | guard 完成后，Task11 才执行 failure/boundary matrix。 |
| E16 | `7 → 12` | guard 完成后，Task12 才汇合最终依赖/direct 对照。 |
| E17 | `7 → 13` | guard 完成后，Task13 才执行兼容回归。 |
| E18 | `7 → 14` | guard 完成后，Task14 才记录迁移/运维状态。 |
| E19 | `8 → 10` | role-chain 完成后，Task10 才执行真实矩阵。 |
| E20 | `8 → 11` | role-chain 完成后，Task11 才验收聚合拒绝。 |
| E21 | `8 → 12` | role-chain 完成后，Task12 才汇合最终矩阵。 |
| E22 | `8 → 13` | role-chain 完成后，Task13 才收口兼容矩阵。 |
| E23 | `8 → 14` | role-chain 完成后，Task14 才记录等待原因。 |
| E24 | `9 → 10` | resume/native 合同完成后，Task10 才执行 fresh/resume 矩阵。 |
| E25 | `9 → 11` | resume/fingerprint 完成后，Task11 才验收零启动。 |
| E26 | `9 → 12` | resume 合同完成后，Task12 才汇合最终矩阵。 |
| E27 | `9 → 13` | resume 合同完成后，Task13 才收口兼容矩阵。 |
| E28 | `9 → 14` | resume 合同完成后，Task14 才记录支持状态。 |
| E29 | `D → 1b` | Task4/5/6a 行为完成后，1b 接入 projector/start_validated/boundary；1b 只消费1a DTO+2c shape。 |
| E30 | `1b → 1c` | 1b validated 消费面完成后，1c 做生产装配/sync bridge/credential phase/run identity。 |

因此任务拓扑唯一写作：`1a → 2a → 2c → {4/5/6a} → 6c → 2d → 3 → {7/8/9} → {10/11/12/13/14}`，并行入口为 `D → 1b → 1c`。1b 不消费2d/3；Task6 Step4 的后续现场证据是复用而非依赖。

### §0.2 Task 1 切片输入边界

| 切片 | 输入与禁止消费 |
|---|---|
| 1a | 不消费后续 Task；唯一负责映射、纯 projection/boundary DTO、validated 承载契约及基础 registry 接口。 |
| 1b | 消费 1a DTO 与 2c shape；在 E29 闸门后接入 D 的已完成行为；**不消费 2d 或 3**，不把 durable writer/admission 反向带入入口。 |
| 1c | 只消费 1b 已完成的入口/validated 消费面，并按本节 owner 门完成 registry、factory、sync bridge、credential phase 与 run identity 装配。 |

### §0.3 共享文件 owner 门（全路径）

| 文件 | 唯一 owner 顺序 | 约束 |
|---|---|---|
| `src/product/logical_codebase/mod.rs` | `1a → 2b → 2c → 6c → 2d` | 1a负责DTO/导出；2b负责source caller shape；2c负责validator接线；6c负责probe聚合；2d负责durable writer；1b不在此文件追加逻辑。 |
| `src/cross_cutting/mod.rs` | `1a → 6a → 1b` | 1a注册基础模块；6a接入boundary helper；1b接入gateway sync/validated入口。 |
| `src/web/gateway_factory.rs` | `1a → 1c → 2d → 3` | 1a冻结构造字段；1c负责registry/projector/gateway装配；2d注入durable writer；3接admission/readonly/material-prep。 |
| `src/product/logical_codebase/coordinator_provider_turn.inc.rs` | `1b → 1c` | 1b只改validated input/audit sink消费；1c只改credential phase准备与装配调用。 |
| `src/product/work_item_split_engine/tests/engine_gateway_guard.rs` | `1b → 9a → 13` | 依次完成split_sync入口、resume指纹迁移和direct兼容回归。 |
| `tests/it_web/provider_gateway_envelope.rs` | `1a → 7 → 9a → 13` | 依次冻结envelope、guard、resume audit和direct对照测试。 |
| `src/product/coding_workspace_engine/provider_stream/launch.rs`、`src/product/coding_workspace_engine/provider_stream.rs` | `7` 独占 | 仅Task7改validated/raw分流与audit sink；其它Task只读消费。 |
### §0.4 registry 边界

`src/web/state.rs:497-527` 的 `fake_mode_provider_registry` 是**合法的 non-LC legacy 运行时封装**，不属于正常 LC registry，不消费 gateway projection，也不构成 fallback；其中现有普通 `register` 五次调用原样保留。正常 LC 生产 registry 仅迁移 `state.rs::real_provider_registry:470` 到原子 `register_gated(name,adapter,projector,gate)`。LC 测试 fixture 使用 `register_test_pair`；因此 129 个既有 `register(name,adapter)` 调用的边界是“测试/非 LC legacy 保留”，不扩大生产迁移范围。

### §0.5 拓扑摘要

完整拓扑由 §0.1 的 E1–E30 组成；本节不再另列边。1b 的接口输入固定为 1a DTO + 2c shape，且不消费 2d/3；共享文件门的全路径以 §0.3 为准。任何其它位置出现的依赖或 owner 文字均只写“见 §0”。

## Global Constraints

 

- 四家显式映射 `ProviderRefType`、`ProviderDialect`、wire dialect、exact version 和真实 adapter；Fake、未知值、未来值不得回退 Claude、其它 provider 或 legacy direct。
- 正常 LC 的每个 action 行独立记录 `launch`、`resume`、`write_boundary`：`Confirmed`、`Denied { reason }`、`Unknown`。旧记录缺矩阵时为 `Unknown`，旧 `supported_actions` 或 provenance 不产生正常会话 Confirmed。
- fresh 必须通过 launch/write-boundary 及其它门；resume 为 Unknown 不阻止合法 fresh。明确 resume 追加对应 action 的 resume Confirmed，不得静默 fresh。
- 进程 cwd 必须是 manifest `provider_context_root` canonical path；target checkout/worktree 独立解析。Planning/Review logical writable roots 为空，Coding 恰一个 canonical target；logical roots 与 D4 不是 OS 预防隔离。
- projection 冻结 action/role、tool policy、permission/approval、sandbox、MCP/config、trust、cwd、target、roots、exact version/dialect 与 digest；调用者不能自造 projection。
- Codex `danger-full-access` 永久拒绝，稳定码 `codex_danger_full_access_unsupported`。Planning/Review 仅已验证 `read-only + on-request`；Coding 要求 `workspace-write + process cwd=root + protocol cwd=target + trust + D4 + target-only probe` 全部成立。
- Claude/Codex/Pi 的 `validate_tool_policy_for_role` 在 session child/RPC/extension 前执行；Orchestrator/WorkItemSplitter/Reviewer 必须 `DenyFileWriteBuiltins`，普通 Executor/Handoff 无通用策略。根 recipe 的完整 BootstrapExecutorMarker 例外保持原合同。Kimi 拒绝非空通用策略，继续既有 ClientServicePolicy。
- `automation_gateway_preflight` 与 gateway 同源判定，一次列全 role/provider/action/reason/capability/projection 引用；LC 的 `gateway_required=false` 无旁路效力；SingleRepository 保留原跳过语义。
- #8 发布最终政策正文与 digest 链，gateway 只消费其最终 `artifact.policy_text` 原始 UTF-8 字节（=canonical root下`policy_id`文件原字节），`policy_digest=artifact.digest=receipt.policy_digest`；`rule_digest`独立校验AGENTS原字节，不要求两摘要相等。producer形态仅以#8最新获批契约为准；gateway不拼接正文、不评论发布算法、不复制/物化/注入或回退内部JSON/成员仓，也不另造sidecar协议。
- 根 recipe 固定 Claude、五步四命令、trust 硬前置、receipt auditor、预算门和 readiness 维持现状。phase-aware capability 分流只隔离已交付 recipe evidence，不豁免 capability，也不把旧记录迁成正常矩阵 Confirmed。
- 不新增多 target 自动化、隐式 StartCoding、跨 attempt 编排、fallback、规则副本或 thin pointer；不修 #10 僵死 run / #13 Fake 默认策略等未授权台账。
- 所有支持格必须有真实 CLI/version/wire/argv/cwd/target/tool/approval/digest/写探针/快照证据；根发现旧报告不替代 gateway/resume/boundary E2E。缺证据为 Unknown/Denied，不以删格缩小验收范围。
- Rust快反馈：`cargo test --locked --lib <具体测试过滤名>`；集成：`cargo test --locked --test it_web <具体过滤名> -- --nocapture`；不得显式 `-j 1`。每切片先提交新增失败断言供 Main 跑红并回传，worker 据真实红点实现，合入后 Main 跑该片绿；检查与同批共享编辑互斥。全量 checks 只在所有切片落地后统一一轮，不由多个 worker 并跑。
- 每个执行切片独占精确文件、独立提交、单次派工不超过4h；表中为并行 worker 的单片上限估计，不是把所有切片机械相加的总工时。Round 2 将 2c 拆为 shape validator、2d 拆为 6c 后 durable writer；新增后置串行约2–3h，仍按 proposal 的可交付关键路径预算采用 **12–18 人日**（能力与存储2–3日、projection/registry/sync-stream3–4日、Codex/boundary3–4日、admission/tool-policy/role-chain/resume2–3日、四家 E2E/direct 对照2–4日）；design 的14–20日是含更宽现场风险的上界说明，不与本计划相加。外部 CLI/OS/home/trust 等待不计开发产能，四家共享 home/trust 的真实现场必须串行或加锁。报告证据留盘但不 `git add`；代码提交只暂存本切片明确文件。

## Review Focus

1. **CLI 升级/版本未知：** 同 provider 的 exact version、gateway/wire dialect 或 tool token 变化后，旧证据不继续 allow；正常矩阵转 Unknown/Denied，零 session spawn。Task 2/5/14 的版本漂移测试及真实 probe 钉住此行为；Pi 0.83.0、Kimi 0.34.0 现有最低门只作底线，不等于所有更高版本已验证。
2. **trust 与 capability 竞态：** validate 后任一 trust digest/action row/availability/D4/projection 改变，在最后 revalidate 时拒绝且进程启动计数为 0；已经构造的 envelope 不能绕过。Task 3/9 参数化逐维漂移测试。
3. **单仓误入 LC 门：** SingleRepository 即使配置 Pi/Kimi、`gateway_required=false` 或 legacy sync/stream，也保持原 cwd/argv/permission/output/GitFinalize；正常 LC 同标志仍被检查。Task 8/13 对照测试。
4. **projection digest 与真实写面：** digest 漂移、只拒内建工具而 terminal/MCP 可写、沙箱不存在或挂载失效时，不能凭空 writable roots / 无漂移快照升级。Task 4/5/6/11 覆盖。元数据负向探针保护 root/non-target 的 `.git`/`.aria` 及 target 的 `.git` 链接指针；target 内受控 git commit 是正向证据，git-dir 权限沿已交付 identity/授权链，不新增整个 `.git` 写授权（Main 已裁决）。
5. **存量 resume 指纹：** 老记录缺 projection/evidence/native session 确认，以及 policy/authority/cwd/target/git identity/trust/tool/MCP bundle 任一漂移，必须 supersede 并等待用户显式新会话；不得在 adapter 内清 resume id 后启动 fresh。Task 9/10/12 分别钉住决策、原生恢复和真实失败格；单仓既有漂移后 fresh 语义保留。

---

## 接地基线与已有草稿续用

编写基线为 worktree `feat-b-0808-add-monorepo` 的 HEAD `18e21a9bf75606d6b31a6bdbe9a023ff2f3c30bf`（2026-10-03 只读获取）。已有文件至 Task 6，复用其目标、四家映射、三态/投影/复验/TDD主题；纠正以下断点：`CapabilityEvidence` 实为 provenance 而非三态；并行表与 Files 有交叉；缺 sync bridge 与真实入口迁移；#8 消费接口不应自造 sidecar；任务估计超过单次 4h；缺后八包及自审。

| 已读接点 | 当前事实与计划落点 |
|---|---|
| `provider_gateway.rs::{ProviderRef::from_provider_name,ProviderRefType,validate_inner,start_streaming,run_sync,resume_or_start}` | 当前仅 Claude/Codex，validated input 在启动前复验；sync 使用两家旧 CliProviderAdapter；正常/recipe 共用 capability。Task 1/2/3/9 延伸，不重做 root-cwd。 |
| `policy.rs::{ProviderDialect,SessionPolicyEnvelope,AggregatePolicyArtifact}` | 三 action、空/单 writable roots 和 raw policy_text SHA-256 已有；`CodexCliV1` 序列化保留。#8 拥有最终政策发布；Task 1 只改 dialect，Task 3 只消费 locator。 |
| `provider_capability_store.rs::{CapabilityEvidence,ProviderCapabilityRecord,to_record,ensure_bootstrap}`、`production_policy_resolvers.rs::StoreBackedProviderCapabilitySource::require_supported` | provenance 现为 Declared/FixtureVerified/ProductionVerified，默认版本为 0.0.0-managed、action 列表全/空、resume 二态；Task 2 禁止把这些默认值视作正常真实证据。 |
| `session_launch.rs`、`streaming_provider/mod.rs::validate_tool_policy_for_role`、`ProviderStartAudit` | opaque policy 与 raw input 已绑定，三家 guard 已在首步，audit 仅有 tool/version/dialect 三元组；Task 1/7/9 扩绑定和消费，不重写 direct。 |
| 四家 `start` 与 Codex `codex_launch_params/codex_session_handshake` | Claude=stream-json；Codex=app-server；Pi=RPC+extension；Kimi=ACP。Codex当前策略read-only/on-request，Coder danger-full-access；三家当前 drift 清 id 后 fresh，Kimi start无通用策略拒绝。Task 4/5/7/9 只在 LC validated 路径修订。 |
| `process_manager.rs::{spawn,spawn_isolated}`、Kimi `client_services/{sandbox,terminal,terminal_handlers}.rs` | spawn_isolated 只清环境，不是文件沙箱；Kimi已有 bwrap terminal、Auto拒无bwrap、Supervised无隔离，不能外推到整provider/MCP。Task 6 补产品拥有的全写面边界，复用 git identity。 |
| `gateway_factory.rs::build_for_lc`、`provider_admission_check.inc.rs::check`、`cross_target_check.rs` | factory已有 scoped policy/capability和root assertion；admission仍遍历成员language.md；D4 capture/detect已交付。Task 3统一根正文消费并保留已有门；Task 6不重建D4。 |

证据底座全文已读：`cadence/reports/2026-10-01_预研报告_LC根初始化provider实测_v1.0.md`、`2026-10-01_验收报告_LC根初始化provider-CLI矩阵_v1.0.md`、`2026-10-01_验收报告_LC根初始化大LC与写边界_v1.0.md`、`2026-10-02_验收报告_LC根初始化E2E终局_v1.1.md`、`v1.2.md`。四家旧越界探针真实写入 root/非 target，证据级别仍 best_effort_configured；旧 E2E #8 手工物化不得复用为本 change 通过。先例计划 `cadence/plans/2026-10-01_实施计划_LC根初始化_v1.0.md` 只参考格式/实名回归/审查对照，不沿用旧依赖和缺失测试路径。

## 冻结接口与文件责任

下列标为“新增”的接口是本计划的实施产物，尚未存在；现状符号见上表。Task 间不得另造同义类型；不存在的类型有唯一产出 Task。

- **Task 1a（映射/承载）：** `ProviderRefType::{ClaudeCode,Codex,Pi,KimiCode}`；`ProviderDialect::{ClaudeCodeCliV1,CodexCliV1,PiRpcV1,KimiAcpV1}`；新增 `ProviderWireDialect::{ClaudeCodeStreamJson,CodexAppServerRpc,PiRpc,KimiAcp}`，序列化分别 `claude-stream-json`、`codex-app-server-rpc`、`pi-rpc`、`kimi-acp`。`ProviderRef::from_provider_name(&ProviderName, impl Into<String>) -> Result<ProviderRef,ProviderGatewayError>` 仍是唯一映射。
- **Capability矩阵 DTO/持久化：** 保留 `CapabilityEvidence::{Declared,FixtureVerified,ProductionVerified}` 名字，仅作 provenance；三态唯一复用 `ProviderCapabilityEvidence::{Confirmed,Denied { reason:String },Unknown}`。`ProviderActionCapability { action:SessionPolicyAction,launch:ProviderCapabilityEvidence,resume:ProviderCapabilityEvidence,write_boundary:ProviderCapabilityEvidence,projection_digest:String,evidence_ref:String }` 由矩阵合同 owner 定义并由 durable store 落盘。record 保留 `version/adapter_dialect/evidence`，新增 schema_version=2/wire_dialect/action_matrix/trust/probed_at/probe_artifact_ref/root_recipe_evidence；legacy字段仅DTO解码，normal不保留旧API。
- **Phase-aware capability接口：** `require_supported(&self,provider:&ProviderRef,action:SessionPolicyAction)->Result<ProviderCapability,ProviderGatewayError>`保留名，正常action row；新增同参返回型`require_resume_supported/require_write_boundary`。`require_root_recipe_supported(&self,provider:&ProviderRef,credential:&BootstrapPhaseCredential)->Result<ProviderCapability,ProviderGatewayError>`仅现有固定Claude recipe事实，不推normal Confirmed。新增gateway内部`validate_root_recipe_request(&self,request:SessionLaunchRequest,credential:&BootstrapPhaseCredential)->Result<ValidatedSessionLaunchPolicy,ProviderGatewayError>`；opaque launch私有冻结phase，普通调用不可构造。credential每次重验durable Running；root-recipe不误套normal target-only/read-only action门，仍逐项检policy/capability/root/target/trust/availability及原receipt auditor。
- **Projection 与纯 DTO：** 新增 `ProviderProjectionInput`，字段私有且只能由 gateway 构造；它携带 envelope、provider/action row、role、permission/tool/MCP/config/trust/boundary 输入。`ProviderPolicyProjector::project(&self,input:&ProviderProjectionInput)->Result<ProviderPolicyProjection,ProviderProjectionError>` 由 provider projector owner 实现。`ProviderPolicyProjection` 的固定只读字段为 `provider_type/provider_dialect/wire_dialect/exact_version/action/role/permission_mode/tool_policy/approval_policy/sandbox/working_directory/protocol_working_directory/target/readable_roots/writable_roots/trust_digest/config_digest/mcp_bundle_digest/boundary_evidence_ref/capability_projection_digest/projection_digest`；提供 `ProviderProjectionView` 或等价 `pub(crate)` getter，validated launch 也只提供 `pub(crate)` boundary/projection accessor，provider 不能自造或改写。digest 固定字段序+长度分隔+schema前缀，不能使用 Debug 文本或调用方摘要；只有 gateway 将候选 projection 与 capability evidence 校验后包装成 launch 权。纯 `ProviderBoundaryPlan`、`ProviderBoundaryEvidence`、`ProviderBoundaryError` DTO 归 `src/cross_cutting/provider_boundary.rs`，其 owner 门见 §0。
- **装配与生命周期：** registry 必须原子保存同一 provider key 的 adapter、projector 和 availability gate：新增 `ProviderRegistry::register_gated(name:ProviderName,adapter:Arc<dyn StreamingProviderAdapter>,projector:Arc<dyn ProviderPolicyProjector>,gate:Arc<ProviderAvailabilityGate>)->Result<(),ProviderRegistryError>` 与 `projector(&ProviderName)->Option<Arc<dyn ProviderPolicyProjector>>`；新增 test-only `register_test_pair(name,adapter,projector)->Result<(),ProviderRegistryError>`，普通 `register(name,adapter)` 不再生产可用且不得形成半注册。生产 caller 迁移、Fake/test fixture边界与原子 entry 规则见 §0.4。新增 `StreamingProviderAdapter::start_validated(ValidatedStreamingProviderInput,CancellationToken)->Result<ProviderSession,ProviderAdapterError>` 与 `ProviderAdapter::run_validated(ValidatedAdapterInput)->Result<AdapterOutput,ProviderAdapterError>`；默认返回 unsupported，不能回调裸 `start/run`。Gated 装饰器透传 projector 并在取用时复验同一 gate；raw `start/run` 仅保留单仓 direct。
- **准备与同步：** 新增 `ProviderLaunchAuditContext { workspace_session_id:String, role_run_seq:u64, audit_sink:Arc<dyn ToolPolicyAuditSink> }`。gateway 冻结 `prepare_streaming_launch(&self,input:StreamingProviderInput,request:SessionLaunchRequest,context:ProviderLaunchAuditContext)->Result<ValidatedStreamingProviderInput,ProviderGatewayError>` 与 `prepare_sync_launch(&self,input:AdapterInput,request:SessionLaunchRequest,context:ProviderLaunchAuditContext)->Result<ValidatedAdapterInput,ProviderGatewayError>`；所有 LC 角色先绑定 run-bound sink，Coder/Kimi 无通用 `tool_policy` 也必须写统一 launch audit。新增 `prepare_root_recipe_launch(&self,input:StreamingProviderInput,request:SessionLaunchRequest,credential:BootstrapPhaseCredential,context:ProviderLaunchAuditContext)->Result<ValidatedStreamingProviderInput,ProviderGatewayError>`，只由 durable recipe driver 调用，不能由普通 input 自报 phase。gateway `start_streaming/run_sync` 仍只接受 validated 类型；`provider_stream/launch.rs` 的 `launch_provider_session` 仅在 `(validated,gateway)` 成对时走 gateway，LC 缺任一项返回稳定错误，非 LC 无 gateway 才保留裸 `provider.start`。gateway 内部把 `start_streaming`/`run_sync` 改为 `start_validated`/`run_validated`，不回调裸 adapter start/run。
- **WS Plan/split run identity：** 新增 `WorkItemSplitProviderRunHandle { run_ref:String, workspace_session_id:String, role_run_seq:u64 }`；`LifecycleStore::begin_work_item_split_provider_run(project_id:&str,issue_id:&str,provider:&ProviderName,workspace_session_id:&str)->Result<WorkItemSplitProviderRunHandle,ProductStoreError>`、`complete_work_item_split_provider_run(&self,handle:&WorkItemSplitProviderRunHandle,prompt:&str,structured_output:&Value)->Result<(),ProductStoreError>`、`fail_work_item_split_provider_run(&self,handle:&WorkItemSplitProviderRunHandle,reason:&str)->Result<(),ProductStoreError>`。WS fresh/revision/followup先 begin、绑定 handle 的 sink、再调用 `start_work_item_plan_author`/streaming gateway，成功 complete、失败 fail；retry 每次新 handle。`parse.rs` 的 complete 函数消费已有 handle，不再重新 `save_work_item_split_provider_run`。
- **根recipe启动保持：** 正常 session 的 `ValidatedSessionLaunchPolicy` 必须有 projection、matrix 与 audit，真实 adapter 只走 `start_validated/run_validated`；唯一 private RootRecipe phase 仍经 gateway opaque 权、durable credential 复验后走原 Claude recipe 启动/marker/receipt 链，不能因普通角色矩阵要求空写根把 root recipe 改只读，也不能用 RootRecipe 事实给用户 Coding 授权。该路径是原 recipe 合同保留，不是 normal LC legacy fallback。
- **Codex projection：** `CodexSandboxMode::{ReadOnly,WorkspaceWrite}` 与 `CodexSandboxProjection { mode:CodexSandboxMode,approval_policy:String,process_cwd:PathBuf,protocol_cwd:PathBuf,target_root:PathBuf,boundary_evidence_ref:String }` 归映射合同 owner 定义，Codex provider 实现 mapping。固定 wire read-only/workspace-write，只读 approval=on-request、Coding Auto=never/Supervised=on-request；无 DangerFullAccess 枚举。`codex_launch_params` 保留 direct 分支，LC 只读 projection，不改全局 direct 默认。
- **Boundary行为：** 纯 boundary DTO owner 与共享文件门见 §0；`ProcessManager::spawn_with_boundary(command:&str,args:&[&str],working_dir:&Path,env_vars:&BTreeMap<String,String>,plan:&ProviderBoundaryPlan,cancel:CancellationToken)->Result<ManagedProcess,ProviderAdapterError>` 由 boundary 行为 owner 实现。validated launch 必须持有或可取得不可伪造的 plan；`start_validated`、ProcessManager/native/OS launcher 选择和 evidence_ref 必须来自同一 projection/plan。native 经真实 probe 可满足时使用 native；否则产品拥有的 Linux bwrap launcher 冻结 mount/metadata/provider runtime 目录，保持 cwd 及既有配置发现，覆盖 provider 后代/MCP/extension；Kimi 宿主 client services 另外消费同一 target plan。无支持 OS/namespace 或不可控写通道为 Unknown/Denied，不退到无隔离 spawn。
- **#8政策消费：** 复用 `AggregatePolicyArtifactStore::{for_lc,get}`、`AggregatePolicyArtifact::{policy_id,revision,digest,policy_text}`、`RootRecipeReceiptStore::get`、`RootRecipeReceipt::{policy_digest,rule_digest,canonical_root}`；新增只读 helper `verify_published_policy_body(authority_root:&Path,artifact:&AggregatePolicyArtifact,receipt:&RootRecipeReceipt)->Result<(),ProviderGatewayError>`。它验证 safe relative `policy_id`、canonical 无 symlink 逃逸、policy 文件原始 UTF-8 字节等于 `artifact.policy_text`、`artifact.digest == receipt.policy_digest == raw body SHA-256`、artifact/receipt canonical root 和 revision/policy_id 一致；`rule_digest` 独立读取 canonical root `AGENTS.md` 原字节。不得造发布 API、sidecar、成员仓回退或复制/物化政策。
- **Trust source：** 新增只读 `ProviderTrustSource` trait：`verify_trusted(&self,project_id:&str,lc_id:&str,provider:&ProviderName,canonical_root:&Path)->Result<ProviderTrustVerification,ProviderGatewayError>`；gateway持有 `trust:Arc<dyn ProviderTrustSource>`，factory装配。GET/early eligibility只调用`ProviderTrustHomeAdapter::{read_state,digest}`的read-only source，不能调用有 durable audit副作用的`ProviderTrustRegistry::verify_trusted`；spawn revalidate消费同一source，禁止偷偷 ensure/revoke。
- **证据摘要分层：** action row 的 `projection_digest` 是已实测 version/action 的完整权限 profile 摘要（该 action 允许的 role×permission 整张固定映射、tool/approval/sandbox/native 或 OS 方案/MCP 控制规范），不能随单次 role 变化。session projection 另冻结 `capability_projection_digest` 及包含当前 role/root/target/authority/trust/config 的全 `projection_digest`；probe evidence 另记当次真实 session digest。先计算 profile 并比对应 action evidence，再绑定 evidence_ref、计算 session 摘要；禁止 evidence_ref/digest 递归哈希自身；resume 同时比对 profile/evidence 与全 session 摘要。

**实现选项：** 推荐LC专用validated sync bridge，成本是一个事件消费器和专用runtime线程，收益是四家复用同一projection/guard/native协议且direct零改动；分别写四个sync CLI adapter会复制权限和恢复逻辑，并改既有task-run routing，因此不选。推荐优先复用native边界、Linux不足时扩展既有bwrap机制；仅依赖D4成本低但不能满足REQ-LCG-03/07，不作为可交付方案。

## 工作包映射与并行分组

| OpenSpec工作包 | Plan Task | REQ | 可独立派工切片（每片≤4h） |
|---|---|---|---|
| 1.1 | 1 | LCG-01/02 | 1a 映射/trait/opaque承载；1b validated双栈和全部LC入口；1c 生产装配与sync运行身份 |
| 1.2 | 2 | LCG-01 | 2a DTO/roundtrip/legacy；2b shape/source caller；2c shape validator；4/5/6a projection/boundary；6c真实 probe；2d durable Confirmed import |
| 1.3 | 3 | LCG-02/03 | 3a policy/canonical/target；3b trust/capability/D4/availability复验 |
| 2.1 | 4 | LCG-03/06 | 4a Claude；4b Pi；4c Kimi（互不共享文件） |
| 2.2 | 5 | LCG-04 | 5a Codex projection/wire；5b受限门+approval/boundary测试 |
| 2.3 | 6 | LCG-03/07 | 6a Linux launcher/后代；6b D4 freshness；6c四家真实probe/evidence（Kimi宿主target门归4c） |
| 3.1 | 7 | LCG-06 | 7a common guard与三家顺序回归；7b Kimi拒绝与recipe回归 |
| 3.2 | 8 | LCG-06 | 8a同源role-chain/DTO；8b GET/Enable/rebind接线 |
| 3.3 | 9 | LCG-05 | 9a fingerprint/explicit决策；9b Claude/Codex；9c Pi/Kimi原生确认 |
| 4.1 | 10 | LCG-07 | 10a真实harness；10b–10e四家各自五阶段fresh/resume现场 |
| 4.2 | 11 | LCG-07 | 11a失败零spawn harness；11b–11e各 provider 探针/快照现场 |
| 4.3 | 12 | LCG-02/07 | 12a #8交付门/recipe；12b四家 workspace streaming direct 对照与最终证据汇合（不宣称四家同步 direct） |
| 5.1 | 13 | LCG-02 | 13a单仓 Claude/Codex sync direct + Pi/Kimi reject；13b四家 workspace streaming legacy direct、GitFinalize/人工StartCoding红线 |
| 5.2 | 14 | LCG-01/03/04 | 14a migration/waiting测试；14b运维/最终矩阵对照 |

| 批次 | 可并行切片 | 独占源码边界（任务 Files 中的测试随同归属） | 真依赖与汇合 |
|---|---|---|---|
| A 合同先行 | 1a | gateway/policy/projection/session_launch/registry/trait/装饰器、capability provider 映射、planning dialect 映射与全部穷举 fixture；mod 接线同 worker | 见 §0 |
| B 存储与 launcher | 2a；6a | 2a capability_store/provider_capabilities；6a process_manager/provider_boundary，以及 Kimi client-services 的 helper 先行段 | 见 §0 |
| C capability source | 2b、2c、2d | 2b shape/source callers；2c shape validator；2d capability durable writer 与 gateway factory | 见 §0 |
| D 四家 projection | 4a、4b、4c、5a/5b；6b | 各自 provider 目录独占；6b 仅 cross_target_check.rs 与 D4 测试，provider_stream.rs 归 Task7 | 见 §0 |
| E 准入/双栈 | 1b、1c、6c、3 | gateway/admission/factory/streaming 与入口共享文件按 owner 门串行；6c 仅 probe 聚合模块+mod 接线；2d 仅 durable writer | 见 §0 |
| F role-chain/resume/兼容 | 8a、8b；9a、9b、9c；13 | 8 独占 automation；9 独占 gateway 恢复/audit/各 provider 恢复；13 独占 direct 测试/StartCoding 回归 | 见 §0 |
| G harness | 10a、11a、12a | 新 tests/it_web/web_lc_gateway_multi_provider/ 各分文件；module/it_web.rs 只由10a改 | 见 §0 |
| H 四家真实现场 | 10b–10e、11b–11e | 同 provider 同 fixture 内串行；不同 provider 独占各自 evidence 子目录 | 见 §0 |
| I 收口 | 12b；14a、14b | 12 写 policy_dependency/direct_comparison 模块与 E2E 汇总；14 独占 capability waiting/迁移测试与运维报告 | 见 §0 |

**全局共享文件门：** 所有共享文件 owner、全路径与切片职责见 §0；本处不再复述依赖边。`gateway_factory.rs`、`coordinator_provider_turn.inc.rs`、两个 `provider_stream` 文件、`engine_gateway_guard.rs` 与 `provider_gateway_envelope.rs` 均按 §0 owner 门执行。#8 发布/recipe 实现本 change 仅读；Main 与 #8 文件 owner 排开相同文件编辑，不改发布算法。


## Task 1：四家映射、validated双栈与真实入口（tasks.md 1.1；REQ-LCG-01/02）

**Files（按切片独占）：**

- **1a，3–4h：** Modify `src/product/logical_codebase/provider_gateway.rs`、`src/product/logical_codebase/policy.rs`（仅dialect）、`src/product/logical_codebase/mod.rs`、`src/product/logical_codebase/planning_context_resolver.rs`（dialect→ProviderType）、`src/product/logical_codebase/provider_capability_store.rs`（仅provider_type序列化四分支）、`src/cross_cutting/mod.rs`、`src/cross_cutting/session_launch.rs`、`src/cross_cutting/provider_registry.rs`、`src/cross_cutting/provider_adapter.rs`、`src/cross_cutting/provider_availability_gate.rs`、`src/cross_cutting/streaming_provider/mod.rs`；Create `src/product/logical_codebase/provider_projection.rs`、`src/cross_cutting/provider_boundary.rs`（纯 ProviderBoundaryPlan/ProviderBoundaryEvidence/ProviderBoundaryError DTO owner）；Test gateway与当前穷举match fixture：`src/product/logical_codebase/coordinator_tests.inc.rs`、`src/product/logical_codebase/planning_context_resolver_tests.inc.rs`、`src/product/logical_codebase/provider_admission_preflight_tests.inc.rs`、`src/product/coding_workspace_engine/tests/group_review_identity_snapshot.rs`、`src/product/coding_workspace_engine/tests/provider_gateway_validated_input.rs`、`src/product/work_item_split_engine/tests/engine_gateway_guard.rs`、`src/product/workspace_engine/tests/part_32.rs`、`tests/it_web/provider_gateway_envelope.rs`；只补显式枚举分支，2b后续迁移 matrix，拒绝默认实现不称支持。依赖与汇合见 §0。
- **1b，三个2–4h串行段：** Create `src/cross_cutting/gateway_sync_provider.rs`；Modify `src/cross_cutting/mod.rs`、`src/product/logical_codebase/provider_gateway.rs`、`src/product/work_item_split_engine/engine.rs`、`src/product/work_item_split_engine/parse.rs`、`src/product/workspace_engine/provider_drive.rs`、`src/product/workspace_engine/provider_drive/author_root_launch.inc.rs`、`src/product/workspace_engine/lifecycle/routing_reference.inc.rs`、`src/product/workspace_engine/review/drive.rs`、`src/web/workspace_ws_handler/run/provider_run.rs`、`src/web/workspace_ws_handler/run/provider_run/work_item_plan_legacy_author.inc.rs`、`src/web/workspace_ws_handler/run/followups.rs`、`src/web/workspace_ws_handler/run/single_candidate.rs`、`src/web/workspace_ws_handler/run.rs`、`src/web/workspace_ws_handler/run/gateway_start.rs`；coding 段 Modify `src/product/coding_workspace_engine/lifecycle.rs`、`src/product/coding_workspace_engine/provider_retry.rs`、`src/product/coding_workspace_engine/provider_retry_parts/coder_root_launch.inc.rs`、`src/product/coding_workspace_engine/group_review_orchestrator.rs`、`src/product/coding_workspace_engine/internal_pr_review.rs`、`src/product/logical_codebase/coordinator_provider_turn.inc.rs`；`provider_stream/launch.rs` 只读消费，接线归 Task7。顺序为 bridge+split→workspace→coding。Test `src/product/logical_codebase/provider_gateway_tests/audit.inc.rs`、`src/product/work_item_split_engine/tests/engine_gateway_guard.rs`、`src/web/workspace_ws_handler/tests/gateway_start.rs`、`src/product/coding_workspace_engine/tests/provider_gateway_validated_input.rs`；新bridge内测current-thread调度。
- **1c，3–4h，拆为两个不共享文件的微切片：** `1c-factory` Modify `src/web/state.rs`（default_provider_registry 原子装配四家 adapter/projector/gate）与 `src/web/gateway_factory.rs`（依赖注入与 LC sync bridge，不扩大 normal admission 写权限）；`1c-coordinator` Modify `src/product/logical_codebase/coordinator_provider_turn.inc.rs`（credential phase 准备）、`src/product/lifecycle_store/plan.rs`（split 运行身份）、`src/product/lifecycle_store/tool_policy_run_audit.rs`（复用 run-bound sink），并按 §0 的 owner 门接续。Test上述文件内测试。Read-only `src/task_run/provider_factory.rs`、`src/cross_cutting/cli_adapter.rs`，单仓现有 Claude/Codex 两槽 routing 不迁移；Pi/Kimi 不列入本 change 的同步 direct 交付。

**Interfaces：** 1a 只产出本计划冻结的映射、纯 projection/boundary DTO、validated 承载与 registry 基础接口；1b 消费 1a DTO、2c shape，并在 §0 E29 闸门满足后接入 D 的 projector/start_validated/boundary 行为；1c 只消费 1b 已完成的 validated 入口消费面。不得在本处写反向依赖或要求1b/1c反向提供给2/3/4/5/6。新增 `WorkItemSplitProviderRunHandle { run_ref:String, workspace_session_id:String, role_run_seq:u64 }`；其 lifecycle 与 caller 迁移按 §0 owner 门执行。

 - [ ] **Step 1：写失败测试。** 在1a测试写 `lcg_t01_provider_ref_maps_four_real_providers` / `lcg_t01_fake_and_unknown_never_fallback`、`lcg_t01_registry_rejects_half_registered_adapter_without_projector`、`lcg_t01_registry_keeps_adapter_projector_and_gate_same_key`、`lcg_t01_registry_projector_lifetime_matches_adapter`；1b写 `lcg_t01_sync_bridge_uses_validated_start_and_preserves_output`、`lcg_t01_bridge_does_not_block_tokio_current_thread`、`lcg_t01_all_lc_entrypoints_require_projection`、`lcg_t01_validated_call_never_falls_back_to_raw_start_or_run`、`lcg_t01_ws_plan_split_run_handle_closes_on_success_and_failure`、`lcg_t01_parse_consumes_existing_split_run_handle`；1c写 `lcg_t01_split_start_audit_precedes_completed_record`。registry 测试必须覆盖旧 `register_gated(name,provider,gate)` 迁移后 adapter/projector/gate 同 key 原子注册、availability recheck 使用同一 gate、半注册无可见 entry；validated 隔离测试断言 gateway start/run 计数与裸 `provider.start/run` 计数分开且 LC validated 调用不会落到裸路径。关键断言：
```rust
assert_eq!(mapped_ref.provider_type, expected_provider);
assert_eq!(direct_calls, 0);
assert_eq!(legacy_bridge_calls, 0);
assert_eq!(observed_cwd, canonical_root);
assert_eq!(observed_target, member_worktree);
assert_eq!(output.structured_output, Some(expected_payload));
assert_eq!(output.timeout_status, TimeoutStatus::NotTimedOut);
assert!(timer_completed_before_provider_finished);
assert!(provider_start_seq < split_completed_seq);
assert!(failed_run.status == "failed");
assert_eq!(parsed.provider_run_ref, handle.run_ref);
```
 - [ ] **Step 2：Main运行红测试。** 每个提交段先执行 `cargo test --locked --lib lcg_t01`，预期新 enum/validated 分发、WS run handle 成功/失败收口或 raw bridge/输出/调度断言失败；必须列出实际命中的新测试数，0 tests 不是红证据。既有 direct tests 不能删除或改成 allow。
 - [ ] **Step 3：实现对应签名。** 1a冻结映射、projection opaque 承载及 validated trait（默认 unsupported 只供未接入 adapter 拒绝）。1b 新增 `prepare_streaming_launch(&self,input,request,context)` 与 `prepare_sync_launch(&self,input,request,context)`，所有 LC 角色在 prepare 前绑定 audit sink；真实 gateway 仅调用 validated trait。同步 bridge 的 `run_validated(&self,launch:ValidatedAdapterInput)->Result<AdapterOutput,ProviderAdapterError>` 使用专用 OS 线程/Tokio runtime 与现有 completion/sentinel parser，不嵌套 current-thread `block_on`；超时、取消、Failed、PermissionTimeout、malformed output 沿现有错误，不用空 JSON/exit0 兜底。WS streaming Plan/split 的每个真实 caller 依次 begin handle、bind sink、start、parse、complete/fail；sync split 走同一 gateway bridge 但不借 streaming fresh 推导 resume。1c 注册四家 adapter/projector/gate 原子装配，Factory 注入 gateway sync bridge，recipe…
 - [ ] **Step 4：Main运行绿测试。** `cargo test --locked --lib lcg_t01`、`cargo test --locked --lib logical_repository_generate_fails_closed_with_gateway_required`、`cargo test --locked --lib logical_repository_generate_revision_fails_closed_with_gateway_required`、`cargo test --locked --lib legacy_repository_generate_still_invokes_adapter_directly`、`cargo test --locked --lib split_sync_gateway_launch_rebinds_cwd_to_canonical_root`、`cargo test --locked --lib logical_provider_entrypoints_use_gateway_for_sync_and_streaming_stacks`；全部 LC 入口均有 opaque projection/真实 audit，且 streaming Plan/split 与 split_sync 的 run_ref/证据不混用。Coder/Kimi 即使无通用 tool policy 也分配 run-bound sink；非法外来 `Some(policy)` 拒绝，不能在 adapter 偷清。direct 旧 builder 字节不改；去掉 LC `Validated*::new` 的 production-facing public 构造入口，迁移所有生产 caller。
 - [ ] **Step 5：逐段提交。** 仅 `git add` 各切片 Files；1a提交四家 LC validated 合同；1b分别提交 sync bridge、workspace streaming Plan/split、Coding/Review caller 迁移；1c提交真实 projector/gate 装配与 LC 运行审计。`src/product/coding_workspace_engine/provider_stream/launch.rs` 的接线由 Task7 提交。每次提交只暂存本段精确 Files，不提交证据报告。


## Task 2：Capability三态存储迁移与phase隔离（tasks.md 1.2；REQ-LCG-01）

**Files（按切片独占）：**

- **2a，3–4h：** Modify `src/product/logical_codebase/provider_capability_store.rs`（DTO/get/upsert/bootstrap及内嵌测试）、`src/cross_cutting/provider_capabilities.rs`（复用三态与四家dialect常量）；Test同文件。
- **2b，两个≤4h串行原子段：** shape/source签名与全部受影响fixture同一commit，Modify `src/product/logical_codebase/production_policy_resolvers.rs`、`src/product/logical_codebase/provider_gateway.rs`、`src/product/logical_codebase/mod.rs` 及既有 caller/tests：`src/product/logical_codebase/coordinator_tests.inc.rs`、`src/product/logical_codebase/planning_context_resolver_tests.inc.rs`、`src/product/logical_codebase/provider_admission_preflight_tests.inc.rs`、`src/product/logical_codebase/provider_gateway_tests.rs`、`src/product/coding_workspace_engine/tests/group_review_identity_snapshot.rs`、`src/product/coding_workspace_engine/tests/provider_gateway_validated_input.rs`、`src/product/work_item_split_engine/tests/engine_gateway_guard.rs`、`src/product/workspace_engine/tests/part_32.rs`、`src/web/workspace_ws_handler/tests/single_candidate_lc_admission.rs`、`tests/it_web/provider_gateway_envelope.rs`。第二段只改同三实现文件/相关 tests 的 phase 判定与错误，不先提交半编译类型让下片兜编译。
- **2c，3–4h：** Create `src/product/logical_codebase/provider_capability_probe.rs`（real probe/evidence shape validator）；Modify `src/product/logical_codebase/mod.rs`（shape/source 接线）。`src/web/gateway_factory.rs` 只由 2d 负责 durable writer 注入，2c 不与 4/5/6a 形成循环。Test 新模块内测试。不能只改 bootstrap 写四条 allow 记录。
- **2d，2–3h，6c 后置：** Modify `src/product/logical_codebase/provider_capability_probe.rs`、`src/product/logical_codebase/provider_capability_store.rs`、`src/product/logical_codebase/mod.rs`、`src/web/gateway_factory.rs`；Test 新模块及 `src/product/logical_codebase/provider_gateway_tests/audit.inc.rs`。只把已通过三方一致性校验的 evidence 导入 durable action row；不重新 probe、不接受 record 自报。

**Interfaces：** 具体输入输出、切片前置与汇合仅以 §0 为准。Task2 提供 matrix/source 三方法；gateway `ProviderCapability` 用 `provider_type/version/adapter_dialect/wire_dialect/capability_snapshot_ref/action_capability/trust`。2c shape validator 冻结 `ProviderCapabilityProbeService::validate_probe_shape(record:&ProviderCapabilityRecord,evidence:&ProviderBoundaryEvidence,projection:&ProviderPolicyProjection)->Result<(),ProviderCapabilityProbeError>`，只验证字段可比对、digest 形状、action/profile 类型与 evidence 引用，不写 durable。2d 冻结 `record_verified_probe(project_id:&str,record:&ProviderCapabilityRecord,evidence:&ProviderBoundaryEvidence,projection:&ProviderPolicyProjection)->Result<(),ProductStoreError>`：必须逐字段比对 evidence、projection、record 后才导入 durable Confirmed。

- [ ] **Step 1：写失败测试。** `lcg_t02_matrix_round_trips_three_states`、`lcg_t02_legacy_actions_read_unknown`、`lcg_t02_fresh_and_resume_use_separate_cells`、`lcg_t02_cli_version_drift_invalidates_row`、`lcg_t02_probe_evidence_projection_record_imports_confirmed_atomically`、`lcg_t02_probe_evidence_projection_record_mismatch_stays_unknown`、`lcg_t02_unknown_normal_matrix_keeps_existing_root_recipe_contract`。`lcg_t02_probe_evidence_projection_record_imports_confirmed_atomically` 构造真实三元组并断言：

```rust
assert_eq!(evidence.exact_version, projection.exact_version);
assert_eq!(evidence.projection_digest, projection.projection_digest);
assert_eq!(record.version, evidence.exact_version);
assert_eq!(record.action_matrix[&action].projection_digest, projection.projection_digest);
assert_eq!(record.action_matrix[&action].launch, ProviderCapabilityEvidence::Confirmed);
assert_eq!(record.action_matrix[&action].write_boundary, ProviderCapabilityEvidence::Confirmed);
assert_eq!(record.probe_artifact_ref.as_deref(), Some(evidence.artifact_ref.as_str()));
```
`mismatch` case 断言 durable row 字节保持旧值/Unknown，返回错误且 provider spawn count=0。另用包含三种状态且 Denied reason=`boundary probe denied` 的 DTO，断言 legacy/recipe 语义不变。

```rust
assert_eq!(loaded.action_matrix, written.action_matrix);
assert_eq!(row.resume, ProviderCapabilityEvidence::Denied { reason: "boundary probe denied".into() });
assert_eq!(legacy_row.launch, ProviderCapabilityEvidence::Unknown);
assert!(fresh_with_launch_and_boundary_confirmed.is_ok());
assert!(explicit_resume_with_unknown_resume.is_err());
assert_eq!(new_cli_row.launch, ProviderCapabilityEvidence::Unknown);
assert_eq!(normal_spawn_count, 0);
assert!(valid_credential_existing_recipe_evidence.is_ok());
assert!(ordinary_request_using_recipe_evidence.is_err());
```
- [ ] **Step 2：Main运行红测试。** `cargo test --locked --lib lcg_t02`；预期无 matrix 字段/三态 Denied 丢 reason、legacy 自动 allow、或 evidence/projection/record 不一致仍写 Confirmed 而 FAIL。记录每个新测试真实红点，不把旧绿单测冒充新 TDD。
- [ ] **Step 3：实现对应签名。** 2a 读取缺 schema/matrix 为 v1，旧字段仅 decode，三 action Unknown 且 Denied reason 完整，未知 schema/状态拒绝。2b 只完成类型形状、所有 fakes 与 phase caller 的原子迁移；2c 只做 shape validator；4/5/6a 提供 projection/boundary；6c 产真实 evidence；2d 按三方逐字段一致才写 durable Confirmed，不自行把 CLI 启动/MCP/provenance 作 Confirmed；旧版本 evidence 保留、当前 normal 行失效等待真实 probe。root-recipe evidence 独立隔离，不升级 normal。
- [ ] **Step 4：Main运行绿测试。** `cargo test --locked --lib lcg_t02`、`cargo test --locked --lib provider_capability_store`、`cargo test --locked --lib store_backed_capability`；新增三方 Confirmed import 与 mismatch 保留旧字节均 PASS，Denied 理由经 JSON/API 往返不丢失。
- [ ] **Step 5：提交。** 2a/2b/2c 各按精确 Files 提交；2d 后置于6c，提交“以 evidence/projection/record 三方一致性导入 durable capability”；永久行为/等待说明随 Task14 运维报告，不另建 changelog。


## Task 3：Canonical政策、trust与spawn前复验（tasks.md 1.3；REQ-LCG-02/03）

**Files：** 3a/3b 各2–4h，串行共享：Modify `src/product/logical_codebase/provider_gateway.rs`（validate/revalidate/统一无副作用 verdict）、`src/product/logical_codebase/provider_admission_check.inc.rs`、`src/product/logical_codebase/provider_admission_preflight.rs`、`src/product/logical_codebase/production_policy_resolvers.rs`、`src/web/gateway_factory.rs`；迁移真实 factory caller/读写语义并列为本 Task owner：`src/web/coding_ws_handler/runner/task.rs`、`src/web/workspace_session/manager/mod.rs`、`src/web/handlers/aggregate_initialization/production_dependencies.inc.rs`、`src/web/workspace_ws_handler/tests/gateway_start.rs`、`tests/it_web/provider_gateway_envelope.rs`。Test `src/product/logical_codebase/provider_admission_preflight_tests.inc.rs`、`src/product/logical_codebase/provider_gateway_tests/task28_root_authority.inc.rs`、`src/product/logical_codebase/provider_gateway_tests/audit.inc.rs`。Read-only #8 policy producer 区及其精确 receipt/trust owner 文件：`src/product/logical_codebase/root_recipe_receipt.rs`、`root_recipe_receipt_types.inc.rs`、`root_recipe_receipt_store.inc.rs`、`root_recipe_receipt_auditor.inc.rs`、`root_recipe_receipt_tests.inc.rs`、`provider_trust.rs`、`provider_trust_adapters.rs`、`provider_trust_store.rs`、`bootstrap_projector.inc.rs`；本 Task 只消费/接线，不改发布、receipt 登记、trust ensure/revoke 算法。`build_readonly_for_lc` caller 只能做 early/GET/action eligibility 读取；`build_for_lc_material_prep` caller 只能是 #8/recipe material prep，禁止 normal admission 借用。

**Interfaces：** 具体输入输出、切片前置与汇合仅以 §0 为准。只读 trust source 冻结为 `ProviderTrustSource::verify_trusted(&self,project_id:&str,lc_id:&str,provider:&ProviderName,canonical_root:&Path)->Result<ProviderTrustVerification,ProviderGatewayError>`；其 home 读取复用 `ProviderTrustHomeAdapter::{read_state,digest}`，`ProviderTrustRegistry::verify_trusted` 因 durable audit 副作用不得用于 GET/early。新增只读 `verify_published_policy_body(authority_root:&Path,artifact:&AggregatePolicyArtifact,receipt:&RootRecipeReceipt)->Result<(),ProviderGatewayError>`，验证 safe relative policy_id、canonical 无 symlink 逃逸、policy 原始 UTF-8 字节、`artifact.digest == receipt.policy_digest == raw body SHA-256`、artifact/receipt canonical root/revision/policy_id；`rule_digest` 独立读取 canonical root `AGENTS.md` 原字节。gateway持有 `trust:Arc<dyn ProviderTrustSource>`，factory装配，同一source供spawn revalidate消费。

新增 `admission_verdict(&self,request:&SessionLaunchRequest,projection:&ProviderPolicyProjection,is_resume:bool)->Result<(),ProviderGatewayError>` 完整复验；early `action_admission_verdict(&self,provider:&ProviderRef,action:SessionPolicyAction,role:&AdapterRole,permission_mode:ProviderPermissionMode)->Result<ProviderActionAdmission,ProviderAdmissionError>` 返回 `ProviderActionAdmission { capability_snapshot_ref:String, projection_ref:String }`，不携未来 target/worktree 或 D4。`LogicalCodebaseGatewayFactory::build_readonly_for_lc(project_id:&str,lc_id:Option<&str>)->Result<LogicalCodebaseProviderGateway,ProviderGatewayError>` 只能读取，不调用 `ensure_bootstrap`、不写 policy/capability/trust/audit；新增 `build_for_lc_material_prep(project_id:&str,lc_id:Option<&str>)->Result<LogicalCodebaseProviderGateway,ProviderGatewayError>` 仅供 #8/recipe material prep 的显式调用，写入边界只允许其 recipe material，normal admission不能借用。

admission 成员规则拆为 `missing_root_rules` 与 `missing_member_rules`：仅 AggregateBootstrap phase 可豁免前者；后者和 policy/capability/authority/target/trust/availability/projection/D4 任何 phase 均阻断。#8 policy body 消费不生成/替代成员 `language.md`，本 change 不复制成员规则。

- [ ] **Step 1：写失败测试。** `lcg_t03_root_policy_body_missing_waits_without_materialization`、`lcg_t03_policy_locator_rejects_symlink_and_digest_drift`、`lcg_t03_validate_then_each_frozen_dimension_drifts_zero_spawn`、`lcg_t03_root_cwd_and_target_are_independent`、`lcg_t03_early_eligibility_needs_no_future_d4_and_writes_nothing`、`lcg_t03_member_language_rule_missing_is_not_root_phase_exempt`；逐维 policy/authority/cwd/target/git/trust/action row/projection/config/MCP/availability/D4 漂移；另断 early 合法证据在 attempt 未创建时可判资格且 policy/capability/trust audit 前后字节不变，实际 spawn 缺 D4 拒绝；AggregateBootstrap 只可豁免 root 规则缺失，成员规则缺失必须 waiting+zero spawn。

```rust
assert!(matches!(&waiting, ProviderAdmissionError::Waiting { reason_code, .. } if reason_code == "provider_policy_artifact_missing"));
assert_eq!(root_files_before, root_files_after);
assert!(matches!(error, ProviderGatewayError::PolicyDrift { .. } | ProviderGatewayError::ProviderUnavailable(_)));
assert_eq!(adapter_start_count, 0);
assert_eq!(adapter_run_count, 0);
assert_eq!(envelope.working_directory, canonical_root);
assert_eq!(envelope.writable_roots, vec![canonical_target]);
assert_eq!(member_rule_missing.status, "waiting");
assert_eq!(member_rule_missing.provider_spawn_count, 0);
```

- [ ] **Step 2：Main运行红测试。** `cargo test --locked --lib lcg_t03`，预期当前只读内部 artifact/成员 language 规则及有限复验不能覆盖新增 root locator/trust/boundary 漂移；红点必须来自新增行为，已有 cwd 分离绿测试保留。
 - [ ] **Step 3：实现精确 helper/verdict。** `verify_published_policy_body(authority_root,artifact,receipt)` 只读 canonical root 安全 locator，核对 raw UTF-8、artifact/receipt digest、root/revision/policy_id 与独立 AGENTS raw-byte `rule_digest`。阶段顺序为 identity/manifest、policy body/artifact/receipt、provider mapping/version/action capability、trust source、role/tool、logical roots、projection/adapter、availability、boundary/D4。prepare 冻结合法 projection，spawn 前重新读取以上事实；cwd canonical 必须等于 manifest root，不能仅 prefix 允许子目录；target git identity 独立复验，resume 追加 action resume。`missing_root_rules` 仅 AggregateBootstrap phase 可豁免；`missing_member_rules` 和其它门任何 phase 均阻断。trust 只读，不能在 spawn 偷偷 ensure/revoke；root recipe 凭据只沿 #8 临时政策合同与自身 evidence，不豁免 policy/capability/gateway/cwd/target/trust。
- [ ] **Step 4：Main运行绿测试。** `cargo test --locked --lib lcg_t03`、`cargo test --locked --lib logical_root_or_target_fingerprint_drift_is_rejected`、`cargo test --locked --lib bootstrap_phase_only_waives_missing_root_rules`；全部 PASS，失败无 provider_start 成功记录且零 child；正常缺 #8 返回可操作 Retry/Revalidate waiting，recipe 零回归。
- [ ] **Step 5：提交。** 3a/3b 仅暂存上列精确 Files，commit“校验 canonical 根政策正文与 target 身份”“在 spawn 前复验 LC 冻结事实与成员规则门”。`git add src/product/logical_codebase/provider_gateway.rs src/product/logical_codebase/provider_admission_check.inc.rs src/product/logical_codebase/provider_admission_preflight.rs src/product/logical_codebase/production_policy_resolvers.rs src/web/gateway_factory.rs src/product/logical_codebase/root_recipe_receipt.rs src/product/logical_codebase/root_recipe_receipt_types.inc.rs src/product/logical_codebase/root_recipe_receipt_store.inc.rs src/product/logical_codebase/root_recipe_receipt_auditor.inc.rs src/product/logical_codebase/root_recipe_receipt_tests.inc.rs src/product/logical_codebase/provider_trust.rs src/product/logical_codebase/provider_trust_adapters.rs src/product/logical_codebase/provider_trust_store.rs src/product/logical_codebase/bootstrap_projector.inc.rs src/product/logical_codebase/provider_admission_preflight_tests.inc.rs src/product/logical_codebase/provider_gateway_tests/task28_root_authority.inc.rs src/product/logical_codebase/provider_gateway_tests/audit.inc.rs && git commit -m "feat: 在 spawn 前复验 LC 冻结事实"`；上述 #8/trust owner 文件只做只读接线或由其 owner 提供接口，不改其 producer 算法。


## Task 4：Claude/Pi/Kimi真实权限projection（tasks.md 2.1；REQ-LCG-03/06）

**Files（4a/4b/4c各2–4h，Kimi目录必要时按provider进程→client两段串行）：**

- **4a独占：** Create `src/cross_cutting/claude_code_provider/projection.rs`；Modify `src/cross_cutting/claude_code_provider/mod.rs`（build_args/start_validated）、`src/cross_cutting/claude_code_provider/tests/mod.rs`（LC audit test fixture）；Test `src/cross_cutting/claude_code_provider/tests/policy_session.rs`、`src/cross_cutting/claude_code_provider/tests/args.rs`。
- **4b独占：** Create `src/cross_cutting/pi_provider/projection.rs`；Modify `src/cross_cutting/pi_provider/mod.rs`（build_args/start_validated/extension准备）、`src/cross_cutting/pi_provider/tests/mod.rs`（LC audit fixture）；Test `src/cross_cutting/pi_provider/tests/policy_session.rs`、`src/cross_cutting/pi_provider/tests/session_flow.rs`。
- **4c独占：** Create `src/cross_cutting/kimi_code_provider/projection.rs`；Modify `src/cross_cutting/kimi_code_provider/mod.rs`、`src/cross_cutting/kimi_code_provider/session.rs`、`src/cross_cutting/kimi_code_provider/client_services/mod.rs`、`src/cross_cutting/kimi_code_provider/client_services/fs_service.rs`、`src/cross_cutting/kimi_code_provider/client_services/fs_handlers.rs`、`src/cross_cutting/kimi_code_provider/client_services/terminal.rs`、`src/cross_cutting/kimi_code_provider/client_services/terminal_handlers.rs`、`src/cross_cutting/kimi_code_provider/client_services/sandbox.rs`；Test `src/cross_cutting/kimi_code_provider/tests/tool_policy_zero_change.rs`、`src/cross_cutting/kimi_code_provider/tests/session_tests.rs`、`src/cross_cutting/kimi_code_provider/tests/mcp_bundle_tests.rs`。`client_services/policy.rs` 只读，其矩阵不变；新增 LC 目标写门是独立路径边界，不能改角色决策表。

**Interfaces：** 具体输入输出、切片前置与汇合仅以 §0 为准。Task4 实现三家 `ProviderPolicyProjector` 与各自 `start_validated`，消费 `ProviderProjectionView`、action row、boundary plan/evidence 与 launch audit；Kimi 通用 tool 为 None，但仍执行 exact version probe、native session/handshake 与统一 `ProviderStartAudit.lc_projection` 落盘，不能因 `tool_policy=None` 早退。Kimi host root 保持 ro、target root rw，linked-worktree git-dir 沿冻结授权；fs read/write 与 terminal 都消费不可伪造 target boundary plan，terminal 实际 cwd 不改变 provider 进程 cwd。`mcp_bundle_digest` 只对应 Aria 注入，native 项目配置单独标来源。公共 projection.rs 不由并行 provider worker 编辑。

- [ ] **Step 1：写失败测试。** 4a `lcg_t04_claude_projection_has_headless_mcp_allowlist_and_deny_tokens`；4b `lcg_t04_pi_projection_has_exclude_write_tokens_and_root_cwd`；4c `lcg_t04_kimi_projection_separates_client_read_root_from_target_write_root` / `lcg_t04_kimi_native_mcp_is_not_aria_bundle` / `lcg_t04_kimi_host_fs_and_terminal_share_target_boundary`；各家增 `lcg_t04_projection_digest_changes_on_target_role_tool_or_config`、`lcg_t04_no_generic_tool_policy_still_records_version_audit_and_native_session`。断言：

```rust
assert!(claude_args.windows(2).any(|p| p == ["--disallowedTools", "Edit,Write,NotebookEdit"]));
assert!(claude_args.iter().any(|arg| arg == "--allowedTools"));
assert!(pi_args.windows(2).any(|p| p == ["--exclude-tools", "edit,write"]));
assert_eq!(provider_cwd, canonical_root);
assert!(target_write_allowed);
assert!(!root_or_other_member_write_allowed);
assert!(kimi_generic_tool_policy.is_none());
assert!(kimi_version_probe_recorded && kimi_provider_start_audit_recorded);
assert_eq!(kimi_native_session_id, kimi_handshake_session_id);
assert_ne!(original_projection_digest, changed_projection_digest);
```

- [ ] **Step 2：Main逐切片运行红测试。** `cargo test --locked --lib lcg_t04_claude`、`cargo test --locked --lib lcg_t04_pi`、`cargo test --locked --lib lcg_t04_kimi`；预期无 LC projector 或 client root 与 target 未分离失败。已有物理 deny token 测试本来绿，不能仅重命名为新红。
- [ ] **Step 3：实现 trait 和 validated 分支。** 四家 projection 只从 gateway 输入及 `ProviderProjectionView` 生成；真实 version 未知/adapter 不匹配拒绝。Claude headless MCP allowlist 只列既有合法工具，不扩大到 MCP 写工具；Pi extension 准备在 guard 之后，并在 launcher 覆盖面内，其 RPC/MCP 继续既有 adapter；Kimi `start_validated` 在 version/child 前拒非空通用策略，仍执行 exact version probe、统一 `ProviderStartAudit.lc_projection`、native session/handshake；ACP cwd 保持 root，read/write/terminal 宿主 handler 额外消费不可伪造 target boundary（含 git identity）。三家请求构造与最终 argv/wire/audit 同源，不把 logical roots 翻译成不存在的 CLI 参数。direct `start`/Kimi 原 ClientServicePolicy 表与 native 发现保持原值。
- [ ] **Step 4：Main运行绿测试。** 三个 provider 过滤与 `cargo test --locked --lib lcg_t04_projection_digest`、`cargo test --locked --lib kimi_four_role_client_service_table_stays_unchanged`；每条 LC start（含无通用 tool policy 的 Coding/Kimi）都有关联 version/wire/profile、native session/handshake 与 session projection digest。resume 严格字段由9补；不以 `tool_policy=None` 跳过统一 launch audit。
- [ ] **Step 5：提交。** 每家仅暂存其 Files，commit 分别“投影 Claude 的 LC 权限与 MCP”“投影 Pi 的 LC 角色与 RPC 权限”“绑定 Kimi 的 LC 独立目标写边界”；例如4b：`git add src/cross_cutting/pi_provider/projection.rs src/cross_cutting/pi_provider/mod.rs src/cross_cutting/pi_provider/tests/policy_session.rs src/cross_cutting/pi_provider/tests/session_flow.rs && git commit -m "feat: 投影 Pi 的 LC 角色与 RPC 权限"`。


## Task 5：Codex受限sandbox与wire（tasks.md 2.2；REQ-LCG-04）

**Files：** 5a/5b各3–4h串行；Create `src/cross_cutting/codex_provider/projection.rs`；Modify `src/cross_cutting/codex_provider/mod.rs`、`src/cross_cutting/codex_provider/session.rs`、`src/cross_cutting/codex_provider/tests/mod.rs`（validated audit/native session测试fixture）；Test `src/cross_cutting/codex_provider/tests/approval_policy.rs`、`src/cross_cutting/codex_provider/tests/bridging.rs`、`src/cross_cutting/codex_provider/tests/streaming.rs`。公共gateway/automation硬门归3/8，不由5并改。

**Interfaces：** 具体输入输出、切片前置与汇合仅以 §0 为准。Task5 实现 `CodexPolicyProjector::project(...)` 与 LC `codex_launch_params` 分支，消费 `CodexSandboxProjection`、action matrix 与 boundary；`CodexSandboxMode` 只在 projection 中限制，不修改 direct 默认 danger-full-access。Coding `process_cwd=root`、`protocol_cwd=target`，且 root discovery/trust/native 或 OS 写边界未被当前版本证实时拒绝。

- [ ] **Step 1：写失败测试。** `lcg_t05_danger_full_access_zero_child_for_fresh_resume_and_permission_modes`、`lcg_t05_planning_review_wire_is_read_only_on_request`、`lcg_t05_coding_requires_protocol_target_and_target_only_evidence`、`lcg_t05_version_or_protocol_cwd_drift_invalidates_projection`。断言所有Auto/Supervised/fresh/resume危险请求稳定reason，spawn0；合法只读与编码的真实RPC params：

```rust
assert_eq!(reason, "codex_danger_full_access_unsupported");
assert_eq!(spawn_count, 0);
assert_eq!(readonly_params["sandbox"], "read-only");
assert_eq!(readonly_params["approvalPolicy"], "on-request");
assert_eq!(coding_params["sandbox"], "workspace-write");
assert_eq!(coding_params["cwd"], target.to_string_lossy().as_ref());
assert_eq!(process_cwd, canonical_root);
assert!(missing_any_required_evidence.is_err());
```

- [ ] **Step 2：Main运行红测试。** `cargo test --locked --lib lcg_t05`；预期现有LC Codex被全provider硬拒，且Coder参数还是danger-full-access，新增合法受限分支断言FAIL；已有危险拒绝绿测试继续保留。
- [ ] **Step 3：实现受限projection/wire。** `CodexPolicyProjector`生成上节固定mapping；`codex_launch_params(input:&StreamingProviderInput)`原direct签名/分支保留，LC采用validated projection协议target，不以raw输入覆盖。`thread/start`、`thread/resume`共享同投影；policy fileChange/commandExecution审批保留拒绝，Coder仍沿ApprovalBridge映射Auto/Supervised。native根发现/protocol cwd/实际writable set未证实则`codex_target_boundary_unverified`或版本Unknown；OS边界也是Task6真实probe前置，不回危险模式、不改cwd/provider。
- [ ] **Step 4：Main运行绿测试。** `cargo test --locked --lib lcg_t05`、`cargo test --locked --lib codex_policy_session_answers_approvals_on_wire_without_bridge`、`cargo test --locked --lib resume_or_start_blocks_codex_danger_full_access_before_resume_decision`（迁移到LC projection判定后仍PASS）；RPC捕获与provider_start `sandbox/approval/cwd`一致，direct旧Coder参数测试仍绿。
- [ ] **Step 5：提交。** `git add src/cross_cutting/codex_provider/projection.rs src/cross_cutting/codex_provider/mod.rs src/cross_cutting/codex_provider/session.rs src/cross_cutting/codex_provider/tests/approval_policy.rs src/cross_cutting/codex_provider/tests/bridging.rs src/cross_cutting/codex_provider/tests/streaming.rs && git commit -m "feat: 对 LC Codex 强制受限 sandbox 与协议 target"`（5a/5b各自只暂存实际本段文件）。REQ-ENV-05 delta/main spec由Main实施完成后按OpenSpec流程同步，Task5不越权改验收。


## Task 6：四家真实写边界与D4证据门（tasks.md 2.3；REQ-LCG-03/07）

- **6a，launcher核心/挂载保护/后代 probe 三个3–4h段串行：** Modify `src/cross_cutting/provider_boundary.rs`（仅实现1a已定义 DTO 的 launcher/helper 行为，不创建 DTO）、`src/cross_cutting/process_manager.rs`、`src/cross_cutting/mod.rs`、`src/cross_cutting/kimi_code_provider/client_services/mod.rs`、`src/cross_cutting/kimi_code_provider/client_services/sandbox.rs`。本段在 Kimi 两个共享文件中只做 `frozen_writable_git_paths`/boundary helper 的 crate-visible 可见性和只读 mount-plan 先行接口，方法行为不改；Test 新模块与 `src/cross_cutting/process_manager/drop_tests.rs`。依赖与汇合见 §0。
- **6b，2–3h：** Modify `src/product/coding_workspace_engine/cross_target_check.rs`（baseline freshness 只读复检 seam）；Test `src/product/coding_workspace_engine/tests/provider_gateway_validated_input/d4_member_baseline.rs`。`src/product/coding_workspace_engine/provider_stream.rs` 的 run-bound audit 与 `tool_policy=None` 统一接线归 Task7 owner，6b 不并改；Kimi 宿主 target 门归4c，6b不并改Kimi目录。共享 owner 见 §0。
- **6c，每家≤4h现场切片：** Create `src/product/logical_codebase/provider_boundary_probe.rs`（`ProviderBoundaryProbe::run` evidence 校验/签发）；Modify `src/product/logical_codebase/mod.rs`（单 owner 接线）；Evidence目录 `cadence/reports/lc-gateway-multi-provider/{claude-code,codex,pi,kimi-code}/boundary/`；汇总 `cadence/reports/2026-10-03_验收报告_LC网关多provider写边界_v1.0.md`。源码合入后四家现场各自输出，不并跑 Cargo。共享 owner 见 §0。
**Interfaces：** 具体输入输出与前置仅以 §0 为准。Task6 消费 1a 定义的纯 `ProviderBoundaryPlan`/`ProviderBoundaryEvidence`/`ProviderBoundaryError` 与 `ProviderPolicyProjection` DTO、2c shape、4/5 native mapping 和6a launcher helper；不消费2d durable writer，也不定义这些 DTO 类型。Produces本 Task 的 probe/launcher 行为与 `BoundaryFixture`，以及 `ProviderBoundaryProbe::run(projection:&ProviderPolicyProjection,fixture:&BoundaryFixture)->Result<ProviderBoundaryEvidence,ProviderBoundaryError>` 的真实 evidence 实例；plan 必须携带 target/root/git-dir/metadata/provider-runtime mount，`start_validated`、ProcessManager/native/OS launcher 只能从同一 projection 取得 plan/evidence_ref。证据记录真实 provider/version/OS/profile/session/fixture digest、正负每 attempt/error、target 写+受控 commit/pre-post/D4。新增 `revalidate_cross_target_baseline(paths:&ProductAppPaths,attempt:&CodingExecutionAttempt,run_id:&str)->Result<(),StableCode>` 复用现 capture/detect。精确 Kimi boundary owner 为 `client_services/terminal.rs`、`sandbox.rs`、`fs_service.rs`、对应 handlers：root ro、target rw、linked git-dir 沿冻结授权，所有 fs/terminal 通道拒 symlink escape。bootstrap probe 在服务拥有的隔离临时 fixture 内，用 projector 候选物理映射+同一 ProcessManager boundary 启动真实 CLI/native 协议并留事实，不伪造 normal validated 权、不在用户 LC 执行 Unknown。2d 只消费该服务输出写 durable store；10/11 产品 E2E 仍经完整 normal gateway，evidence 不全立即降状态。

- [ ] **Step 1：写失败测试。** `lcg_t06_readonly_blocks_builtin_terminal_mcp_and_child_writes`、`lcg_t06_coding_allows_target_commit_and_denies_protected_roots`、`lcg_t06_missing_sandbox_or_namespace_keeps_unknown`、`lcg_t06_d4_baseline_drift_before_spawn_zero_child`、`lcg_t06_unobserved_or_over_budget_evidence_never_confirms`。OS试验不是模型自述，断言受控文件实际变化与错误：

```rust
assert_eq!(write_target_result, Ok(()));
assert!(controlled_target_commit_succeeded);
assert!(protected_attempts.iter().all(|attempt| attempt.was_refused_with_evidence()));
assert_eq!(protected_pre_snapshot, protected_post_snapshot);
assert_eq!(unavailable_boundary_state, ProviderCapabilityEvidence::Unknown);
assert_eq!(spawn_count_after_d4_drift, 0);
assert_ne!(missing_observation_state, ProviderCapabilityEvidence::Confirmed);
```

- [ ] **Step 2：Main运行红测试。** `cargo test --locked --lib lcg_t06`，无sandbox/namespace机器的能力case应明确Unknown而非skip→pass；真实bwrap强制case若不可运行返回环境阻断，不把ignored计支持。现有ProcessManager只env/process-group，新增文件写拒绝断言应红。
- [ ] **Step 3：实现boundary签名。** Linux产品-owned bwrap沿现有只读host/冻结target/git identity模式扩展，保持网络（provider API/MCP需要）和root原路径；不可照搬Kimi terminal的`--unshare-net`/cwd fd改`/tmp/work`到整个provider。明确各版本自然session/runtime目录可写集合并记录用途，不能整HOME、整root或MCP目录宽写；不可隔离的外部MCP写通道阻断。read-only所有LC代码/metadata只读；Coding仅target+既有授权git-dir+隔离temp/provider runtime写，后置只读挂载保护target `.git`指针和所有`.aria`。Kimi宿主file/terminal必须同时过4c target边界，不能只把ACP child sandbox后宣称host已隔离。namespace实际起子进程与mount失败不回plain spawn。D4重采全部active main checkout，差异/缺失在spawn前和交付前阻断。probe用于受控隔离fixture的探测通道，不是用户normal action的Unknown旁路；只有完整防护+真实正负探针才签`write_boundary=Confirmed`。
- [ ] **Step 4：Main验证并归档。** `cargo test --locked --lib lcg_t06`、`cargo test --locked --lib cross_target`；probe 执行与真实现场证据复用后续 E2E Task 10/11 的命令、fixture 与证据，不把后续 E2E 写成 Task6 的代码依赖。记录 actual argv/mount/errno/工具channel/版本，2c shape 校验通过后才允许2d导入 capability；现有预算：snapshot `max_entries=20_000`/`max_bytes=64 MiB`、inventory soft4096/hard8192B，沿HEAD不新立预算；FIFO/观测缺失/超限不截断冒充通过。
- [ ] **Step 5：提交。** 6a 三段按其精确 Files 提交“加入 LC 产品写边界 launcher”“保护 LC 元数据与后代写面”“锁定沙箱失败关闭”，只拓 Kimi 现有 git helper 可见性时显式暂存该二文件；6b：`git add src/product/coding_workspace_engine/cross_target_check.rs src/product/coding_workspace_engine/tests/provider_gateway_validated_input/d4_member_baseline.rs && git commit -m "feat: 复验 LC role-run D4 基线"`；6c：`git add src/product/logical_codebase/provider_boundary_probe.rs src/product/logical_codebase/mod.rs && git commit -m "test: 以真实探针签发 LC 写边界证据"`；`provider_stream.rs` 由 Task7 独占提交，报告日志不暂存。


## Task 7：Tool-policy双向guard与Kimi隔离控制面（tasks.md 3.1；REQ-LCG-06）

**Files：** 7a/7b 各1–3h，Task4/5 合入后串行追加：Modify `src/cross_cutting/streaming_provider/mod.rs`（共用 guard/LC run context）、`src/cross_cutting/streaming_provider/tests.rs`；Modify `src/product/coding_workspace_engine/provider_stream.rs`（`attach_tool_policy_audit` :110–157，取消 `tool_policy=None` 早退并统一 run-bound sink）；Modify `src/product/coding_workspace_engine/provider_stream/launch.rs`（`launch_provider_session` :19–51，validated+gateway 成对分流；LC 缺 validated/gateway 不得回裸 start）；Test `src/cross_cutting/claude_code_provider/tests/policy_session.rs`、`src/cross_cutting/codex_provider/tests/approval_policy.rs`、`src/cross_cutting/pi_provider/tests/policy_session.rs`、`src/cross_cutting/kimi_code_provider/tests/tool_policy_zero_change.rs`、`tests/it_web/provider_gateway_envelope.rs`。只有新回归发现 adapter 顺序不满足时修改相应 `mod.rs`；此 Task 为 guard/分流整体验收 owner，不能与4/5同时写 provider 目录。

**Interfaces：** 具体输入输出、切片前置与汇合仅以 §0 为准。Task7 消费既有 `validate_tool_policy_for_role(&AdapterRole,Option<&ProviderToolPolicy>)->Result<(),ToolPolicyGuardError>` 与 `start_validated`，产出 LC 同一 guard 顺序回归与 `provider_generic_tool_policy_forbidden` Kimi 稳定拒绝码（Kimi 实际拒绝由4c实现，此处不重复逻辑）。BootstrapExecutorMarker 仍只授权固定 Claude root recipe 完整凭据/receipt 上下文。

- [ ] **Step 1：写失败测试。** `lcg_t07_invalid_role_policy_zero_child_and_zero_extension` 覆盖三家×Orchestrator/Splitter/Reviewer 缺 deny、Executor/Handoff 带 deny；`lcg_t07_kimi_generic_policy_rejected_before_version_child`；`lcg_t07_root_recipe_marker_is_not_normal_coder_policy`；`lcg_t07_validated_launch_never_invokes_raw_start_or_run`；`lcg_t07_lc_tool_policy_none_still_allocates_audit_sink`。用真实 process/extension seam 计数而非 grep 源码，断言：

```rust
assert!(invalid_role_policy_result.is_err());
assert_eq!(session_child_count, 0);
assert_eq!(rpc_handshake_count, 0);
assert_eq!(extension_start_count, 0);
assert_eq!(raw_start_count_for_lc, 0);
assert_eq!(raw_run_count_for_lc, 0);
assert_eq!(validated_start_count_for_lc, 1);
assert_eq!(validated_run_count_for_lc, 1);
assert!(audit_sink_allocated_when_tool_policy_is_none);
assert_eq!(kimi_reason, "provider_generic_tool_policy_forbidden");
assert!(valid_recipe_marker_result.is_ok());
assert!(same_marker_on_ordinary_coder.is_err());
```
- [ ] **Step 2：Main运行红测试。** `cargo test --locked --lib lcg_t07`；三家原 guard 本来绿，新 validated 身份/extension 顺序、`provider_stream/launch.rs` 裸 start 分流、`provider_stream.rs` None 早退、Kimi拒绝新增断言为红；不可通过删 Kimi 或 Bootstrap 案例缩窄矩阵。
- [ ] **Step 3：对齐 guard 与分流调用。** LC `start_validated` 验证 projection role 与 input role 后调用共用 guard，早于版本/session child/extension；gateway `start_streaming/run_sync` 只调用 adapter `start_validated/run_validated`。`launch_provider_session` 仅非 LC legacy 直接 `provider.start`；LC validated/gateway 缺失返回稳定错误，禁止回退。`attach_tool_policy_audit` 不因 `tool_policy=None` 返回，Coder/Kimi 也分配 run-bound sink；所有错误直接返回，不删 policy 再尝试。Kimi 只拒非空通用策略，原24格 ClientServicePolicy 不改，宿主写边界另由4c实现。valid bootstrap marker 校验 durable operation 与 receipt context，普通 Coder 不借 marker 获取 root 写权。
- [ ] **Step 4：Main运行绿测试。** `cargo test --locked --lib lcg_t07`、`cargo test --locked --lib adapter_tool_policy_guard_is_bidirectional_for_every_role`、`cargo test --locked --lib kimi_four_role_client_service_table_stays_unchanged`、`cargo test --locked --lib bootstrap_executor_requires_credential_and_receipt_context`；预期 PASS 且 guard 拒绝早于全部 session 执行面，LC raw start/run 计数为0，validated 计数与 audit 均出现。最后一项现存在 lib 的 streaming_provider/tests.rs，不误用 it_web 过滤0 tests。
- [ ] **Step 5：提交。** `git add src/cross_cutting/streaming_provider/mod.rs src/cross_cutting/streaming_provider/tests.rs src/product/coding_workspace_engine/provider_stream.rs src/product/coding_workspace_engine/provider_stream/launch.rs src/cross_cutting/claude_code_provider/tests/policy_session.rs src/cross_cutting/codex_provider/tests/approval_policy.rs src/cross_cutting/pi_provider/tests/policy_session.rs src/cross_cutting/kimi_code_provider/tests/tool_policy_zero_change.rs tests/it_web/provider_gateway_envelope.rs && git commit -m "test: 锁定 LC 角色工具守卫与 validated 分流"`；发生实现顺序修正时仅补列本段对应 provider/mod.rs。


## Task 8：Automation同源role-chain全量预检（tasks.md 3.2；REQ-LCG-06）

**Files：** 8a/8b各2–4h；Modify `src/web/handlers/automation_gateway_preflight.rs`、`src/web/handlers/automation_target.rs`（GET）、`src/web/handlers/automation_enrollment.rs`（Enable/rebind）；Test各文件内现存tests；Read-only `src/web/handlers/support.rs`的carrier resolver（沿用唯一入口）、Task3 gateway/verdict与factory。不得另写一张静态provider allow矩阵。

**Interfaces：** 具体输入输出、切片前置与汇合仅以 §0 为准。Task8 消费 gateway 的 `action_admission_verdict`、carrier/declared target 与 readonly factory，产出 `AutomationRoleChainViolation { role:String,provider:String,action:SessionPolicyAction,reason_code:String,capability_snapshot_ref:Option<String>,projection_ref:Option<String> }`。两个入口完整新签名为 `validate_role_chain_for_enrollment(gateway:Option<&LogicalCodebaseProviderGateway>,author_provider:&ProviderName,reviewer_provider:&ProviderName,carrier:&AutomationCarrierResolution,gateway_required:bool,test_provider_enabled:bool)->ApiResult<()>` 和 `validate_role_chain_for_declared_enrollment_target(gateway:Option<&LogicalCodebaseProviderGateway>,author_provider:&ProviderName,reviewer_provider:&ProviderName,declared:&EnrollmentTarget,gateway_required:bool,test_provider_enabled:bool)->ApiResult<()>`。

- [ ] **Step 1：写失败测试。** `lcg_t08_preflight_lists_all_role_provider_action_evidence_failures`（Pi boundary Unknown、Codex coding缺protocol evidence、Kimi trust缺失同场），`lcg_t08_lc_gateway_false_still_checks_all_roles`、`lcg_t08_single_repository_skips_lc_predicates`、`lcg_t08_get_enable_rebind_share_verdict`。断言：

```rust
assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
assert_eq!(payload["code"], "automation_role_chain_unsupported");
assert_eq!(roles, ["plan_author", "coder", "plan_reviewer", "code_reviewer", "internal_reviewer"]);
assert!(violations.iter().all(|v| v.get("action").is_some() && v.get("reason_code").is_some()));
assert_eq!(provider_start_count, 0);
assert_eq!(lc_false_flag_verdict, lc_true_flag_verdict);
assert!(single_repository_verdict.is_ok());
assert_eq!(get_reasons, enable_reasons);
assert_eq!(enable_reasons, rebind_reasons);
```

- [ ] **Step 2：Main运行红测试。** `cargo test --locked --lib lcg_t08`；预期当前静态谓词只列role/provider/reason、`!gateway_required`直接return且不读能力/trust/projection，新增断言FAIL。
 - [ ] **Step 3：实现同源聚合。** 每个 role 依次读取显式 provider、action 和 Task3 early 资格 verdict，收集全部可确定 capability/trust/projection-profile/adapter/boundary/tool 错误；不要求尚不存在的 attempt worktree 或 role-run D4，实际 launch 继续完整门且无豁免。Codex 按 LC 受限 profile 而非 direct 全局默认判定，危险 profile 仍拒。LC 忽略 false 绕过，SingleRepository 先走原跳过，Fake 仅 test 隔离。GET/Enable 从同 carrier，rebind 沿 declared target 读取现有 LC/成员而不猜 issue；缺 checkout 材料列 reason，不建 worktree/provider。Task3 提供 readonly factory 读取，不运行 `ensure_bootstrap`，GET 不写 policy/capability/trust audit；根事实仍在真实 spawn 复验。
- [ ] **Step 4：Main运行绿测试。** `cargo test --locked --lib lcg_t08`、`cargo test --locked --lib role_chain_preflight_skips_gateway_predicates_for_single_repository`、`cargo test --locked --lib single_repository_enable_authorizes_physical_target`；预期PASS，旧“Pi/Kimi恒静态unsupported”测试迁为“缺当前action证据时拒、证据完整时通过”，不删除硬拒路径。
- [ ] **Step 5：提交。** `git add src/web/handlers/automation_gateway_preflight.rs src/web/handlers/automation_target.rs src/web/handlers/automation_enrollment.rs && git commit -m "feat: 以同源 LC 门禁聚合 automation 全角色违规"`（8a核心DTO先提交，8b调用者接线后提交；共享文件串行）。


## Task 9：四家原生resume与显式fresh语义（tasks.md 3.3；REQ-LCG-05）

**Files（均在Task4/5/7完成后）：**

- **9a，核心+完整字面量/指纹 caller 迁移为一段≤4h原子 commit，决策测试为下一段≤4h：** Modify `src/product/logical_codebase/provider_gateway.rs`、`src/cross_cutting/tool_policy_audit.rs`、`src/product/lifecycle_store/tool_policy_run_audit.rs`；Test `src/product/logical_codebase/provider_gateway_tests.rs`、`src/product/logical_codebase/provider_gateway_tests/task28_root_authority.inc.rs`、`src/product/lifecycle_store/tests/tool_policy_audit.rs`、`src/product/work_item_split_engine/tests/engine_gateway_guard.rs`。新增 audit 完整字面量一起迁 `src/cross_cutting/claude_code_provider/mod.rs`、`src/cross_cutting/codex_provider/mod.rs`、`src/cross_cutting/pi_provider/mod.rs`、`src/cross_cutting/claude_code_provider/tests/policy_session.rs`、`src/cross_cutting/pi_provider/tests/policy_session.rs`、`src/cross_cutting/codex_provider/tests/approval_policy.rs`；不先提交缺字段半编译版本。`SessionResumeFingerprint::from_envelope` 的生产 caller 是 `provider_gateway.rs` 两处，split sync 测试 caller 是 `engine_gateway_guard.rs`，必须同一原子迁移。
- **9b，Claude/Codex各2–4h可并行：** Modify `src/cross_cutting/claude_code_provider/mod.rs`、`src/cross_cutting/codex_provider/mod.rs`、`src/cross_cutting/codex_provider/session.rs`；Test `src/cross_cutting/claude_code_provider/tests/policy_session.rs`、`src/cross_cutting/codex_provider/tests/approval_policy.rs`、`src/cross_cutting/codex_provider/tests/bridging.rs`；没有 Codex `tests/policy_session.rs`，不能把 Claude 路径省略后泛化给 Codex。
- **9c，Pi/Kimi各2–4h并行，engine段2–4h串行：** Modify `src/cross_cutting/pi_provider/mod.rs`、`src/cross_cutting/pi_provider/session.rs`、`src/cross_cutting/kimi_code_provider/mod.rs`、`src/cross_cutting/kimi_code_provider/session.rs`；Test `src/cross_cutting/pi_provider/tests/policy_session.rs`、`src/cross_cutting/pi_provider/tests/session_flow.rs`、`src/cross_cutting/kimi_code_provider/tests/session_tests.rs`、`src/cross_cutting/kimi_code_provider/mcp_bundle_tests.rs`。engine/显式 fresh 段 Modify `src/product/workspace_engine/review/drive.rs`、`src/product/workspace_engine/lifecycle.rs`、`src/product/workspace_engine/provider_drive/author_root_launch.inc.rs`、`src/product/coding_workspace_engine/provider_retry.rs`、`src/web/workspace_ws_handler/decisions/inbound.rs`、`src/web/workspace_ws_handler/protocol.rs`、`src/web/workspace_ws_handler/run/provider_run.rs`、`src/web/workspace_ws_handler/run/gateway_start.rs`；Test `src/product/workspace_engine/tests/author_revision_loop.rs`、`src/product/workspace_engine/tests/author_revision_loop_parts/root_cwd_revision.inc.rs`、`src/web/workspace_ws_handler/tests/planning_resume.rs`。`StartNew` 只落 durable superseded/稳定 waiting action，不在同一 revision driver fresh；显式用户动作复用 `WsInMessage::StartGeneration`→`WorkspaceEngine::start_generation`，所有 Author/ChoiceFollowup/Revision/Plan/review caller 均复核。

**Interfaces：** 具体输入输出、切片前置与汇合仅以 §0 为准。Task9 产出 `SessionResumeFingerprint::from_envelope(envelope:&SessionPolicyEnvelope,projection:&ProviderPolicyProjection,action_evidence_digest:&str,target_git_identity:&str)->SessionResumeFingerprint` 与 `LcProviderStartAudit`；from_envelope 全 production/test caller 迁移。新增 gateway 内部 `resume_or_start_with_projection(&self,request:ResumeSessionLaunchRequest,projection:ProviderPolicyProjection)->Result<GatewaySessionDisposition,ProviderGatewayError>`，调用者先完整 prepare 得到候选 projection 再比指纹；旧 `resume_or_start` 可委托完整上下文判定，不能用仅 envelope 指纹生成可 spawn 权。`GatewaySessionDisposition::StartNew` 仅 supersede 并等待显式 fresh；缺旧 LC v2 字段拒 resume，direct 三元组保留。同步 split resume 没有 native session contract 时记录 `resume=Unknown`、零 spawn，不借 streaming fresh 推导支持。

- [ ] **Step 1：写失败测试。** `lcg_t09_matching_full_fingerprint_resumes_native_session`、`lcg_t09_each_fingerprint_dimension_supersedes_without_fresh_spawn`、`lcg_t09_legacy_audit_missing_projection_cannot_resume_lc`、`lcg_t09_explicit_resume_unknown_zero_spawn`、`lcg_t09_start_new_waits_for_explicit_start_generation`、`lcg_t09_claude_missing_or_wrong_native_id_never_fresh`、`lcg_t09_codex_missing_or_wrong_native_id_never_fresh`、`lcg_t09_pi_missing_or_wrong_native_id_never_fresh`、`lcg_t09_kimi_missing_or_wrong_native_id_never_fresh`、`lcg_t09_kimi_bundle_drift_never_session_new`、`lcg_t09_split_sync_resume_unknown_is_not_streaming_fresh`。断言：

```rust
assert!(matches!(matching, GatewaySessionDisposition::Resume(_)));
assert!(matches!(drifted, GatewaySessionDisposition::StartNew { .. }));
assert_eq!(decision_spawn_count, 0);
assert_eq!(fresh_after_explicit_resume_count, 0);
assert!(old_run_superseded_recorded);
assert!(start_new_waiting.allowed_actions.contains(&BootstrapActionKind::StartGeneration));
assert_eq!(revision_driver_provider_start_count, 0);
assert_eq!(confirmed_native_id, requested_native_id);
assert!(native_id_missing_result.is_err());
assert!(started_child_was_killed_and_reaped);
assert_eq!(split_sync_resume_state, ProviderCapabilityEvidence::Unknown);
```

 - [ ] **Step 2：Main运行红测试。** `cargo test --locked --lib lcg_t09`；现指纹仅部分维度、三家 adapter 可清 id 后直接 fresh、Pi get_state/Kimi session 响应可回填 request id，新增严格 LC 断言应 FAIL。旧 direct 漂移后 fresh 测试继续绿。
- [ ] **Step 3：实现签名与原生确认。** fingerprint 长度分隔、schema 域版本、canonical authority/cwd/target/git/每项 digest，旧 LC 没有 v2 材料即 supersede；fresh 不读取旧 resume 行。三家 `start_validated` 在 child 前比较全 LC audit，drift/Unknown 零 child；原生恢复 Claude 需真实 native 会话确认，Codex `thread/resume` 必须应答同 id，Pi `--session-id`+RPC `get_state` 必须真实同 id 且不回填请求，Kimi `session/load` 应答与能力协商确认同 id，不因 bundle 漂移走 session/new。握手发现不支持/错 id 属于已启动 child 后的 runtime 失败，必须 kill/reap 并记录“未恢复”，不伪称零 spawn；spawn 前门失败才要求零 spawn。`StartNew` 只持久化 superseded 与稳定 waiting action，由现有 `WsInMessage::StartGeneration`/`WorkspaceEngine::start_generation` 接受显式 fresh；同一 revision driver 不得继续 `start_streaming`，用户显式动作后重新 prepare/revalidate。split sync 无 native session contract 时仅记录 Unknown/zero spawn；SingleRepository 调用旧 helper 原行为。
- [ ] **Step 4：Main运行绿测试。** `cargo test --locked --lib lcg_t09`、`cargo test --locked --lib planning_resume`、`cargo test --locked --lib resume_rejects_digest_version_or_dialect_drift_and_missing_record`；严格 LC 与 legacy direct 均 PASS，durable superseded 落旧 run，sink 缺失/写失败沿 kill 链传播，StartNew 不在 revision driver 静默 fresh。
- [ ] **Step 5：提交。** 9a 类型/全部字面量 caller 原子提交，9b/9c/provider 各自按精确 Files 单独提交；engine 显式 fresh 段提交“禁止 LC 显式恢复静默转 fresh”。现场证据不暂存。


## Task 10：四家五阶段真实fresh/resume E2E（tasks.md 4.1；REQ-LCG-07）

- **Files：** 10a 3–4h：Create `tests/it_web/web_lc_gateway_multi_provider/mod.rs`、`tests/it_web/web_lc_gateway_multi_provider/harness.rs`、`tests/it_web/web_lc_gateway_multi_provider/live_matrix.rs`；Modify `tests/it_web.rs`（唯一 module 接线 owner）。10b–10e 每家现场一片≤4h：Evidence `cadence/reports/lc-gateway-multi-provider/<provider>/matrix/`，provider 目录固定为 `claude-code`、`codex`、`pi`、`kimi-code`；汇总仅 Task12 写 `cadence/reports/2026-10-03_验收报告_LC网关多provider真实E2E_v1.0.md`。临时 non-Git root/main/worktree/store 在隔离 tempdir，未经审计不覆盖用户文件。Task10 生产入口 Files 还包括 `src/web/workspace_ws_handler/run/provider_run.rs`、`src/web/workspace_ws_handler/run/provider_run/work_item_plan_legacy_author.inc.rs`、`src/web/workspace_ws_handler/run/followups.rs`、`src/web/workspace_ws_handler/run/single_candidate.rs`、`src/web/workspace_ws_handler/run.rs`、`src/web/workspace_ws_handler/run/gateway_start.rs`；现场不改源码，但 harness/证据必须覆盖其 `start_work_item_plan_author` caller。

**Interfaces：** 具体输入输出、切片前置与汇合仅以 §0 为准。Task10 产出 test-only `LiveLcGatewayHarness::run_provider_matrix(provider:ProviderName,evidence_root:&Path)->Result<LiveMatrixEvidence,LiveMatrixFailure>`、`LiveMatrixEvidence`（Task10 唯一定义）及 `EvidenceCell` 键=`provider/exact_version/stage/entrypoint/fresh_or_resume`。复用 `tests/it_web/web_lc_operations_api.rs::{request_json,LcOperationsFixture}` 的 HTTP 形态与真实 git fixture 建法，但不复用 Noop/Fake 驱动当真实证据；`entrypoint` 必须枚举 `workspace_streaming_plan/split` 与 `split_sync` 两种 Plan/split 栈，二者各有独立 run_ref、launch/resume evidence 和 capability cell；workspace streaming 主入口是 `start_work_item_plan_author` caller 链，`WorkItemSplitEngine::generate/generate_revision` 仅代表独立 `split_sync` 对照。

- [ ] **Step 1：写失败测试/证据断言。** 新增四个 `#[ignore]`真实测试 `lcg_live_claude_five_stages_fresh_resume`、`lcg_live_codex_five_stages_fresh_resume`、`lcg_live_pi_five_stages_fresh_resume`、`lcg_live_kimi_five_stages_fresh_resume`；harness test `lcg_t10_evidence_requires_exact_version_wire_and_native_confirmation` 用缺字段记录测试拒绝（此单测只验证证据结构）。真实断言：

```rust
assert_eq!(cell.provider, selected_provider);
assert_eq!(cell.process_cwd, canonical_root);
assert_eq!(cell.target, selected_member_worktree);
assert!(!cell.exact_version.is_empty());
assert_eq!(cell.audit_projection_digest, cell.frozen_projection_digest);
assert_eq!(cell.native_resume_confirmed_id, cell.requested_resume_id);
assert!(cell.argv_or_wire_capture_exists && cell.approval_and_tool_events_exist);
assert!(cell.completed_product_artifact_exists);
assert!(cell.entrypoint == "workspace_streaming_plan/split" || cell.entrypoint == "split_sync");
assert!(cell.run_ref_is_unique_within_entrypoint);
```

- [ ] **Step 2：Main运行红证据。** `cargo test --locked --test it_web lcg_t10_evidence_requires_exact_version_wire_and_native_confirmation -- --nocapture`；新 harness/字段缺失应 FAIL。首次真实执行下列命令，预期现有未完整格按缺前置明确阻断，不手工填 Confirmed。`#[ignore]` 默认不跑不是 E2E 通过，执行时必须显式开关且 `--ignored` 命中新测试；开关缺失要失败而非 return 成功。
 - [ ] **Step 3：实现 harness 与五阶段顺序。** 每家新 LC 先产品创建/真实登记/trust，再固定 Claude recipe，再取得 #8 最终 policy 文件/artifact/receipt，最后真实索引 ready；五阶段顺序固定为 Story、Design、Plan、Coding、Review，各阶段分别执行 fresh/resume。Plan streaming 的 `start_work_item_plan_author` 与 `split_sync` 的 `generate/generate_revision` 分列证据；WS Plan/split 每个 fresh/revision/retry 先 begin `WorkItemSplitProviderRunHandle`、绑定 sink、streaming start，再 parse/complete/fail。若 sync split 没有 native session contract，记录 `resume=Unknown`、zero spawn，不借 streaming fresh 推导支持。产品确认走真实 HTTP/WS gate，#13 Fake 默认只用已有 provider_select，不直写状态或能力。
- [ ] **Step 4：Main执行四家真实矩阵。** 从 worktree 根运行（新增开关只选测试与证据落点，不向 provider 注入政策）：

```bash
LC_GATEWAY_E2E=1 cargo test --locked --test it_web lcg_live_claude_five_stages_fresh_resume -- --ignored --nocapture --test-threads=1
LC_GATEWAY_E2E=1 cargo test --locked --test it_web lcg_live_codex_five_stages_fresh_resume -- --ignored --nocapture --test-threads=1
LC_GATEWAY_E2E=1 cargo test --locked --test it_web lcg_live_pi_five_stages_fresh_resume -- --ignored --nocapture --test-threads=1
LC_GATEWAY_E2E=1 cargo test --locked --test it_web lcg_live_kimi_five_stages_fresh_resume -- --ignored --nocapture --test-threads=1
```

每个 case 输出 `evidence_root/cell` 与实际 result；四家×五阶段×fresh/resume 至少40主格，Plan streaming/split_sync 与 Review 子入口另列，不能合并隐藏。版本记录必须重新读取当前真实 CLI（旧报告版本仅起点，不直接钉支持）。任一缺证据格记录 Unknown/Denied+reason；成功支持格具全部断言才 PASS；环境不可运行须报告 BLOCKED，不能删格宣布 change 全通过。
- [ ] **Step 5：提交 harness。** `git add tests/it_web.rs tests/it_web/web_lc_gateway_multi_provider/mod.rs tests/it_web/web_lc_gateway_multi_provider/harness.rs tests/it_web/web_lc_gateway_multi_provider/live_matrix.rs && git commit -m "test: 建立四家 LC 五阶段真实 fresh 与 resume 矩阵"`；现场各家不 commit 报告，Main 收口落盘。

**每格证据形态：** `cell.json` 含 stage/action/role/entrypoint/provider/version/gateway+wire dialect/native id/session/run；`provider-events.jsonl` 含真实 argv 或脱敏 RPC/ACP 请求响应、tool_call/tool_result/approval；`frozen-facts.json` 含 policy_id/revision/raw-body/capability row/projection/trust/适用 bundle digest 与 canonical cwd/target/git identity；`pre-snapshot.json`/`post-snapshot.json`、D4 baseline 引用、`boundary-attempts.jsonl`、`result.json` 与 sha256 清单。敏感 token/API key/home 无关 trust 条目不落盘；provider-start 真实 wire 不能空 argv 占位，子进程 PID/时间线需可追溯。


## Task 11：越界写、D4与失败零spawn真实验收（tasks.md 4.2；REQ-LCG-07）

**Files：** 11a 2–4h：Create `tests/it_web/web_lc_gateway_multi_provider/failure_matrix.rs`、`tests/it_web/web_lc_gateway_multi_provider/boundary_matrix.rs`；Modify `tests/it_web/web_lc_gateway_multi_provider/mod.rs`（10a后串行include）；四家现场11b–11e每片≤4h，Evidence `cadence/reports/lc-gateway-multi-provider/<provider>/boundary/`、`failures/`。汇总 `cadence/reports/2026-10-03_验收报告_LC网关多provider写边界_v1.0.md` 与Task6同一report，唯一Main整合owner；不并改一个报告。

**Interfaces：** 具体输入输出、切片前置与汇合仅以 §0 为准。Task11 消费 boundary/D4 与10 harness，产出 `LiveLcGatewayHarness::run_boundary_and_failure_matrix(provider:ProviderName)->Result<LiveMatrixEvidence,LiveMatrixFailure>` 同 schema。spawn统计session/协议child、extension/MCP后代、native方法次数，availability `--version`另记不冒充session。完整fixture失败证据都逐格保留；不能观察OS保护则Unknown。

- [ ] **Step 1：写失败测试。** `lcg_t11_zero_spawn_evidence_counts_process_and_protocol_children`；四个真实名明确为 `lcg_live_boundary_and_failures_claude`、`lcg_live_boundary_and_failures_codex`、`lcg_live_boundary_and_failures_pi`、`lcg_live_boundary_and_failures_kimi`。负向固定缺readiness/正文/receipt/trust、launch/boundary Unknown/Denied、version/wire漂移、Codex danger、role非法、false标志、resume Unknown/fingerprint、target/git pointer与D4缺失/漂移；实际waiting/reason与计数断言：

```rust
assert_eq!(rejected_session_child_count, 0);
assert_eq!(rejected_native_handshake_count, 0);
assert!(!rejected_provider_start_success_exists);
assert!(coding_target_controlled_file_written);
assert!(protected_writes_all_refused_by_os_or_native_policy);
assert_eq!(protected_before, protected_after);
assert!(d4_detected_or_delivered_without_non_target_drift);
assert_ne!(incomplete_cell_state, ProviderCapabilityEvidence::Confirmed);
```

- [ ] **Step 2：Main运行红测试。** `cargo test --locked --test it_web lcg_t11_zero_spawn_evidence_counts_process_and_protocol_children -- --nocapture`；预期现有harness只数成功audit不能证零child而FAIL。真实baseline用旧无boundary配置的受控fixture应可写root/非target（旧报告已有ground truth，不重新确认旧失败）；新增实验直接使用新边界，不为复现已知失败跑危险旧CLI。
- [ ] **Step 3：实现真实探针。** Coding正向在target创建唯一受控文件、真实git add/commit；只读格target写也必须拒。分别经builtin write、terminal、extension、MCP和child尝试root/非target main checkout/非target worktree/root与非target`.git`/所有`.aria`/target `.git`指针/越界symlink。每条有tool或OS拒绝原文及文件digest，模型“不写”不是attempt证据。先前存在文件只比摘要，不覆盖；probe未知变化立即保留现场并阻断、清理只移除本case生成的受控文件。D4沿现有所有active main快照（HEAD+porcelain）并加root/metadata快照，预算/特殊文件不可观测不截断。child spawn失败分清“创建失败=0”与“握手失败已创建=kill/reap”，不能把后者计入零spawn成功格。
- [ ] **Step 4：Main真实验证。** 对四家分别执行 `LC_GATEWAY_E2E=1 cargo test --locked --test it_web lcg_live_boundary_and_failures_claude -- --ignored --nocapture --test-threads=1`，同命令过滤后缀替换为`codex`、`pi`、`kimi`（四个测试名在Step1逐项定义）。每种失败都有真实process观测+durable拒绝两面，成功boundary具预防+正向+负向+D4+清理链；只读所有代码root无写，Coding成员main零越界。给出缺证据格、实际argv/OS/version，而非“整体通过”掩盖。
- [ ] **Step 5：提交测试。** `git add tests/it_web/web_lc_gateway_multi_provider/mod.rs tests/it_web/web_lc_gateway_multi_provider/failure_matrix.rs tests/it_web/web_lc_gateway_multi_provider/boundary_matrix.rs && git commit -m "test: 验证 LC 越界写与失败零 provider 启动"`；报告/原日志不commit。


## Task 12：#8交付门、recipe与真实单仓对照（tasks.md 4.3；REQ-LCG-02/07）

**Files：** 12a/12b 各2–4h；Create `tests/it_web/web_lc_gateway_multi_provider/policy_dependency.rs`、`tests/it_web/web_lc_gateway_multi_provider/direct_comparison.rs`；Modify `tests/it_web/web_lc_gateway_multi_provider/mod.rs`（串行）；Evidence `cadence/reports/lc-gateway-multi-provider/dependency-8/`、`direct-comparison/`；Main 汇总 `cadence/reports/2026-10-03_验收报告_LC网关多provider真实E2E_v1.0.md`。Read-only #8 change 与 `src/web/handlers/aggregate_initialization/production_dependencies.inc.rs`、root receipt/recipe，不能为了跑矩阵手工发布政策。

**Interfaces：** 具体输入输出、切片前置与汇合仅以 §0 为准。Task12 消费10/11格证据、#8 `canonical_root/policy_id`+raw UTF-8 SHA-256/artifact/receipt/revision链、固定 Claude recipe 以及两类 direct 对照：`task_run/provider_factory.rs` 现有 Claude/Codex sync direct 两槽（Pi/Kimi reject）与四家 workspace streaming `start` legacy path。Produces test-only `LiveLcGatewayHarness::assert_policy_publication_chain()->Result<(),LiveMatrixFailure>` 与总矩阵结论；没有新 policy publisher/fixture API，不把 workspace streaming direct 对照写成四家同步 direct 支持。

- [ ] **Step 1：写失败测试。** `lcg_t12_missing_publication_blocks_every_provider_without_fixture_seed`、`lcg_t12_body_revision_receipt_drift_blocks_before_spawn`、`lcg_t12_single_repository_sync_direct_two_slot_comparison`、`lcg_t12_workspace_streaming_legacy_four_provider_comparison`、真实 `lcg_live_fresh_lc_policy_publication_recipe_and_direct_comparison`。断言：

```rust
assert_eq!(raw_body, artifact.policy_text.as_bytes());
assert_eq!(format!("sha256:{:x}", Sha256::digest(&raw_body)), artifact.digest);
assert_eq!(receipt.policy_digest, artifact.digest);
assert_eq!(receipt.canonical_root, canonical_root);
assert_eq!(policy_revision_in_locator, artifact.revision);
assert_eq!(recipe_provider, ProviderName::ClaudeCode);
assert_eq!(recipe_step_count, 5);
assert_eq!(recipe_command_count, 4);
assert_eq!(spawn_count_with_missing_body, 0);
assert_eq!(sync_direct_after, sync_direct_before);
assert_eq!(workspace_streaming_after, workspace_streaming_before);
assert_eq!(pi_kimi_sync_result, Err(DirectProviderUnsupported));
```

- [ ] **Step 2：Main运行红测试。** `cargo test --locked --test it_web lcg_t12 -- --nocapture`；新消费 gate 断言应红；若 #8 未产品化，真实成功 case 必须标外部依赖 BLOCKED，不 fixture 物化后改成绿。缺件/漂移负向 case 不依赖 #8 成功交付，可先执行。
- [ ] **Step 3：实现只读验收与汇合。** 新 fresh fixture 全部由产品 API/真实 Claude recipe 与 #8 producer 提供根政策正文/locator/artifact/receipt；consumer 只校验 `policy_id` 文件原字节等于 `artifact.policy_text`、raw SHA-256 等于 artifact/receipt.policy_digest，独立 `rule_digest` 校验 AGENTS 原字节，不在网关重定/执行 producer 聚合。成功 E2E 前置是 #8 明确提交与无 seed fresh/存量重冻结证据已由 Main 验收，再从新 LC 起跑10/11；旧 v1.1 手工物化不能替代。两类 direct 对照分别记录 argv/cwd/permission/output baseline，recipe 在 normal 全 Unknown 时仍固定凭据/原 recipe evidence。逐格关联 REQ/任务/证据，缺件 waiting 不可改为 allow。
- [ ] **Step 4：Main真实验证与契约校验。** `LC_GATEWAY_E2E=1 cargo test --locked --test it_web lcg_live_fresh_lc_policy_publication_recipe_and_direct_comparison -- --ignored --nocapture --test-threads=1`；预期 #8 完全产品化后 fresh 链无手工 seed 且 direct 两类对照一致。`openspec validate lc-gateway-multi-provider` 与 `openspec status --change lc-gateway-multi-provider` 校验规划/契约，不当作 CLI 验收。旧报告 #8/#10/#13 台账只交叉引用，#10/#13 层1仍原样非目标。
- [ ] **Step 5：提交测试/交付报告。** `git add tests/it_web/web_lc_gateway_multi_provider/mod.rs tests/it_web/web_lc_gateway_multi_provider/policy_dependency.rs tests/it_web/web_lc_gateway_multi_provider/direct_comparison.rs && git commit -m "test: 锁定政策发布前置与两类 direct 对照"`；Main 落总报告，报告不暂存。若 #8 未交付，Task12 验收状态 BLOCKED，已完成负向证据保留；任务不能勾完成或归档。


## Task 13：Direct/GitFinalize/人工StartCoding零回归（tasks.md 5.1；REQ-LCG-02）

**Files：** 13a/13b 各2–3h；Modify/Test `tests/it_web/provider_gateway_envelope.rs`（dual stack/direct 输出对照）、`src/product/work_item_split_engine/tests/engine_gateway_guard.rs`（split_sync direct）、`src/web/coding_start_parts/tests.inc.rs`（人工 StartCoding）、`src/web/coding_ws_handler/tests/sc_start_guard.rs`（先 advance 门）、`src/web/handlers/aggregate_initialization/tests_parts/root_safety.inc.rs`（D3 recipe 隔离）；Test `tests/it_web/web_repository_initialization/operation_http.rs`（六步 GitFinalize 既有锁）。此 Task 测试独占，Task7/9 改同名文件完成后再运行；同源角色链 case 归8，不并改 automation 实现。

**Interfaces：** 具体输入输出、切片前置与汇合仅以 §0 为准。Task13 消费两类兼容合同并明确不扩大范围：
1. `task_run/provider_factory.rs` 的现有 Claude/Codex 同步 direct routing 两槽保持不变；Pi/Kimi 明确 reject，不能写成四家同步 direct 交付。
2. 四家 workspace streaming direct legacy path（`StreamingProviderAdapter::start`）作为单独对照，保留既有 raw streaming 行为；它不是 split_sync、不是 LC validated gateway，也不能被对照测试升级为本 change 的四家同步支持。另消费 `StartCodingCommand`、`CodingStartOrigin::Manual`、单仓 six-step/四命令/GitFinalize。生产接口保持不变，不为测试新造 direct fallback。

- [ ] **Step 1：写失败回归。** `lcg_t13_single_repository_sync_claude_codex_direct_topology_unchanged`（Claude/Codex 两槽 args/cwd/output，Pi/Kimi reject）、`lcg_t13_workspace_streaming_legacy_four_provider_path_unchanged`（四家 raw start 的 args/cwd/permission/output）、`lcg_t13_sync_direct_never_uses_lc_bridge`、`lcg_t13_ready_plan_does_not_start_other_target_without_manual_command`。扩展 root-safety 断言 normal matrix Unknown 不改变 recipe 隔离；具体断言：

```rust
assert_eq!(sync_direct_args_after, sync_direct_args_before);
assert_eq!(sync_direct_cwd_after, sync_direct_cwd_before);
assert_eq!(sync_direct_output_after, sync_direct_output_before);
assert_eq!(sync_direct_permission_after, sync_direct_permission_before);
assert_eq!(pi_kimi_sync_result, Err(DirectProviderUnsupported));
assert_eq!(workspace_streaming_args_after, workspace_streaming_args_before);
assert_eq!(workspace_streaming_output_after, workspace_streaming_output_before);
assert_eq!(lc_bridge_calls_for_direct, 0);
assert_eq!(other_target_attempt_count_after_confirmed_plan, before_count);
assert_eq!(provider_start_count_before_explicit_start_coding, 0);
assert_eq!(repository_initialization_steps[5], "git_finalize");
```

- [ ] **Step 2：Main运行红测试。** `cargo test --locked --test it_web lcg_t13_single_repository_sync_claude_codex_direct_topology_unchanged -- --nocapture`、`cargo test --locked --test it_web lcg_t13_workspace_streaming_legacy_four_provider_path_unchanged -- --nocapture`、`cargo test --locked --lib lcg_t13_sync_direct_never_uses_lc_bridge`。新增 LC 邻接身份/参数对照若先 PASS 如实记 baseline 绿，不制造假红或改生产行为使其红；本工作包是兼容锁验收，不是新增 direct 功能。
- [ ] **Step 3：完善回归 fixture。** 使用既有单仓 builder/legacy 与初始化脚本事件，观察实际调用/cwd/输出/步骤；Fake 仅确定性测试替身，四家真实 workspace streaming 对照不标 LC 支持，四家同步 direct 不在本 change 交付。保留 `shared_executor_cannot_reach_repository_registration_or_git_finalize`、`manual_start_coding_keeps_sc_ready_gate`、`start_coding_before_advance_ready_is_rejected_with_sc_coding_requires_advance` 原行为；不得从 ready Plan 新增自动多 target 派工。若发现实现回归，退回拥有该实现文件的 Task 修复并范围复审，13不另改边界。
- [ ] **Step 4：Main运行绿回归。** `cargo test --locked --test it_web provider_gateway_envelope -- --nocapture`、`cargo test --locked --test it_web web_repository_initialization -- --nocapture`、`cargo test --locked --lib legacy_repository_generate_still_invokes_adapter_directly`、`cargo test --locked --lib logical_repository_generate_fails_closed_with_gateway_required`、`cargo test --locked --lib logical_repository_generate_revision_fails_closed_with_gateway_required`、`cargo test --locked --lib manual_start_coding_keeps_sc_ready_gate`、`cargo test --locked --lib shared_executor_cannot_reach_repository_registration_or_git_finalize`；sync direct、workspace streaming direct 与 LC validated gateway 三类路径仍分离。
- [ ] **Step 5：提交。** `git add tests/it_web/provider_gateway_envelope.rs src/product/work_item_split_engine/tests/engine_gateway_guard.rs src/web/coding_start_parts/tests.inc.rs src/web/coding_ws_handler/tests/sc_start_guard.rs src/web/handlers/aggregate_initialization/tests_parts/root_safety.inc.rs tests/it_web/web_repository_initialization/operation_http.rs && git commit -m "test: 锁定单仓 direct 与 workspace streaming 兼容"`；13a/13b 按本段实际 Files 分别提交。


## Task 14：存量迁移等待面与运维证据（tasks.md 5.2；REQ-LCG-01/03/04）

**Files：** 14a 2–4h：Modify `src/product/logical_codebase/provider_capability_store.rs`（存量读取/evidence失效测试）、`src/product/logical_codebase/provider_admission_preflight_tests.inc.rs`（waiting材料/allowed actions）、`src/web/handlers/automation_gateway_preflight.rs`（诊断投影测试，仅8完成后串行）；14b 1–2h：Evidence `cadence/reports/2026-10-03_验收报告_LC网关多provider能力迁移与运维_v1.0.md`、`cadence/reports/lc-gateway-multi-provider/capability-migration/`。Read-only当前14工作包/REQ/spec及HEAD预算，不改OpenSpec tasks状态（由Main验收后勾选）。报告同时承载运维说明与永久行为变更记录，不另建重复changelog。

**Interfaces：** 具体输入输出、切片前置与汇合仅以 §0 为准。Task14 消费旧 matrix Unknown、root-recipe 隔离、waiting、Codex 限制与真实 evidence，产出当前精确 `provider/version/OS/action/entrypoint/fresh-or-resume/state/reason/evidence_ref` 支持矩阵与操作顺序：补前置、真实 probe、校验证据、产品 Revalidate/Retry、完整 fresh admission。不得提供手工JSON改Confirmed操作或新“迁移默认allow”。

- [ ] **Step 1：写失败测试。** `lcg_t14_legacy_matrix_unknown_projects_actionable_waiting`、`lcg_t14_cli_upgrade_keeps_history_and_blocks_until_reprobe`、`lcg_t14_unverified_os_version_or_cost_is_never_default_supported`。断言：

```rust
assert_eq!(legacy_launch_state, ProviderCapabilityEvidence::Unknown);
assert!(waiting.missing_materials.contains(&missing_evidence_ref));
assert!(waiting.allowed_actions.contains(&BootstrapActionKind::Revalidate));
assert!(waiting.allowed_actions.contains(&BootstrapActionKind::Retry));
assert_eq!(session_spawn_count, 0);
assert!(old_probe_artifact_still_exists);
assert_ne!(untested_version_state, ProviderCapabilityEvidence::Confirmed);
```

- [ ] **Step 2：Main运行红测试。** `cargo test --locked --lib lcg_t14`；预期旧waiting仅`provider_capability_not_satisfied`且未完整列版本/projection/OS evidence材料，新增可操作断言红；已有Unknown拒绝绿测试不冒充新红。
- [ ] **Step 3：补消费诊断与运维记录。** 冻结stable reasons：`provider_unsupported_for_gateway_launch`、`provider_capability_action_unknown`、`provider_resume_unsupported`、`provider_trust_missing`、`provider_write_boundary_unverified`、`provider_adapter_dialect_mismatch`、`codex_danger_full_access_unsupported`、`codex_target_boundary_unverified`及policy/body漂移码；缺字段作为missing materials，不扩大allowed actions。报告注明当前实测版本/OS仅逐格支持；Pi最低0.83.0/Kimi最低0.34.0是旧产品底线；未实测更新版本/Windows/macOS launcher/大LC新成本为Unknown。保留20k/64MiB snapshot和4096/8192B inventory机械预算既有证据，报告新实测耗时/token，不把发现矩阵外推五阶段安全。写清Codex共享home撤销digest敏感边界、Kimi native/Aria MCP分层、#8状态与无fixture门。
- [ ] **Step 4：Main范围验收与收口。** `cargo test --locked --lib lcg_t14`、`openspec validate lc-gateway-multi-provider`；逐行对照14工作包、全部spec场景、非目标与每格证据，blocked格不能改为supported。全部代码合入后Main只执行一轮项目命令：`cargo fmt --check`、`cargo clippy --all-targets --all-features --locked -- -D warnings`、`cargo check --locked`、`cargo test --locked`。真实CLI40主格及子入口另由10–12实际命令验收，不被全量单测替代；通过独立review后才sync/archive/分支收口。
- [ ] **Step 5：提交与交接。** `git add src/product/logical_codebase/provider_capability_store.rs src/product/logical_codebase/provider_admission_preflight_tests.inc.rs src/web/handlers/automation_gateway_preflight.rs && git commit -m "test: 锁定 LC capability 迁移等待与版本失效"`；运维报告不暂存，无产品源码变化时14b不commit。Main根据真实验收证据勾tasks，不因Plan完稿而提前归档。


## 计划自审记录

自审日期2026-10-03。本节是计划接地/一致性自审，不是实现、测试或真实CLI通过声明。

1. **Spec coverage：** `tasks.md` 的14个工作包逐一映射 Task1–14；Spec 七条 requirements/全部 scenario 覆盖：映射/Unknown/fresh-vs-resume→1/2/14；admission/漂移/cwd/双栈→1/3/8；projection/logical roots/boundary→4/5/6/11；Codex 三场景→5/8/11；resume 四家/Unknown/fingerprint→9/10/12；guard/多违规/false 标志→7/8/13；真实支持/越界缺证据→10/11/12。单仓、recipe、手工 StartCoding 非目标以13锁定；#8外部依赖明确等待，没有范围缺口用 fixture 遮盖。
2. **Step scan：** 每 Task 均有 Files、Interfaces、且恰有 Step1–5；每个失败步骤有实名测试和断言，验证步骤使用具体过滤名，集成命令符合 `--test it_web <filter> -- --nocapture`，并有 commit。Task1/10 含 `single_candidate.rs`，Task1/7 含 `provider_stream/launch.rs` 与 `provider_stream.rs`，Task2 含2c shape/2d durable 两段；Task6 Step4 仅复用 Task10/11 证据，不形成后续 Task 的代码依赖；Task10–12 明确 `LC_GATEWAY_E2E=1`、`--ignored`、四家命名、证据目录、40主格、双入口和 #8 BLOCKED 语义。无源码修改、无 Cargo/E2E 执行声明。
3. **Type consistency：** provenance 保留原 `CapabilityEvidence`，三态唯一复用 `ProviderCapabilityEvidence`；record 字段保留 `version/adapter_dialect/evidence` 避免拼写漂移；`ProviderActionCapability`、`ProviderPolicyProjector`、`ProviderPolicyProjection`/`ProviderProjectionView`、`CodexSandboxProjection`、`ProviderBoundaryPlan`、`ProviderTrustSource`、`WorkItemSplitProviderRunHandle`、`LcProviderStartAudit` 均有唯一产出 owner。`ProviderBoundary*` 纯 DTO 归1a，6a仅实现行为；fake registry 按 §0.4 保留 non-LC legacy，正常 LC 生产注册按原子四参合同迁移。
4. **Review Focus：** 五风险均落实名新测试：版本升级→`lcg_t02_cli_version_drift_invalidates_row`/`lcg_t05_version_or_protocol_cwd_drift_invalidates_projection`/`lcg_t14_cli_upgrade_keeps_history_and_blocks_until_reprobe`；trust/capability竞态→`lcg_t03_validate_then_each_frozen_dimension_drifts_zero_spawn`；单仓误入 LC 门→`lcg_t08_single_repository_skips_lc_predicates`/`lcg_t13_single_repository_sync_claude_codex_direct_topology_unchanged`/`lcg_t13_workspace_streaming_legacy_four_provider_path_unchanged`；projection digest/物理边界→`lcg_t04_projection_digest_changes_on_target_role_tool_or_config`、`lcg_t04_kimi_host_fs_and_terminal_share_target_boundary`、Task6/11 boundary matrix；存量 resume 指纹→`lcg_t09_legacy_audit_missing_projection_cannot_resume_lc`/`lcg_t09_start_new_waits_for_explicit_start_generation`/Task10/12。
5. **Proportion：** 计划只钉路径、签名、契约固定值、测试断言、§0依赖顺序和现场证据形态，没有生产函数体；切片时长是并行 worker 单片上限，关键路径采用 proposal 的12–18人日，外部 CLI/OS/home/trust 等待不计产能。七条 spec、14包、权威设计与报告底座均有落点；算法只钉 digest/挂载/root-target/同步 runtime 中不可随意选择的边界。
6. **全文一致性扫描记录（round4）：** 已扫描 §0.1 E1–E30、所有 Task Files/Interfaces/Step、共享文件说明、round1–3 对照表和自审文字。依赖与 owner 的现行表述统一为“见 §0”；唯一任务边清单为 §0.1，唯一共享文件门为 §0.3，fake registry 边界为 §0.4。Task1 不再反向消费 Task2/3/4/5/6，Task6 Step4 不再依赖 Task10/11；旧历史边描述均标为作废，未发现与 §0 冲突的现行依赖文字。

> **历史追溯声明：** 本表仅保留 round1 发现与落点追溯；其中任何依赖、owner 顺序或“消费/前置”描述均已作废，以 §0 为准。
## 修订对照表（fix-round-1，23 条）

| # | finding | 具体落点（Task / Files / Interfaces / 测试） |
|---:|---|---|
| 1 | P0-1：真实 WS Plan/split streaming 入口漏列 | Task1b/10；`run/provider_run.rs`、`work_item_plan_legacy_author.inc.rs`、`followups.rs`、`run.rs`、`gateway_start.rs`；`workspace_streaming_plan/split` 与 `start_work_item_plan_author`；`lcg_t10_*`。 |
| 2 | P0-2：probe 证据无法导入 capability | Task2c/6c；`ProviderCapabilityProbeService::record_verified_probe(project_id,record,evidence,projection)` 与 `ProviderBoundaryProbe::run`；**历史旧边 `2a→4/5/6a→6c→2c` 已作废，现行依赖见 §0**；`lcg_t02_cli_version_drift_invalidates_row`。 |
| 3 | P1-1：0 命令误用文件名过滤 | Task1/13/9；使用具体 `logical_repository_*`、`legacy_repository_*`、`split_sync_gateway_launch_rebinds_cwd_to_canonical_root` 等过滤名；不再把 `engine_gateway_guard` 当 cargo filter。 |
| 4 | P1-2：Validated 构造与 caller 迁移不全 | Task1b；`session_launch.rs` 构造入口收口，列出 workspace/coding/review 全部生产 caller；`lcg_t01_all_lc_entrypoints_require_projection`。 |
| 5 | P1-3：audit 只覆盖有 tool_policy 路径 | Task1/4/7；`ProviderLaunchAuditContext`、`provider_stream.rs` helper 取消 `tool_policy=None` 早退；Kimi/Coder 仍 version/native/audit；`lcg_t04_no_generic_tool_policy_still_records_version_audit_and_native_session`。 |
| 6 | P1-4：`engine_gateway_guard` fingerprint caller 漏迁 | Task9a；`src/product/work_item_split_engine/tests/engine_gateway_guard.rs` 与 gateway 两处 caller 同原子迁移；`lcg_t09_split_sync_resume_unknown_is_not_streaming_fresh`。 |
| 7 | P1-5：估时机械相加超契约 | Global Constraints/并行表/自审；采用 proposal 12–18人日关键路径，明确并行片上限与外部等待不计产能。 |
| 8 | P2：root receipt 文件责任不精确 | Task3；逐列 `root_recipe_receipt.rs`、types/store/auditor/tests 五文件，且只读消费。 |
| 9 | P2：provider trust 文件责任不精确 | Task3；逐列 `provider_trust.rs`、`provider_trust_adapters.rs`、`provider_trust_store.rs`，新增只读 `ProviderTrustSource`，不改 ensure/revoke producer。 |
| 10 | A1：factory normal admission 有写副作用 | Task3/8；`build_readonly_for_lc` 与 `build_for_lc_material_prep`，迁移 factory caller 的读/物料准备责任；`lcg_t03_early_eligibility_needs_no_future_d4_and_writes_nothing`。 |
| 11 | A2：gateway trust source 未冻结 | Task3；gateway 持有 `Arc<dyn ProviderTrustSource>`，GET/early 只读 home adapter，spawn revalidate 同源；`lcg_t03_validate_then_each_frozen_dimension_drifts_zero_spawn`。 |
| 12 | A3：projection 只有私有字段，provider 无法安全消费 | Task1/4/5；`ProviderProjectionView` 或 `pub(crate)` getter，validated launch boundary/projection accessor；provider 禁止自造 projection；`lcg_t04_projection_digest_changes_on_target_role_tool_or_config`。 |
| 13 | A4：streaming prepare 缺 audit context | Task1a/1b；`prepare_streaming_launch(input,request,context)` 与 sync 对齐，所有 LC run 先绑定 sink；`lcg_t01_split_start_audit_precedes_completed_record`。 |
| 14 | A5：StartNew 后 revision driver 立即 fresh | Task9c；`drive.rs`、`lifecycle.rs`、WS inbound/protocol/run 文件；`StartNew` 仅 supersede/waiting，复用 `StartGeneration`；`lcg_t09_start_new_waits_for_explicit_start_generation`。 |
| 15 | A6：direct 口径混写 | Task1/12/13；单仓仅 Claude/Codex sync 两槽、Pi/Kimi reject；四家 workspace streaming legacy 单独对照；`lcg_t13_single_repository_sync_claude_codex_direct_topology_unchanged`。 |
| 16 | P23：direct acceptance 仍写成四家同步交付 | Task12/13 Interfaces/Step4 明确“不交付四家同步 direct”，两类 baseline 各自记录；`lcg_t12_single_repository_sync_direct_two_slot_comparison`、`lcg_t12_workspace_streaming_legacy_four_provider_comparison`。 |
| 17 | A7：#8 consumer helper 缺 receipt 参数 | 全局接口/Task3；`verify_published_policy_body(authority_root,artifact,receipt)` 校验 artifact/receipt/body 三方 digest、root、revision、policy_id 与独立 rule_digest；`lcg_t12_body_revision_receipt_drift_blocks_before_spawn`。 |
| 18 | A8：boundary plan 未接到 adapter | Task1/4/5/6；validated launch→`ProviderBoundaryPlan`→`start_validated`/ProcessManager/native，同一 projection/evidence_ref；Task6 boundary negative matrix。 |
| 19 | 第19条：Kimi host fs/terminal/sandbox 文件漏列 | Task4c/6a；精确 `client_services/{terminal,sandbox,fs_service,terminal_handlers,fs_handlers}.rs`，root ro/target rw/symlink escape；`lcg_t04_kimi_host_fs_and_terminal_share_target_boundary`。 |
| 20 | A9：成员规则被 AggregateBootstrap 一并豁免 | Task3；`missing_root_rules` 与 `missing_member_rules` 分拆，成员缺失任意 phase waiting+zero spawn；`lcg_t03_member_language_rule_missing_is_not_root_phase_exempt`。 |
| 21 | A22：phase 豁免范围未钉死 | Global/Task2/3；仅 root rules 可 phase 豁免，policy/capability/authority/target/trust/projection/availability/D4 永不豁免；`bootstrap_phase_only_waives_missing_root_rules`。 |
| 22 | A10：WS provider-run/run_ref 生命周期不闭合 | Task1/10；`WorkItemSplitProviderRunHandle` begin/complete/fail，followup/revision/retry 均新 run_ref，parse 消费旧 handle；`lcg_t01_ws_plan_split_run_handle_closes_on_success_and_failure`。 |
| 23 | 第20/21条：registry 原子 projector 生命周期与无通用 tool-policy audit | Task1/4/7；adapter/projector/gate 同 key 原子注册，Kimi/Coder 仍 exact version/native/audit；`lcg_t01_all_lc_entrypoints_require_projection`、`lcg_t04_no_generic_tool_policy_still_records_version_audit_and_native_session`。 |
> **历史追溯声明：** 本表仅保留 round2 发现与落点追溯；其中任何依赖、owner 顺序或“消费/前置”描述均已作废，以 §0 为准。

## 修订对照表（fix-round-2，7 条）

| # | finding | 具体落点（Task / Files / Interfaces / 测试） |
|---:|---|---|
| R2-1 | P0：真实 WS Plan/split caller 仍漏 `single_candidate.rs` | Task1b/10 Files 增加 `src/web/workspace_ws_handler/run/single_candidate.rs`；明确 `drive_single_candidate_reredrive:318+`、`run_single_candidate_author:456+` 均走 `start_work_item_plan_author`，纳入每次 begin handle、sink、complete/fail 和 `workspace_streaming_plan/split` evidence；依赖与汇合见 §0。 |
| R2-2 | P0：2c 与4/5/6c evidence 依赖成环 | Task2 拆为2c shape validator、2d durable writer；2c 不写 durable，6c 产生真实 evidence 后2d 才写 Confirmed；现行依赖与汇合见 §0。 |
| R2-3 | P0：P0-2 验收只有版本漂移，缺 durable Confirmed 闭环 | Task2 Step1 增加 `lcg_t02_probe_evidence_projection_record_imports_confirmed_atomically` 与 mismatch case；断言 evidence exact version/profile/projection/boundary ref、projection digest、record 字段三方一致，`launch/write_boundary=Confirmed`，不一致保留旧 Unknown/字节、零 spawn。 |
| R2-4 | P1：真实 gateway 分流文件漏列且仍可裸 start | Task1b/7 Files 增加 `src/product/coding_workspace_engine/provider_stream/launch.rs`；冻结 `launch_provider_session` 仅 `(validated,gateway)` 走 gateway、LC 缺任一项稳定拒绝、非LC无gateway才裸 start；Task7 `lcg_t07_validated_launch_never_invokes_raw_start_or_run` 覆盖计数隔离。 |
| R2-5 | P1：`tool_policy=None` 早退 owner 文件漏列 | Task7 Files 增加 `src/product/coding_workspace_engine/provider_stream.rs`，明确 `attach_tool_policy_audit:110–157` 是唯一早退修订 owner；`lcg_t07_lc_tool_policy_none_still_allocates_audit_sink` 断言 Coder/Kimi 统一 audit。 |
| R2-6 | P1：Kimi `client_services/mod.rs`、`sandbox.rs` 与6a共享 owner冲突 | Task6a 只先行 crate-visible helper/只读 mount-plan 接口并独占合入；Task4c 在6a之后独占同文件行为改动；并行表 B/D 与 Task4/6 Files 均标明先后，禁止并改。 |
| R2-7 | P1：registry 原子契约验收与旧签名迁移不具体 | Task1a Step1 增加半注册拒绝、同 gate、projector 生命周期测试；冻结 `register_gated(name,adapter,projector,gate)->Result`、`register_test_pair`，并列旧 `register_gated(name,provider,gate)` caller `state.rs::real_provider_registry:470` 的迁移；半注册无可见 entry。 |

本轮 round-2 只修订计划文本；未修改源码、OpenSpec 契约或报告，未提交；未执行 Cargo、真实 CLI、E2E、fmt、clippy、check 或全量 test。

## 历史依赖审查记录（round-3，已作废）

> 本段仅记录 round3 曾审查过的闭环问题，不再作为依赖或 owner 的事实源；其中所有边、拓扑摘要和共享 owner 描述均作废，以 §0 为准。round3 的问题与修订结果见下方对照表。

round3 复审确认过的主题包括：DTO owner 前移、2c shape 与 6c probe 的职责拆分、2d durable Confirmed 导入、Task3 admission 消费边、Task1 D→1b 入口接入、Task1b→1c 装配、Task10 入口路径以及 registry 迁移边界。当前可执行拓扑、共享文件全路径和 fake registry 归属统一见 §0。

## 修订对照表（fix-round-3，6 条）

| # | finding | 具体落点（Task / Files / Interfaces / 测试） |
|---:|---|---|
| R3-1 | P0：2c/6c/3 与 E 表存在依赖环，且2c反向消费Task6类型 | 1a前移纯 Evidence/Projection DTO；2c仅做shape validator，6a/6c仅实现boundary/probe，2d后置写 durable，3消费2d；修订后的唯一依赖见 §0。 |
| R3-2 | P1：Task10 Files 路径 typo | Task10 精确改为 `src/web/workspace_ws_handler/run.rs`，不再出现 `run/run.rs`。 |
| R3-3 | P1：validated 缺项稳定拒绝测试归属不钉死 | Task7 owner `src/product/coding_workspace_engine/provider_stream/launch.rs`；`lcg_t07_validated_launch_never_invokes_raw_start_or_run` 断言 LC 缺 validated/gateway 稳定错误且 raw start=0；同 Task `provider_stream.rs:110–157` 的 `lcg_t07_lc_tool_policy_none_still_allocates_audit_sink`。 |
| R3-4 | P1：registry 129 个 register 调用迁移边界不明 | 正常 LC 只迁生产 `state.rs::real_provider_registry:470` 到四参原子 register；129 个既有 `register(name,adapter)` 测试/非LC fixture 与 fake_mode_provider_registry 按 §0.4 保留。 |
| R3-5 | P1：mod.rs/gateway_factory/coordinator 文件共享 owner 未闭合 | 全路径共享 owner 统一见 §0.3；前置 owner 不得碰后置逻辑。 |
| R3-6 | P1：Task6 与2d/Task3 的接口依赖文字不一致 | Task6 只实现 probe/launcher 行为，2d 写 durable，3 消费 durable；现行依赖见 §0。 |

round-3 自审：历史审查已确认 DTO owner、shape/probe/durable 职责、Task10 入口与 registry 迁移边界；该审查结论现全部由 §0 重写承载。未执行产品检查，未改源码/OpenSpec/报告，未提交。

## 修订记录（fix-round-4）

- 新增 §0「依赖与共享文件权威表」，将依赖边、Task1切片输入、全路径共享文件 owner、registry 边界和拓扑摘要集中为唯一事实源。
- 将 `src/cross_cutting/provider_boundary.rs` 的纯 DTO owner 固定为 Task1a；Task6a 改为 Modify 并只实现 launcher/helper 行为，消除 DTO owner 冲突。
- 将 `src/product/logical_codebase/mod.rs` 与 `src/cross_cutting/mod.rs` 拆成两条全路径 owner 门；补齐 `gateway_factory.rs`、`coordinator_provider_turn.inc.rs`、split_sync 与 envelope 测试文件的 owner 门。
- Task1 Interfaces 按 1a/1b/1c 拆开，1b 明确只消费 1a DTO+2c shape，不消费 2d/3；Task6 Step4 改为复用 Task10/11 证据，不形成后续 E2E 代码依赖。
- `fake_mode_provider_registry` 固定为 non-LC legacy 运行时封装；普通 `register` 五次调用与 129 个测试/非 LC fixture 调用不迁移生产语义，正常 LC 只迁生产四参原子注册。
- round1–3 对照表顶部及 round3 历史依赖审查均标明作废；新增本轮全文一致性扫描、round4 对照与逐边闭包验算。

本轮仅修改本计划文本；不修改源码、OpenSpec 契约或报告，不 commit；未执行 Cargo、真实 CLI、E2E、fmt、clippy、check 或全量 test。

## 修订对照表（fix-round-4，8 条）

| # | finding | round4 收敛结果 |
|---:|---|---|
| R4-1 | 依赖描述分散在 E 表、Task Interfaces、Step、共享门和 round3 清单，存在环与漂移 | 新增 §0.1 E1–E30；其它现行位置统一写“见 §0”，历史边全部作废。 |
| R4-2 | `src/product/logical_codebase/mod.rs` 与 `src/cross_cutting/mod.rs` 被混写为一条 owner 链且漏2b | 按 §0.3 拆成两条全路径 owner 门，logical_codebase 与 cross_cutting 不再混写。 |
| R4-3 | DTO owner 在1a与6a之间冲突 | `provider_boundary.rs` 归1a Create，6a Modify；冻结纯 DTO 与行为实现边界。 |
| R4-4 | Task1 Interfaces 反向消费 Task2/3/4/5/6 | 按 §0.2 分拆1a/1b/1c输入；1b不消费2d/3。 |
| R4-5 | Task6 Step4 把后续 E2E 写成实现依赖 | 改为“证据复用”Task10/11 命令、fixture、证据，不形成代码依赖。 |
| R4-6 | fake_mode_provider_registry 的普通 register 边界未钉死 | §0.4 选择合法 non-LC legacy 保留；LC 测试 fixture 使用 `register_test_pair`，129 个既有调用不迁生产。 |
| R4-7 | round1 旧边与 round2/3 历史拓扑可能被误读为现行事实 | 各历史表顶部加入作废声明，round1 第2条就地标注旧边作废，round3 清单降级为历史审查记录。 |
| R4-8 | 缺少可复核的全文扫描和逐边无环过程 | 自审第6项记录扫描结果；下节按 E1–E30 分组验算并给出闭包结论。 |

## 拓扑闭包自检（round4）

逐边按 §0.1 分组验算：

1. **E1–E5：** 1a→2a→2c，2c再分别流入4、5、6a；不存在从2c回写1a/2a的边。
2. **E6–E9：** 4/5/6a单向汇入6c，6c单向汇入2d；2d不回消费6c之外的后续节点。
3. **E10–E13：** 2d单向到3，3单向到7/8/9；不存在从7/8/9回到3的边。
4. **E14–E18：** 7/8/9分别单向汇入10/11/12/13/14；后续任务不反向提供前置依赖。
5. **E19–E28：** 8/9与7分别汇入10/11/12/13/14，保持后置证据只消费前置合同。
6. **E29–E30：** D单向汇入1b，1b单向汇入1c；并行入口不回写主链节点。

> **自审结论：** §0.1 E1–E30 逐边验算通过；历史 round1–3 仅作追溯，任何旧边以 §0 为准。
结论：E1–E30 的每条边均从前置产出指向后置消费；唯一任务拓扑为 `1a→2a→2c→{4/5/6a}→6c→2d→3→{7/8/9}→{10/11/12/13/14}`，并行入口为 `D→1b→1c`，不存在回边。共享文件 owner 仅按 §0.3 处理，依赖图闭包、无环且与全文现行表述一致。



