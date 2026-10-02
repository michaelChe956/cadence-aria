# LC 网关多 Provider 接入 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: 使用 `subagent-driven-development`（推荐）或 `executing-plans` 逐任务实施。步骤使用 checkbox（`- [ ]`）跟踪。本计划只展开已批准的 OpenSpec change；执行前由 Main 并行安排接地审查与一致性裁决。计划编写阶段不改代码、不运行产品检查、不 commit。

**Goal：** 为 LC 的 Story、Design、Plan/split、Coding、Review 交付 Claude Code、Codex、Pi、Kimi Code 四家真实 provider 的统一 gateway、逐 action 三态能力、权限投影、原生 resume 和全链验收证据；缺材料时在 provider 启动前失败关闭，单仓 direct 与 Claude-only 根 recipe 保持原契约。

**Architecture：** 保留 `LogicalCodebaseProviderGateway` 作为 LC 唯一 launch authority，在现有 opaque validated policy 上绑定 provider projection 和运行审计上下文；同步入口使用只接受 validated input 的 streaming-to-sync bridge，四家共用真实 adapter，不另写 Pi/Kimi 裸 CLI adapter。正常 LC 会话消费新 capability matrix；凭据约束的根 recipe 保持独立 evidence 判定。projection 与 native/产品拥有的写边界、trust、cwd/target、政策正文和 D4 在 validate→spawn 间复验，#8 的政策发布由并行 change 提供，本 change 只读消费。

**Tech Stack：** Rust 2024、Tokio、serde、sha2、现有 JSON/file-lock store、Axum、真实 Claude stream-json / Codex app-server JSON-RPC / Pi RPC / Kimi ACP；复用现有 bubblewrap 和 Kimi client-service 路径，无新增 Cargo 外部依赖。命令遵循 `cadence/project-rules/build-test-commands.md`。

**Spec：** `openspec/changes/lc-gateway-multi-provider/proposal.md`、`design.md`、`tasks.md`、`specs/lc-gateway-multi-provider/spec.md`；权威设计 `cadence/designs/2026-10-01_方案设计_LC网关多provider接入_v1.0.md`。#8 外部依赖为 `openspec/changes/aggregate-policy-root-publication/` 的 proposal 与两份 spec；该 change 并行推进，尚不能把其交付写成已完成。

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
- Rust快反馈：`cargo test --locked --lib <过滤名>`；集成：`cargo test --locked --test it_web <过滤名> -- --nocapture`；不得显式`-j 1`。每切片先提交新增失败断言供Main跑红并回传，worker据真实红点实现，合入后Main跑该片绿；检查与同批共享编辑互斥。全量checks只在所有切片落地后统一一轮，不由多个worker并跑。
- 每个执行切片独占精确文件、独立提交、独立审查，单次派工不超过 4h；表中为规模上限估计，若重探测/实现超出切片边界，Main 先拆分再派工。报告证据留盘但不 `git add`；代码提交只暂存本切片明确文件。

## Review Focus

1. **CLI 升级/版本未知：** 同 provider 的 exact version、gateway/wire dialect 或 tool token 变化后，旧证据不继续 allow；正常矩阵转 Unknown/Denied，零 session spawn。Task 2/5/14 的版本漂移测试及真实 probe 钉住此行为；Pi 0.83.0、Kimi 0.34.0 现有最低门只作底线，不等于所有更高版本已验证。
2. **trust 与 capability 竞态：** validate 后任一 trust digest/action row/availability/D4/projection 改变，在最后 revalidate 时拒绝且进程启动计数为 0；已经构造的 envelope 不能绕过。Task 3/9 参数化逐维漂移测试。
3. **单仓误入 LC 门：** SingleRepository 即使配置 Pi/Kimi、`gateway_required=false` 或 legacy sync/stream，也保持原 cwd/argv/permission/output/GitFinalize；正常 LC 同标志仍被检查。Task 8/13 对照测试。
4. **projection digest 与真实写面：** digest 漂移、只拒内建工具而 terminal/MCP 可写、沙箱不存在或挂载失效时，不能凭空 writable roots / 无漂移快照升级。Task 4/5/6/11 覆盖。元数据负向探针保护 root/non-target 的 `.git`/`.aria` 及 target 的 `.git` 链接指针；target 内受控 git commit 是正向证据，git-dir 权限沿已交付 identity/授权链，不新增整个 `.git` 写授权（Main 已裁决）。
5. **存量 resume 指纹：** 老记录缺 projection/evidence/native session 确认，以及 policy/authority/cwd/target/git identity/trust/tool/MCP bundle 任一漂移，必须 supersede 并等待用户显式新会话；不得在 adapter 内清 resume id 后启动 fresh。Task 9/10/12 分别钉住决策、原生恢复和真实失败格；单仓既有 drift→fresh 语义保留。

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
- **Task 1a定义/2a持久化（矩阵）：** 保留 `CapabilityEvidence::{Declared,FixtureVerified,ProductionVerified}` 名字，仅作provenance；三态唯一复用 `ProviderCapabilityEvidence::{Confirmed,Denied { reason:String },Unknown}`。`ProviderActionCapability { action:SessionPolicyAction,launch:ProviderCapabilityEvidence,resume:ProviderCapabilityEvidence,write_boundary:ProviderCapabilityEvidence,projection_digest:String,evidence_ref:String }`在1a先定义，2a落durable。record保留`version/adapter_dialect/evidence`，新增schema_version=2/wire_dialect/action_matrix/trust/probed_at/probe_artifact_ref/root_recipe_evidence；legacy字段仅DTO解码，normal不保留旧API。
- **Task 2b（phase-aware）：** `require_supported(&self,provider:&ProviderRef,action:SessionPolicyAction)->Result<ProviderCapability,ProviderGatewayError>`保留名，正常action row；新增同参返回型`require_resume_supported/require_write_boundary`。`require_root_recipe_supported(&self,provider:&ProviderRef,credential:&BootstrapPhaseCredential)->Result<ProviderCapability,ProviderGatewayError>`仅现有固定Claude recipe事实，不推normal Confirmed。新增gateway内部`validate_root_recipe_request(&self,request:SessionLaunchRequest,credential:&BootstrapPhaseCredential)->Result<ValidatedSessionLaunchPolicy,ProviderGatewayError>`；opaque launch私有冻结phase，普通调用不可构造。credential每次重验durable Running；root-recipe不误套normal target-only/read-only action门，仍逐项检policy/capability/root/target/trust/availability及原receipt auditor。
- **Task 1a/4/5（projection）：** 新增 `ProviderProjectionInput`私有字段，由gateway构造envelope、provider/action row、role、permission/tool/MCP/config/trust/boundary输入。`ProviderPolicyProjector::project(&self,input:&ProviderProjectionInput)->Result<ProviderPolicyProjection,ProviderProjectionError>`；四家在4/5实现。projection私有只读字段固定`provider_type/provider_dialect/wire_dialect/exact_version/action/role/permission_mode/tool_policy/approval_policy/sandbox/working_directory/protocol_working_directory/target/readable_roots/writable_roots/trust_digest/config_digest/mcp_bundle_digest/boundary_evidence_ref/capability_projection_digest/projection_digest`。digest固定字段序+长度分隔+schema前缀，不能Debug文本/调用方摘要；只有gateway将候选projection与capability evidence校验后包装成launch权，不给调用方公开任意构造器。
- **Task 1a/1b（装配）：** `ProviderRegistry::register_projector(name:ProviderName, projector:Arc<dyn ProviderPolicyProjector>)` 与 `projector(&ProviderName)->Option<Arc<dyn ProviderPolicyProjector>>` 为新增，真实 projector 只有四家，Fake仅测试隔离注册。新增 `StreamingProviderAdapter::start_validated(ValidatedStreamingProviderInput,CancellationToken)->Result<ProviderSession,ProviderAdapterError>` 与 `ProviderAdapter::run_validated(ValidatedAdapterInput)->Result<AdapterOutput,ProviderAdapterError>`；默认返回unsupported，不能回调裸start/run。Gated装饰器透传并复验可用性。raw `start/run` 单仓接口保持原值。
- **Task 1a定义/1b实现（准备/同步）：** 新增 gateway `prepare_streaming_launch(&self,input:StreamingProviderInput,request:SessionLaunchRequest)->Result<ValidatedStreamingProviderInput,ProviderGatewayError>`、`prepare_sync_launch(&self,input:AdapterInput,request:SessionLaunchRequest,context:ProviderLaunchAuditContext)->Result<ValidatedAdapterInput,ProviderGatewayError>`；`ProviderLaunchAuditContext { workspace_session_id:String, role_run_seq:u64, audit_sink:Arc<dyn ToolPolicyAuditSink> }`在1a定义。prepare之前所有LC角色先绑定审计sink，二者按provider/role选择语义policy、核对input/request，再生成和绑定projection；coding虽无通用deny也需要真实audit。新增根recipe专用 `prepare_root_recipe_launch(&self,input:StreamingProviderInput,request:SessionLaunchRequest,credential:BootstrapPhaseCredential)->Result<ValidatedStreamingProviderInput,ProviderGatewayError>`，只由durable driver调用，private policy冻结phase和recipe evidence，spawn时复验；普通prepare不能接收自报phase。gateway `start_streaming/run_sync`仍接validated类型。新增 `GatewaySyncProviderAdapter`只用registry的`start_validated`+真实events+现有sentinel parser生成`AdapterOutput`，不经legacy bridge/task-run。
- **根recipe启动保持（Task1c/2b）：** 正常session的`ValidatedSessionLaunchPolicy`必须有projection、matrix与audit，真实adapter只走`start_validated/run_validated`；唯一private RootRecipe phase仍经gateway opaque权、durable credential复验后走原Claude recipe启动/marker/receipt链，不能因普通角色矩阵要求空写根把root recipe改只读，也不能用RootRecipe的事实给用户Coding授权。此路径是原recipe合同的保留，不是normal LC legacy fallback；必须以Task2/7/12专用回归验证。
- **Task 1a定义/5实现（Codex）：** `CodexSandboxMode::{ReadOnly,WorkspaceWrite}`与`CodexSandboxProjection { mode:CodexSandboxMode,approval_policy:String,process_cwd:PathBuf,protocol_cwd:PathBuf,target_root:PathBuf,boundary_evidence_ref:String }`在1a先定义，5实现mapping。固定wire read-only/workspace-write，只读approval=on-request、Coding Auto=never/Supervised=on-request；无DangerFullAccess枚举。`codex_launch_params`保留direct分支，LC只读projection；不改全局direct默认。
- **Task 6（boundary）：** 新增 `ProviderBoundaryPlan`、`ProviderBoundaryEvidence`、`ProviderBoundaryError`；`ProcessManager::spawn_with_boundary(command:&str,args:&[&str],working_dir:&Path,env_vars:&BTreeMap<String,String>,plan:&ProviderBoundaryPlan,cancel:CancellationToken)->Result<ManagedProcess,ProviderAdapterError>`。native经真实probe可满足时使用native；否则产品拥有的Linux bwrap launcher冻结mount/metadata/provider runtime目录，保持cwd及已有配置发现，覆盖provider后代/MCP/extension；Kimi宿主client services另外用同一target边界。无支持的OS/namespace或不可控写通道为Unknown/Denied，不退到无隔离spawn。
- **Task 3（#8消费）：** 复用 `AggregatePolicyArtifactStore::{for_lc,get}`、`AggregatePolicyArtifact::{policy_id,revision,digest,policy_text}`、`RootRecipeReceipt::{policy_digest,rule_digest,canonical_root}`；新增消费helper `verify_published_policy_body(authority_root:&Path, artifact:&AggregatePolicyArtifact)->Result<(),ProviderGatewayError>`（只读）。要求 safe relative policy_id、canonical无symlink逃逸、原字节等于policy_text、SHA-256等于artifact/receipt，revision一致；不造发布API/sidecar。正常 admission用已发布最终政策，RootRecipe相位沿#8已定义临时自举政策合同，不签正常session材料。
- **Task 9（resume）：** `SessionResumeFingerprint::from_envelope`追加projection/action evidence和`lc-resume-v2`域；`GatewaySessionDisposition::StartNew`仍只决定supersede，不spawn。新增可选 `ProviderStartAudit.lc_projection:Option<LcProviderStartAudit>`，精确字段 `policy_id:String,policy_revision:u64,policy_digest:String,authority_root:PathBuf,working_directory:PathBuf,target:PolicyTarget,target_git_identity:String,capability_snapshot_ref:String,action_evidence_digest:String,capability_projection_digest:String,projection_digest:String,trust_digest:Option<String>,mcp_bundle_digest:Option<String>`；exact version/dialect已有顶层字段，wire由adapter_dialect及projection绑定；新字段serde缺省None只保留旧direct可读，LC None拒resume。native不确认同id必须kill失败，不清id fresh。
- **证据摘要分层（Task1a/2/6）：** action row的`projection_digest`是已实测version/action的完整权限profile摘要（该action允许的role×permission整张固定映射表、tool/approval/sandbox/native或OS方案/MCP控制规范），不能随同一action中单次选role变化。session projection另冻结`capability_projection_digest`及包含当前role/root/target/authority/trust/config的全`projection_digest`。先计算profile并比对应action证据，再绑定evidence_ref、算session摘要；probe证据另记当次真实session digest。不能要求新target=session旧fixture摘要，也不能让evidence_ref/digest递归哈希自身；resume同时比对profile/evidence及全session摘要。

**实现选项：** 推荐LC专用validated sync bridge，成本是一个事件消费器和专用runtime线程，收益是四家复用同一projection/guard/native协议且direct零改动；分别写四个sync CLI adapter会复制权限和恢复逻辑，并改既有task-run routing，因此不选。推荐优先复用native边界、Linux不足时扩展既有bwrap机制；仅依赖D4成本低但不能满足REQ-LCG-03/07，不作为可交付方案。

## 工作包映射与并行分组

| OpenSpec工作包 | Plan Task | REQ | 可独立派工切片（每片≤4h） |
|---|---|---|---|
| 1.1 | 1 | LCG-01/02 | 1a 映射/trait/opaque承载；1b validated双栈和全部LC入口；1c 生产装配与sync运行身份 |
| 1.2 | 2 | LCG-01 | 2a DTO/roundtrip/legacy；2b形状与全部caller原子迁移→phase判定；2c版本probe/evidence验签 |
| 1.3 | 3 | LCG-02/03 | 3a policy/canonical/target；3b trust/capability/D4/availability复验 |
| 2.1 | 4 | LCG-03/06 | 4a Claude；4b Pi；4c Kimi（互不共享文件） |
| 2.2 | 5 | LCG-04 | 5a Codex projection/wire；5b受限门+approval/boundary测试 |
| 2.3 | 6 | LCG-03/07 | 6a Linux launcher/后代；6b D4 freshness；6c四家真实probe/evidence（Kimi宿主target门归4c） |
| 3.1 | 7 | LCG-06 | 7a common guard与三家顺序回归；7b Kimi拒绝与recipe回归 |
| 3.2 | 8 | LCG-06 | 8a同源role-chain/DTO；8b GET/Enable/rebind接线 |
| 3.3 | 9 | LCG-05 | 9a fingerprint/explicit决策；9b Claude/Codex；9c Pi/Kimi原生确认 |
| 4.1 | 10 | LCG-07 | 10a真实harness；10b–10e四家各自五阶段fresh/resume现场 |
| 4.2 | 11 | LCG-07 | 11a失败零spawn harness；11b–11e各provider探针/快照现场 |
| 4.3 | 12 | LCG-02/07 | 12a #8交付门/recipe；12b四家direct与最终证据汇合 |
| 5.1 | 13 | LCG-02 | 13a direct/输出；13b GitFinalize/人工StartCoding红线 |
| 5.2 | 14 | LCG-01/03/04 | 14a migration/waiting测试；14b运维/最终矩阵对照 |

| 批次 | 可并行切片 | 独占源码边界（任务Files中的测试随同归属） | 真依赖与汇合 |
|---|---|---|---|
| A 合同先行 | 1a | gateway/policy/projection/session_launch/registry/trait/装饰器、capability provider映射、planning dialect映射与全部穷举fixture；mod接线同worker | 1a产出所有跨Task类型，先补现存穷举match；default拒绝只表示未接入，不算功能支持 |
| B 存储与launcher | 2a；6a | 2a capability_store/provider_capabilities；6a process_manager及新cross_cutting/provider_boundary | 依赖1a；不改gateway/四家adapter，互不共享 |
| C capability source | 2b、2c串行 | production_policy_resolvers/gateway能力段/已受影响fixture/coordinator接线 | 2a后；共享gateway/mod文件必须串行，不与3/9/1b同时编辑 |
| D 四家projection | 4a、4b、4c、5a/5b；6b | 各自provider目录独占；6b仅cross_target_check/provider_stream与coding D4测试 | 1a、2c、6a后；4c独占Kimi client-services，6b不碰Kimi；四家projector文件各自独占 |
| E 准入/双栈 | 3a→3b→1b→1c→7→6c | gateway/admission/factory/streaming与入口共享文件按此顺序串行；6c仅probe聚合模块+mod接线 | 依赖四家projection+launcher；1c装配后才执行四家真实probe、写候选能力证据；正常门不为probe放宽。6c新增mod接线与1a/2c串行 |
| F role-chain/resume/兼容 | 8a→8b ∥ 9a→9b→9c ∥ 13 | 8独占automation；9独占gateway恢复/audit/各provider恢复；13独占direct测试/StartCoding回归 | E后；13的provider_gateway_envelope.rs须等9a（或9不触该文件）与7结束后再编辑，单纯只读审查可并行 |
| G harness | 10a→11a→12a | 新 `tests/it_web/web_lc_gateway_multi_provider/` 各分文件；module/it_web.rs只10a改 | F后；共享harness串行完成，再跑provider现场 |
| H 四家真实现场 | 10b+11b；10c+11c；10d+11d；10e+11e | 同provider同fixture内串行；不同provider独占各自evidence子目录 | #8真实正文交付是成功E2E前置；缺件负向case可先跑。共享用户home/trust时现场串行或锁住，不能宣称无冲突并行 |
| I 收口 | 12b；14a→14b | 12写policy_dependency/direct_comparison模块与E2E汇总；14独占capability waiting/迁移测试与运维报告 | H现场+13后；共享harness mod由12a→12b串行；Main一次项目全量验证和终审，再sync/archive |

**全局共享文件门：** gateway仅1a→2b→3→1b→9；streaming/mod仅1a→7；公共projection只1a（4/5provider自有projection）；production resolver仅2b→3；web/state只1c；it_web.rs只10a。两处mod接线按1a→2c→6a/6c→1b由Main串行，表中无并改。#8拥有发布/recipe实现，本change仅读；1a只在其policy.rs添加dialect，1c只在coordinator_provider_turn.inc.rs改可信prepare调用，Main与#8文件owner排开相同文件编辑，不改发布算法。

## Task 1：四家映射、validated双栈与真实入口（tasks.md 1.1；REQ-LCG-01/02）

**Files（按切片独占）：**

- **1a，3–4h：** Modify `src/product/logical_codebase/provider_gateway.rs`、`src/product/logical_codebase/policy.rs`（仅dialect）、`src/product/logical_codebase/mod.rs`、`src/product/logical_codebase/planning_context_resolver.rs`（dialect→ProviderType）、`src/product/logical_codebase/provider_capability_store.rs`（仅provider_type序列化四分支）、`src/cross_cutting/session_launch.rs`、`src/cross_cutting/provider_registry.rs`、`src/cross_cutting/provider_adapter.rs`、`src/cross_cutting/provider_availability_gate.rs`、`src/cross_cutting/streaming_provider/mod.rs`；Create `src/product/logical_codebase/provider_projection.rs`。Test gateway与所有当前穷举match fixture：`src/product/logical_codebase/coordinator_tests.inc.rs`、`src/product/logical_codebase/planning_context_resolver_tests.inc.rs`、`src/product/logical_codebase/provider_admission_preflight_tests.inc.rs`、`src/product/coding_workspace_engine/tests/group_review_identity_snapshot.rs`、`src/product/coding_workspace_engine/tests/provider_gateway_validated_input.rs`、`src/product/work_item_split_engine/tests/engine_gateway_guard.rs`、`src/product/workspace_engine/tests/part_32.rs`、`tests/it_web/provider_gateway_envelope.rs`；只补显式枚举分支，2b后续迁matrix。拒绝默认实现不称支持。
- **1b，三个2–4h串行段：** Create `src/cross_cutting/gateway_sync_provider.rs`；Modify `src/cross_cutting/mod.rs`、`src/product/logical_codebase/provider_gateway.rs`、`src/product/work_item_split_engine/engine.rs`；workspace段Modify `src/product/workspace_engine/provider_drive.rs`、`src/product/workspace_engine/provider_drive/author_root_launch.inc.rs`、`src/product/workspace_engine/lifecycle/routing_reference.inc.rs`、`src/product/workspace_engine/review/drive.rs`、`src/web/workspace_ws_handler/run/gateway_start.rs`；coding段Modify `src/product/coding_workspace_engine/lifecycle.rs`、`src/product/coding_workspace_engine/provider_retry.rs`、`src/product/coding_workspace_engine/provider_retry_parts/coder_root_launch.inc.rs`、`src/product/coding_workspace_engine/group_review_orchestrator.rs`、`src/product/coding_workspace_engine/internal_pr_review.rs`。顺序bridge+split→workspace→coding。Test `src/product/logical_codebase/provider_gateway_tests/audit.inc.rs`、`src/product/work_item_split_engine/tests/engine_gateway_guard.rs`、`src/web/workspace_ws_handler/tests/gateway_start.rs`、`src/product/coding_workspace_engine/tests/provider_gateway_validated_input.rs`；新bridge内测current-thread调度。
- **1c，3–4h：** Modify `src/web/state.rs`（default_provider_registry装配projector）、`src/web/gateway_factory.rs`（LC专用sync bridge与审计依赖）、`src/product/logical_codebase/coordinator_provider_turn.inc.rs`（凭据phase准备）、`src/product/lifecycle_store/plan.rs`（split运行身份）、`src/product/lifecycle_store/tool_policy_run_audit.rs`（复用run-bound sink）；Test上述文件内测试。Read-only `src/task_run/provider_factory.rs`、`src/cross_cutting/cli_adapter.rs`，单仓两槽routing不迁移。

**Interfaces：** Consumes Task2 source、Task3 admission、Task4/5 projector与`start_validated`、Task6 boundary。Produces上节1a/1b接口；新增 `LifecycleStore::begin_work_item_split_provider_run(project_id:&str,issue_id:&str,provider:&ProviderName)->Result<String,ProductStoreError>` 与 `complete_work_item_split_provider_run(project_id:&str,issue_id:&str,run_ref:&str,prompt:&str,structured_output:&Value)->Result<(),ProductStoreError>`，run_ref是split审计workspace key、单次role_run_seq=1，retry必须新run_ref。旧direct `save_work_item_split_provider_run`仍原样；LC使用begin→bound sink→gateway→complete，不签伪成功run。

- [ ] **Step 1：写失败测试。** 在1a测试写 `lcg_t01_provider_ref_maps_four_real_providers` / `lcg_t01_fake_and_unknown_never_fallback`，断言四家逐项相等、Fake为unsupported、真实start/run计数0；1b写 `lcg_t01_sync_bridge_uses_validated_start_and_preserves_output` / `lcg_t01_bridge_does_not_block_tokio_current_thread` / `lcg_t01_all_lc_entrypoints_require_projection`；1c写 `lcg_t01_split_start_audit_precedes_completed_record`。关键断言：

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
```

- [ ] **Step 2：Main运行红测试。** 每个提交段先执行 `cargo test --locked --lib lcg_t01`，预期新enum/validated分发缺失或raw bridge/输出/调度断言失败；必须列出实际命中的新测试数，0 tests不是红证据。既有direct tests不能删除或改成allow。
- [ ] **Step 3：实现对应签名。** 1a冻结映射、projection opaque承载及新validated trait（默认unsupported只供未实现adapter拒绝）。1b在prepare时核对provider/role/cwd/target/permission/tool与envelope；真实gateway仅调用validated trait。同步bridge实现 `run_validated(&self,launch:ValidatedAdapterInput)->Result<AdapterOutput,ProviderAdapterError>`：专用OS线程拥有Tokio runtime，事件收集复用现有ProviderCompletion与sentinel parser；async调用点用`spawn_blocking`，不嵌套当前runtime.block_on、不让current-thread死锁；超时/取消/Failed/PermissionTimeout/malformed output沿现有错误，不用空JSON/exit0兜底。1c真实四家注册各自projector、Factory LC替换sync adapter，split预分配真实run_ref并绑定sink；recipe传durable凭据，相位不由普通input自报。
- [ ] **Step 4：Main运行绿测试。** `cargo test --locked --lib lcg_t01`、`cargo test --locked --lib engine_gateway_guard`、`cargo test --locked --lib logical_provider_entrypoints_use_gateway_for_sync_and_streaming_stacks`；全部入口有opaque投影/真实audit。prepare前绑定sink：沿现有`LifecycleStore::next_tool_policy_role_run_seq`+RoleRunBoundAuditSink，但LC没有tool policy的Coder/Kimi也必须分配，不能现helper遇None早退；LC Kimi builder按provider生成None通用策略，非法外来Some拒绝，不能在adapter偷偷清掉。direct旧builder字节不改。去掉LCValidated*::new公开组装入口，迁生产caller含fully-qualified provider_drive.rs，不只迁无前缀AST命中。
- [ ] **Step 5：逐段提交。** 仅`git add`各切片Files；1a：`git add src/product/logical_codebase/provider_gateway.rs src/product/logical_codebase/policy.rs src/product/logical_codebase/mod.rs src/product/logical_codebase/planning_context_resolver.rs src/product/logical_codebase/provider_capability_store.rs src/product/logical_codebase/provider_projection.rs src/cross_cutting/session_launch.rs src/cross_cutting/provider_registry.rs src/cross_cutting/provider_adapter.rs src/cross_cutting/provider_availability_gate.rs src/cross_cutting/streaming_provider/mod.rs src/product/logical_codebase/provider_gateway_tests.rs src/product/logical_codebase/coordinator_tests.inc.rs src/product/logical_codebase/planning_context_resolver_tests.inc.rs src/product/logical_codebase/provider_admission_preflight_tests.inc.rs src/product/coding_workspace_engine/tests/group_review_identity_snapshot.rs src/product/coding_workspace_engine/tests/provider_gateway_validated_input.rs src/product/work_item_split_engine/tests/engine_gateway_guard.rs src/product/workspace_engine/tests/part_32.rs tests/it_web/provider_gateway_envelope.rs && git commit -m "feat: 冻结四家 LC validated 启动合同"`；1b三段commit“接通LC同步bridge”“迁移LC规划入口”“迁移LC编码评审入口”，1c“装配真实projector与LC运行审计”，按精确本段Files显式暂存。


## Task 2：Capability三态存储迁移与phase隔离（tasks.md 1.2；REQ-LCG-01）

**Files（按切片独占）：**

- **2a，3–4h：** Modify `src/product/logical_codebase/provider_capability_store.rs`（DTO/get/upsert/bootstrap及内嵌测试）、`src/cross_cutting/provider_capabilities.rs`（复用三态与四家dialect常量）；Test同文件。
- **2b，两个≤4h串行原子段：** shape/source签名与全部受影响fixture同一commit，Modify `src/product/logical_codebase/production_policy_resolvers.rs`、`src/product/logical_codebase/provider_gateway.rs`、`src/product/logical_codebase/mod.rs`及`src/product/logical_codebase/coordinator_tests.inc.rs`、`src/product/logical_codebase/planning_context_resolver_tests.inc.rs`、`src/product/logical_codebase/provider_admission_preflight_tests.inc.rs`、`src/product/logical_codebase/provider_gateway_tests.rs`、`src/product/coding_workspace_engine/tests/group_review_identity_snapshot.rs`、`src/product/coding_workspace_engine/tests/provider_gateway_validated_input.rs`、`src/product/work_item_split_engine/tests/engine_gateway_guard.rs`、`src/product/workspace_engine/tests/part_32.rs`、`src/web/workspace_ws_handler/tests/single_candidate_lc_admission.rs`、`tests/it_web/provider_gateway_envelope.rs`。第二段只改同三实现文件/相关tests的phase判定与错误，不先提交半编译类型让下片兜编译。
- **2c，3–4h：** Create `src/product/logical_codebase/provider_capability_probe.rs`（real probe/evidence导入校验）；Modify `src/product/logical_codebase/mod.rs`、`src/web/gateway_factory.rs`（注入实际version/boundary/evidence source）；Test新模块内测试。不能只改bootstrap写四条allow记录。

**Interfaces：** Consumes1a四家映射，既有`ProviderCapabilityEvidence`、store scope与JSON原子写。Producesmatrix/source三方法；gateway `ProviderCapability`用`provider_type/version/adapter_dialect/wire_dialect/capability_snapshot_ref/action_capability/trust`。`RootRecipeCapabilityEvidence`在record有独立`root_recipe_evidence:Option<RootRecipeCapabilityEvidence>`，固定Claude、旧recipe version/snapshot/provenance/launch事实，不赋予normal resume/boundary；只有可信credential可消费，缺legacy recipe事实不默认Confirmed。新增 `ProviderCapabilityProbeService::record_verified_probe(project_id:&str,record:&ProviderCapabilityRecord)->Result<(),ProductStoreError>`校验当前exact version、权限profile与真实evidence，不以record自报bool/provenance推断。2c实现导入校验，6c/10提供真实producer，避免依赖循环。

- [ ] **Step 1：写失败测试。** `lcg_t02_matrix_round_trips_three_states`、`lcg_t02_legacy_actions_read_unknown`、`lcg_t02_fresh_and_resume_use_separate_cells`、`lcg_t02_cli_version_drift_invalidates_row`、`lcg_t02_unknown_normal_matrix_keeps_existing_root_recipe_contract`。用包含三种状态且Denied reason=`boundary probe denied`的DTO，断言：

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

- [ ] **Step 2：Main运行红测试。** `cargo test --locked --lib lcg_t02`；预期无matrix字段/三态Denied丢reason、legacy自动allow或recipe受新normal门误伤而FAIL。记录每个新测试真实红点，不把旧绿单测冒充新TDD。
- [ ] **Step 3：实现对应签名。** 2a读取缺schema/matrix为v1，旧字段仅decode，三action Unknown且Denied reason完整，未知schema/状态拒绝。2b类型形状和所有fakes原子迁移，删正常ResumeEvidenceState二态；源验证exact version/gateway+wire/snapshot/action/profile/证据，fresh launch+boundary与resume分开。root-recipe证据只冻结legacy已交付Claude事实（新根由原bootstrap流程产生专用事实），不是把旧supported_actions推成normal；`.check(AggregateBootstrap)`调用`validate_root_recipe_request`，normal仍新source。2c候选证据验签/写入，不自行把CLI启动/MCP/provenance作Confirmed；旧版本证据保留、当前normal行失效等待真实probe。
- [ ] **Step 4：Main运行绿测试。** `cargo test --locked --lib lcg_t02`、`cargo test --locked --lib provider_capability_store`、`cargo test --locked --lib store_backed_capability`；新版bootstrap测试应断言normal全Unknown（替换旧自动allow断言），recipe专用测试继续保持原fixture支持；Denied理由经JSON/API往返不丢失。
- [ ] **Step 5：提交。** 2a：`git add src/product/logical_codebase/provider_capability_store.rs src/cross_cutting/provider_capabilities.rs && git commit -m "feat: 持久化 LC action capability 三态矩阵"`。2b第一段source类型+全部fixture同一commit“迁移LC capability矩阵与全部caller”，第二段同精确Files的phase判定commit“隔离正常会话与根recipe能力消费”。2c：`git add src/product/logical_codebase/provider_capability_probe.rs src/product/logical_codebase/mod.rs src/web/gateway_factory.rs && git commit -m "feat: 校验真实探针并写入 LC capability"`；永久行为/等待说明随Task14运维报告，不另建changelog。


## Task 3：Canonical政策、trust与spawn前复验（tasks.md 1.3；REQ-LCG-02/03）

**Files：** 3a/3b各2–4h，串行共享：Modify `src/product/logical_codebase/provider_gateway.rs`（validate/revalidate/统一无副作用verdict）、`src/product/logical_codebase/provider_admission_check.inc.rs`、`src/product/logical_codebase/provider_admission_preflight.rs`、`src/product/logical_codebase/production_policy_resolvers.rs`、`src/web/gateway_factory.rs`；Test `src/product/logical_codebase/provider_admission_preflight_tests.inc.rs`、`src/product/logical_codebase/provider_gateway_tests/task28_root_authority.inc.rs`、`src/product/logical_codebase/provider_gateway_tests/audit.inc.rs`。Read-only #8拥有的 `policy.rs`政策发布区、`root_recipe_receipt*.rs`、`bootstrap_projector.inc.rs`、`provider_trust*.rs`，不改发布/重冻结/登记/撤销逻辑。

**Interfaces：** Consumes1/2 opaque projection/phase/source，#8artifact/receipt；trust用`ProviderTrustHomeAdapter::{read_state,digest}`只读，`ProviderTrustRegistry::verify_trusted(project_id,lc_id,provider,root)`有audit副作用不能用于GET。新增`admission_verdict(&self,request:&SessionLaunchRequest,projection:&ProviderPolicyProjection,is_resume:bool)->Result<(),ProviderGatewayError>`完整复验；early `action_admission_verdict(&self,provider:&ProviderRef,action:SessionPolicyAction,role:&AdapterRole,permission_mode:ProviderPermissionMode)->Result<ProviderActionAdmission,ProviderAdmissionError>`，本Task定义 `ProviderActionAdmission { capability_snapshot_ref:String,projection_ref:String }`，不返回携实际target的旧PreflightResult、不伪造worktree/D4。新增 `LogicalCodebaseGatewayFactory::build_readonly_for_lc(project_id:&str,lc_id:Option<&str>)->Result<LogicalCodebaseProviderGateway,ProviderGatewayError>`无ensure/bootstrap写；原build仅用于允许材料准备的产品命令。Main确认early资格与spawn事实分层，真正spawn完整门没有豁免。

- [ ] **Step 1：写失败测试。** `lcg_t03_root_policy_body_missing_waits_without_materialization`、`lcg_t03_policy_locator_rejects_symlink_and_digest_drift`、`lcg_t03_validate_then_each_frozen_dimension_drifts_zero_spawn`、`lcg_t03_root_cwd_and_target_are_independent`、`lcg_t03_early_eligibility_needs_no_future_d4_and_writes_nothing`；逐维policy/authority/cwd/target/git/trust/action row/projection/config/MCP/availability/D4漂移；另断early合法证据在attempt未创建时可判资格且policy/capability/trust audit前后字节不变，实际spawn缺D4拒绝。

```rust
assert!(matches!(&waiting, ProviderAdmissionError::Waiting { reason_code, .. } if reason_code == "provider_policy_artifact_missing"));
assert_eq!(root_files_before, root_files_after);
assert!(matches!(error, ProviderGatewayError::PolicyDrift { .. } | ProviderGatewayError::ProviderUnavailable(_)));
assert_eq!(adapter_start_count, 0);
assert_eq!(adapter_run_count, 0);
assert_eq!(envelope.working_directory, canonical_root);
assert_eq!(envelope.writable_roots, vec![canonical_target]);
```

- [ ] **Step 2：Main运行红测试。** `cargo test --locked --lib lcg_t03`，预期当前只读内部artifact/成员language规则及有限复验不能覆盖新增root locator/trust/boundary漂移；红点必须来自新增行为，已有cwd分离绿测试保留。
- [ ] **Step 3：实现精确helper/verdict。** `verify_published_policy_body`只读canonical root安全locator，核对raw UTF-8与artifact/receipt；root规则参考AGENTS与既有rule digest，不新要求成员复制language规则。阶段顺序为`identity/manifest → policy body/artifact/receipt → provider mapping/version/action capability → trust → role/tool → logical roots → projection/adapter → availability → boundary/D4`。prepare冻结合法projection，spawn前重新读取以上事实；cwd canonical必须等于manifest root，不能仅prefix允许子目录；target git identity独立复验，resume追加action resume。trust校验只读，不能在spawn偷偷ensure/revoke。root recipe凭据只沿#8临时政策合同与自身evidence，不豁免policy/capability/gateway/cwd/target/trust。
- [ ] **Step 4：Main运行绿测试。** `cargo test --locked --lib lcg_t03`、`cargo test --locked --lib logical_root_or_target_fingerprint_drift_is_rejected`、`cargo test --locked --lib bootstrap_phase_only_waives_missing_root_rules`；全部PASS，失败无provider_start成功记录且零child。正常缺#8返回可操作Retry/Revalidate waiting，recipe零回归。
- [ ] **Step 5：提交。** 3a/3b仅暂存上列精确Files，commit“校验canonical根政策正文与target身份”“在spawn前复验LC全部冻结事实”；如根recipephase与正常session差异出现，按Main已确认边界修正而不关闭能力/authority门。`git add src/product/logical_codebase/provider_gateway.rs src/product/logical_codebase/provider_admission_check.inc.rs src/product/logical_codebase/provider_admission_preflight.rs src/product/logical_codebase/production_policy_resolvers.rs src/web/gateway_factory.rs src/product/logical_codebase/provider_admission_preflight_tests.inc.rs src/product/logical_codebase/provider_gateway_tests/task28_root_authority.inc.rs src/product/logical_codebase/provider_gateway_tests/audit.inc.rs && git commit -m "feat: 在 spawn 前复验 LC 冻结事实"`。


## Task 4：Claude/Pi/Kimi真实权限projection（tasks.md 2.1；REQ-LCG-03/06）

**Files（4a/4b/4c各2–4h，Kimi目录必要时按provider进程→client两段串行）：**

- **4a独占：** Create `src/cross_cutting/claude_code_provider/projection.rs`；Modify `src/cross_cutting/claude_code_provider/mod.rs`（build_args/start_validated）、`src/cross_cutting/claude_code_provider/tests/mod.rs`（LC audit test fixture）；Test `src/cross_cutting/claude_code_provider/tests/policy_session.rs`、`src/cross_cutting/claude_code_provider/tests/args.rs`。
- **4b独占：** Create `src/cross_cutting/pi_provider/projection.rs`；Modify `src/cross_cutting/pi_provider/mod.rs`（build_args/start_validated/extension准备）、`src/cross_cutting/pi_provider/tests/mod.rs`（LC audit fixture）；Test `src/cross_cutting/pi_provider/tests/policy_session.rs`、`src/cross_cutting/pi_provider/tests/session_flow.rs`。
- **4c独占：** Create `src/cross_cutting/kimi_code_provider/projection.rs`；Modify `src/cross_cutting/kimi_code_provider/mod.rs`、`src/cross_cutting/kimi_code_provider/session.rs`、`src/cross_cutting/kimi_code_provider/client_services/mod.rs`、`src/cross_cutting/kimi_code_provider/client_services/fs_handlers.rs`、`src/cross_cutting/kimi_code_provider/client_services/terminal_handlers.rs`；Test `src/cross_cutting/kimi_code_provider/tests/tool_policy_zero_change.rs`、`src/cross_cutting/kimi_code_provider/tests/session_tests.rs`、`src/cross_cutting/kimi_code_provider/mcp_bundle_tests.rs`。`client_services/policy.rs`只读，其矩阵不变；新增LC目标写门是独立路径边界，不能改角色决策表。

**Interfaces：** Consumes1a `ProviderPolicyProjector`、2 action row、6 boundary与1 launch audit。Produces `ClaudeCodePolicyProjector`、`PiPolicyProjector`、`KimiPolicyProjector`（同一trait签名）及各自`start_validated`。Claude/Pi使用projection的physical片段，Kimi通用tool为None，ClientServicePolicy仍由现有role/permission派生；Kimi宿主read root=canonical root、write root=target分离，terminal实际cwd不改变provider进程cwd。`mcp_bundle_digest`只对应Aria注入，native项目配置单独标来源。公共projection.rs不由并行provider worker编辑。

- [ ] **Step 1：写失败测试。** 4a `lcg_t04_claude_projection_has_headless_mcp_allowlist_and_deny_tokens`；4b `lcg_t04_pi_projection_has_exclude_write_tokens_and_root_cwd`；4c `lcg_t04_kimi_projection_separates_client_read_root_from_target_write_root` / `lcg_t04_kimi_native_mcp_is_not_aria_bundle`；各家增 `lcg_t04_projection_digest_changes_on_target_role_tool_or_config`。断言：

```rust
assert!(claude_args.windows(2).any(|p| p == ["--disallowedTools", "Edit,Write,NotebookEdit"]));
assert!(claude_args.iter().any(|arg| arg == "--allowedTools"));
assert!(pi_args.windows(2).any(|p| p == ["--exclude-tools", "edit,write"]));
assert_eq!(provider_cwd, canonical_root);
assert!(target_write_allowed);
assert!(!root_or_other_member_write_allowed);
assert!(kimi_generic_tool_policy.is_none());
assert_ne!(original_projection_digest, changed_projection_digest);
```

- [ ] **Step 2：Main逐切片运行红测试。** `cargo test --locked --lib lcg_t04_claude`、`cargo test --locked --lib lcg_t04_pi`、`cargo test --locked --lib lcg_t04_kimi`；预期无LC projector或client root与target未分离失败。已有物理deny token测试本来绿，不能仅重命名为新红。
- [ ] **Step 3：实现trait和validated分支。** 四家projection只从gateway输入生成；真实version未知/adapter不匹配拒绝。Claude headless MCP allowlist只列既有合法工具，不扩大到MCP写工具；Pi extension准备在guard之后，并在launcher覆盖面内，其RPC/MCP继续既有adapter；Kimi `start_validated`在version/child前拒非空通用策略，ACP cwd保持root，read/write/terminal宿主handler额外消费不可伪造target边界（含git identity）。三家请求构造与最终argv/wire/audit同源，不把logical roots翻译成不存在的CLI参数。direct `start`/Kimi原ClientServicePolicy表与native发现保持原值。
- [ ] **Step 4：Main运行绿测试。** 三个provider过滤与`cargo test --locked --lib lcg_t04_projection_digest`、`cargo test --locked --lib kimi_four_role_client_service_table_stays_unchanged`；首个provider_start关联version/wire/profile与session digest。resume严格字段由9补；本Task对新LC launch audit使用现有sink记录全部角色真实start，不以tool_policy=None跳过。
- [ ] **Step 5：提交。** 每家仅暂存其Files，commit分别“投影Claude的LC权限与MCP”“投影Pi的LC角色与RPC权限”“绑定Kimi的LC独立目标写边界”；例如4b：`git add src/cross_cutting/pi_provider/projection.rs src/cross_cutting/pi_provider/mod.rs src/cross_cutting/pi_provider/tests/policy_session.rs src/cross_cutting/pi_provider/tests/session_flow.rs && git commit -m "feat: 投影 Pi 的 LC 角色与 RPC 权限"`。


## Task 5：Codex受限sandbox与wire（tasks.md 2.2；REQ-LCG-04）

**Files：** 5a/5b各3–4h串行；Create `src/cross_cutting/codex_provider/projection.rs`；Modify `src/cross_cutting/codex_provider/mod.rs`、`src/cross_cutting/codex_provider/session.rs`、`src/cross_cutting/codex_provider/tests/mod.rs`（validated audit/native session测试fixture）；Test `src/cross_cutting/codex_provider/tests/approval_policy.rs`、`src/cross_cutting/codex_provider/tests/bridging.rs`、`src/cross_cutting/codex_provider/tests/streaming.rs`。公共gateway/automation硬门归3/8，不由5并改。

**Interfaces：** Consumes1a `CodexSandboxProjection`/opaque input、2矩阵、6 boundary。Produces `CodexPolicyProjector::project(...)` 与LC `codex_launch_params` 分支；`CodexSandboxMode`只在projection中限制，不修改direct默认danger-full-access。Coding`process_cwd=root`、`protocol_cwd=target`且root discovery/trust/native或OS写边界均被当前版本证实。

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

**Files（分片独占）：**

- **6a，launcher核心/挂载保护/后代probe三个3–4h段串行：** Create `src/cross_cutting/provider_boundary.rs`；Modify `src/cross_cutting/process_manager.rs`、`src/cross_cutting/mod.rs`；Test新模块与`src/cross_cutting/process_manager/drop_tests.rs`。Read-only Kimi sandbox/git路径freeze；需要共享现有`frozen_writable_git_paths`时只拓`src/cross_cutting/kimi_code_provider/client_services/mod.rs`与`sandbox.rs`的crate可见性（方法和行为不变），6a先合入再4c，不能复制第二套git identity算法。
- **6b，2–3h：** Modify `src/product/coding_workspace_engine/cross_target_check.rs`（baseline freshness只读复检seam）、`src/product/coding_workspace_engine/provider_stream.rs`（已有capture/detect调用绑定）；Test `src/product/coding_workspace_engine/tests/provider_gateway_validated_input/d4_member_baseline.rs`。Kimi宿主target门归4c，6b不并改Kimi目录。
- **6c，每家≤4h现场切片：** Create `src/product/logical_codebase/provider_boundary_probe.rs`（evidence校验/签发正常action row）；Modify `src/product/logical_codebase/mod.rs`（单owner接线）；Evidence目录 `cadence/reports/lc-gateway-multi-provider/{claude-code,codex,pi,kimi-code}/boundary/`；汇总 `cadence/reports/2026-10-03_验收报告_LC网关多provider写边界_v1.0.md`。源码合入后四家现场各自输出，不并跑Cargo。

**Interfaces：** Consumes1a projection、2 probe writer、4/5 native映射。Produces本Task唯一`ProviderBoundaryPlan`私有字段、`ProviderBoundaryEvidence`、`BoundaryFixture`与 `ProviderBoundaryProbe::run(projection:&ProviderPolicyProjection,fixture:&BoundaryFixture)->Result<ProviderBoundaryEvidence,ProviderBoundaryError>`，记录真实provider/version/OS/profile/session/fixture digest、正负每attempt/error、target写+commit/pre-post/D4。新增 `revalidate_cross_target_baseline(paths:&ProductAppPaths,attempt:&CodingExecutionAttempt,run_id:&str)->Result<(),StableCode>`复用现capture/detect。bootstrap probe在服务拥有的隔离临时fixture内，用projector候选物理映射+同一ProcessManager boundary启动真实CLI/native协议并留事实，不伪造normal validated权、不在用户LC执行Unknown。2c只验该服务输出后入store；10/11的产品E2E随后仍经完整normal gateway，evidence不全立即降状态。

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
- [ ] **Step 4：Main验证并归档。** `cargo test --locked --lib lcg_t06`、`cargo test --locked --lib cross_target`；四家受控fixture真实probe依Task10/11命令跑，记录actual argv/mount/errno/工具channel/版本，Task2c校验后才写capability。现有预算：snapshot `max_entries=20_000`/`max_bytes=64 MiB`、inventory soft4096/hard8192B，沿HEAD不新立预算；FIFO/观测缺失/超限不截断冒充通过。
- [ ] **Step 5：提交。** 6a三段按其精确Files提交“加入LC产品写边界launcher”“保护LC元数据与后代写面”“锁定沙箱失败关闭”，只拓Kimi现有git helper可见性时显式暂存该二文件；6b：`git add src/product/coding_workspace_engine/cross_target_check.rs src/product/coding_workspace_engine/provider_stream.rs src/product/coding_workspace_engine/tests/provider_gateway_validated_input/d4_member_baseline.rs && git commit -m "feat: 复验 LC role-run D4 基线"`；6c：`git add src/product/logical_codebase/provider_boundary_probe.rs src/product/logical_codebase/mod.rs && git commit -m "test: 以真实探针签发 LC 写边界证据"`；报告日志不暂存。


## Task 7：Tool-policy双向guard与Kimi隔离控制面（tasks.md 3.1；REQ-LCG-06）

**Files：** 7a/7b各1–3h，Task4/5合入后串行追加：Modify `src/cross_cutting/streaming_provider/mod.rs`（共用guard/LC run context）、`src/cross_cutting/streaming_provider/tests.rs`；Test `src/cross_cutting/claude_code_provider/tests/policy_session.rs`、`src/cross_cutting/codex_provider/tests/approval_policy.rs`、`src/cross_cutting/pi_provider/tests/policy_session.rs`、`src/cross_cutting/kimi_code_provider/tests/tool_policy_zero_change.rs`、`tests/it_web/provider_gateway_envelope.rs`。只有新回归发现adapter顺序不满足时修改相应`mod.rs`；此Task为guard整体验收owner，不能与4/5同时写provider目录。

**Interfaces：** Consumes既有 `validate_tool_policy_for_role(&AdapterRole,Option<&ProviderToolPolicy>)->Result<(),ToolPolicyGuardError>` 与新增start_validated。Produces LC同一guard顺序回归与 `provider_generic_tool_policy_forbidden` Kimi稳定拒绝码（Kimi实际拒绝由4c实现，此处不重复逻辑）。BootstrapExecutorMarker仍只授权固定Claude root recipe完整凭据/receipt上下文。

- [ ] **Step 1：写失败测试。** `lcg_t07_invalid_role_policy_zero_child_and_zero_extension`覆盖三家×Orchestrator/Splitter/Reviewer缺deny、Executor/Handoff带deny；`lcg_t07_kimi_generic_policy_rejected_before_version_child`；`lcg_t07_root_recipe_marker_is_not_normal_coder_policy`。用真实process/extension seam计数而非grep源码，断言：

```rust
assert!(invalid_role_policy_result.is_err());
assert_eq!(session_child_count, 0);
assert_eq!(rpc_handshake_count, 0);
assert_eq!(extension_start_count, 0);
assert_eq!(kimi_reason, "provider_generic_tool_policy_forbidden");
assert!(valid_recipe_marker_result.is_ok());
assert!(same_marker_on_ordinary_coder.is_err());
```

- [ ] **Step 2：Main运行红测试。** `cargo test --locked --lib lcg_t07`；三家原guard本来绿，新validated身份/extension顺序/Kimi拒绝新增断言为红；不可通过删Kimi或Bootstrap案例缩窄矩阵。
- [ ] **Step 3：对齐guard调用。** LC `start_validated`验证projection role与input role后调用共用guard，早于版本session child/extension；所有错误直接返回，不删policy再尝试。Kimi只拒非空通用策略，原24格ClientServicePolicy不改，宿主写边界另由4c实现。valid bootstrap marker校验durable operation与receipt context，普通Coder不借marker获取root写权。
- [ ] **Step 4：Main运行绿测试。** `cargo test --locked --lib lcg_t07`、`cargo test --locked --lib adapter_tool_policy_guard_is_bidirectional_for_every_role`、`cargo test --locked --lib kimi_four_role_client_service_table_stays_unchanged`、`cargo test --locked --lib bootstrap_executor_requires_credential_and_receipt_context`；预期PASS且guard拒绝早于全部session执行面。最后一项现存在lib的streaming_provider/tests.rs，不误用it_web过滤0 tests。
- [ ] **Step 5：提交。** `git add src/cross_cutting/streaming_provider/mod.rs src/cross_cutting/streaming_provider/tests.rs src/cross_cutting/claude_code_provider/tests/policy_session.rs src/cross_cutting/codex_provider/tests/approval_policy.rs src/cross_cutting/pi_provider/tests/policy_session.rs src/cross_cutting/kimi_code_provider/tests/tool_policy_zero_change.rs tests/it_web/provider_gateway_envelope.rs && git commit -m "test: 锁定 LC 角色工具守卫与根 recipe 隔离"`；发生实现顺序修正时仅补列本段对应provider/mod.rs。


## Task 8：Automation同源role-chain全量预检（tasks.md 3.2；REQ-LCG-06）

**Files：** 8a/8b各2–4h；Modify `src/web/handlers/automation_gateway_preflight.rs`、`src/web/handlers/automation_target.rs`（GET）、`src/web/handlers/automation_enrollment.rs`（Enable/rebind）；Test各文件内现存tests；Read-only `src/web/handlers/support.rs`的carrier resolver（沿用唯一入口）、Task3 gateway/verdict与factory。不得另写一张静态provider allow矩阵。

**Interfaces：** ConsumesTask3 `action_admission_verdict`、carrier/declared target与readonly factory。Produces `AutomationRoleChainViolation { role:String,provider:String,action:SessionPolicyAction,reason_code:String,capability_snapshot_ref:Option<String>,projection_ref:Option<String> }`。两个入口完整新签名为 `validate_role_chain_for_enrollment(gateway:Option<&LogicalCodebaseProviderGateway>,author_provider:&ProviderName,reviewer_provider:&ProviderName,carrier:&AutomationCarrierResolution,gateway_required:bool,test_provider_enabled:bool)->ApiResult<()>` 和 `validate_role_chain_for_declared_enrollment_target(gateway:Option<&LogicalCodebaseProviderGateway>,author_provider:&ProviderName,reviewer_provider:&ProviderName,declared:&EnrollmentTarget,gateway_required:bool,test_provider_enabled:bool)->ApiResult<()>`；SingleRepository不构造gateway，LC缺gateway列全违规。五role固定顺序，分别映射Planning/Coding/Review，422码不变，材料不存在的ref为None不伪造。

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
- [ ] **Step 3：实现同源聚合。** 每role走显式provider→action→Task3 early资格verdict，收集全部可确定capability/trust/projection-profile/adapter/boundary/tool错误；不要求尚不存在的attempt worktree或role-run D4，实际launch继续完整门且无豁免。Codex按LC受限profile而非direct全局默认判定，危险profile仍拒。LC忽略false绕过，SingleRepository先走原跳过，Fake仅test隔离。GET/Enable从同carrier，rebind沿declared target读取现有LC/成员而不猜issue；缺checkout材料列reason，不建worktree/provider。Task3提供readonly factory读取，不运行`ensure_bootstrap`，GET不写policy/capability/trust audit；根事实仍在真实spawn复验。
- [ ] **Step 4：Main运行绿测试。** `cargo test --locked --lib lcg_t08`、`cargo test --locked --lib role_chain_preflight_skips_gateway_predicates_for_single_repository`、`cargo test --locked --lib single_repository_enable_authorizes_physical_target`；预期PASS，旧“Pi/Kimi恒静态unsupported”测试迁为“缺当前action证据时拒、证据完整时通过”，不删除硬拒路径。
- [ ] **Step 5：提交。** `git add src/web/handlers/automation_gateway_preflight.rs src/web/handlers/automation_target.rs src/web/handlers/automation_enrollment.rs && git commit -m "feat: 以同源 LC 门禁聚合 automation 全角色违规"`（8a核心DTO先提交，8b调用者接线后提交；共享文件串行）。


## Task 9：四家原生resume与显式fresh语义（tasks.md 3.3；REQ-LCG-05）

**Files（均在Task4/5/7完成后）：**

- **9a，核心+完整字面量/指纹caller迁移为一段≤4h原子commit，决策测试为下一段≤4h：** Modify `src/product/logical_codebase/provider_gateway.rs`、`src/cross_cutting/tool_policy_audit.rs`、`src/product/lifecycle_store/tool_policy_run_audit.rs`；Test `src/product/logical_codebase/provider_gateway_tests.rs`、`src/product/logical_codebase/provider_gateway_tests/task28_root_authority.inc.rs`、`src/product/lifecycle_store/tests/tool_policy_audit.rs`。新增audit完整字面量一起迁 `src/cross_cutting/claude_code_provider/mod.rs`、`src/cross_cutting/codex_provider/mod.rs`、`src/cross_cutting/pi_provider/mod.rs`、`src/cross_cutting/claude_code_provider/tests/policy_session.rs`、`src/cross_cutting/pi_provider/tests/policy_session.rs`、`src/cross_cutting/codex_provider/tests/approval_policy.rs`；不先提交缺字段半编译版本。9a完成后才9b/9c。
- **9b，Claude/Codex各2–4h可并行：** Modify `src/cross_cutting/claude_code_provider/mod.rs`、`src/cross_cutting/codex_provider/mod.rs`、`src/cross_cutting/codex_provider/session.rs`；Test `src/cross_cutting/claude_code_provider/tests/policy_session.rs`、`src/cross_cutting/codex_provider/tests/approval_policy.rs`、`src/cross_cutting/codex_provider/tests/bridging.rs`；没有Codex/tests/policy_session.rs，不能把Claude路径省略后泛化给Codex。
- **9c，Pi/Kimi各2–4h并行，engine段2–4h串行：** Modify `src/cross_cutting/pi_provider/mod.rs`、`src/cross_cutting/pi_provider/session.rs`、`src/cross_cutting/kimi_code_provider/mod.rs`、`src/cross_cutting/kimi_code_provider/session.rs`；Test `src/cross_cutting/pi_provider/tests/policy_session.rs`、`src/cross_cutting/pi_provider/tests/session_flow.rs`、`src/cross_cutting/kimi_code_provider/tests/session_tests.rs`、`src/cross_cutting/kimi_code_provider/mcp_bundle_tests.rs`。engine段Modify `src/product/workspace_engine/review/drive.rs`、`src/product/workspace_engine/provider_drive/author_root_launch.inc.rs`、`src/product/coding_workspace_engine/provider_retry.rs`、`src/web/workspace_ws_handler/run/gateway_start.rs`；Test `src/product/workspace_engine/tests/author_revision_loop_parts/root_cwd_revision.inc.rs`、`src/web/workspace_ws_handler/tests/planning_resume.rs`。

**Interfaces：** Consumes1 prepared projection、2 resume row、3复验。Produces `SessionResumeFingerprint::from_envelope(envelope:&SessionPolicyEnvelope,projection:&ProviderPolicyProjection,action_evidence_digest:&str,target_git_identity:&str)->SessionResumeFingerprint`与上节精确`LcProviderStartAudit`；from_envelope全caller迁移。新增gateway内部 `resume_or_start_with_projection(&self,request:ResumeSessionLaunchRequest,projection:ProviderPolicyProjection)->Result<GatewaySessionDisposition,ProviderGatewayError>`，9调用者先完整prepare得到候选projection再比指纹；旧`resume_or_start`公共名可委托该完整上下文判定，不能用仅envelope指纹生成可spawn权限。`StartNew`仅supersede并等待显式fresh；缺旧LC v2字段拒resume，direct三元组保留。

- [ ] **Step 1：写失败测试。** `lcg_t09_matching_full_fingerprint_resumes_native_session`、`lcg_t09_each_fingerprint_dimension_supersedes_without_fresh_spawn`、`lcg_t09_legacy_audit_missing_projection_cannot_resume_lc`、`lcg_t09_explicit_resume_unknown_zero_spawn`、`lcg_t09_claude_missing_or_wrong_native_id_never_fresh`、`lcg_t09_codex_missing_or_wrong_native_id_never_fresh`、`lcg_t09_pi_missing_or_wrong_native_id_never_fresh`、`lcg_t09_kimi_missing_or_wrong_native_id_never_fresh`、`lcg_t09_kimi_bundle_drift_never_session_new`。断言：

```rust
assert!(matches!(matching, GatewaySessionDisposition::Resume(_)));
assert!(matches!(drifted, GatewaySessionDisposition::StartNew { .. }));
assert_eq!(decision_spawn_count, 0);
assert_eq!(fresh_after_explicit_resume_count, 0);
assert!(old_run_superseded_recorded);
assert_eq!(confirmed_native_id, requested_native_id);
assert!(native_id_missing_result.is_err());
assert!(started_child_was_killed_and_reaped);
```

- [ ] **Step 2：Main运行红测试。** `cargo test --locked --lib lcg_t09`；现指纹仅部分维度、三家adapter可清id fresh、Pi get_state/Kimi session响应可回填request id，新增严格LC断言应FAIL。旧direct漂移→fresh测试继续绿。
- [ ] **Step 3：实现签名与原生确认。** fingerprint长度分隔、schema域版本、canonical authority/cwd/target/git/每项digest，旧LC没有v2材料即supersede；fresh不读取旧resume行。三家`start_validated`在child前比较全LC audit，drift/Unknown零child；原生恢复Claude需真实native会话确认，Codex`thread/resume`必须应答同id，Pi`--session-id`+RPC`get_state`必须真实同id且不回填请求，Kimi`session/load`应答与能力协商确认同id，不因bundle漂移走session/new。握手发现不支持/错id属于已启动child后的runtime失败，必须kill/reap且记录“未恢复”，不伪称零spawn；spawn前门失败才要求零spawn。engine对StartNew向用户呈现已有显式新会话操作，不能旧逻辑直接fresh，用户明确fresh后重新prepare/revalidate；single-repository调用旧helper原行为。
- [ ] **Step 4：Main运行绿测试。** `cargo test --locked --lib lcg_t09`、`cargo test --locked --lib planning_resume`、`cargo test --locked --lib resume_rejects_digest_version_or_dialect_drift_and_missing_record`；严格LC与legacy direct均PASS，durable superseded落旧run，sink缺失/写失败沿kill链传播。
- [ ] **Step 5：提交。** 9a类型/全部字面量caller原子提交：`git add src/product/logical_codebase/provider_gateway.rs src/cross_cutting/tool_policy_audit.rs src/product/lifecycle_store/tool_policy_run_audit.rs src/product/logical_codebase/provider_gateway_tests.rs src/product/logical_codebase/provider_gateway_tests/task28_root_authority.inc.rs src/product/lifecycle_store/tests/tool_policy_audit.rs src/cross_cutting/claude_code_provider/mod.rs src/cross_cutting/codex_provider/mod.rs src/cross_cutting/pi_provider/mod.rs src/cross_cutting/claude_code_provider/tests/policy_session.rs src/cross_cutting/pi_provider/tests/policy_session.rs src/cross_cutting/codex_provider/tests/approval_policy.rs && git commit -m "feat: 扩展 LC resume 冻结指纹与审计"`；决策测试/9b/9c各显式本段Files单独commit“锁定LC显式resume决策”“确认Claude原生LC恢复”“确认Codex原生LC恢复”“确认Pi原生LC恢复”“确认Kimi原生LC恢复”；engine段上列四实现/二测试commit“禁止LC显式恢复静默转fresh”。现场证据不暂存。


## Task 10：四家五阶段真实fresh/resume E2E（tasks.md 4.1；REQ-LCG-07）

**Files：** 10a 3–4h：Create `tests/it_web/web_lc_gateway_multi_provider/mod.rs`、`tests/it_web/web_lc_gateway_multi_provider/harness.rs`、`tests/it_web/web_lc_gateway_multi_provider/live_matrix.rs`；Modify `tests/it_web.rs`（唯一module接线owner）。10b–10e每家现场一片≤4h：Evidence `cadence/reports/lc-gateway-multi-provider/<provider>/matrix/`，provider目录固定为`claude-code`、`codex`、`pi`、`kimi-code`；汇总仅Task12写 `cadence/reports/2026-10-03_验收报告_LC网关多provider真实E2E_v1.0.md`。临时non-Git root/main/worktree/store在隔离tempdir，未经审计不覆盖用户文件。

**Interfaces：** Consumes1–9真实生产gateway/adapter与#8产品发布。Produces新增test-only `LiveLcGatewayHarness::run_provider_matrix(provider:ProviderName,evidence_root:&Path)->Result<LiveMatrixEvidence,LiveMatrixFailure>`、`LiveMatrixEvidence`（Task10唯一定义）及`EvidenceCell`键=`provider/exact_version/stage/entrypoint/fresh_or_resume`。复用已读 `tests/it_web/web_lc_operations_api.rs::{request_json,LcOperationsFixture}` 的HTTP形态与真实git fixture建法，但不复用Noop/Fake驱动当真实证据；未存在 `web_lc_root_initialization.rs`，不把旧计划缺失文件当依赖。

- [ ] **Step 1：写失败测试/证据断言。** 新增四个`#[ignore]`真实测试 `lcg_live_claude_five_stages_fresh_resume`、`lcg_live_codex_five_stages_fresh_resume`、`lcg_live_pi_five_stages_fresh_resume`、`lcg_live_kimi_five_stages_fresh_resume`；harness test `lcg_t10_evidence_requires_exact_version_wire_and_native_confirmation`用缺字段记录测试拒绝（此单测只验证证据结构）。真实断言：

```rust
assert_eq!(cell.provider, selected_provider);
assert_eq!(cell.process_cwd, canonical_root);
assert_eq!(cell.target, selected_member_worktree);
assert!(!cell.exact_version.is_empty());
assert_eq!(cell.audit_projection_digest, cell.frozen_projection_digest);
assert_eq!(cell.native_resume_confirmed_id, cell.requested_resume_id);
assert!(cell.argv_or_wire_capture_exists && cell.approval_and_tool_events_exist);
assert!(cell.completed_product_artifact_exists);
```

- [ ] **Step 2：Main运行红证据。** `cargo test --locked --test it_web lcg_t10_evidence_requires_exact_version_wire_and_native_confirmation -- --nocapture`；新harness/字段缺失应FAIL。首次真实执行下列命令，预期现有未完整格按缺前置明确阻断，不手工填Confirmed。`#[ignore]`默认不跑不是E2E通过，执行时必须显式开关且`--ignored`命中新测试；开关缺失要失败而非return成功。
- [ ] **Step 3：实现harness与五阶段顺序。** 每家新LC先产品创建/真实登记/trust→固定Claude recipe→#8最终policy文件/artifact/receipt→真实索引ready；严格顺序`Story fresh→Story resume/ChoiceFollowup→Design fresh→Design resume/Revision→Plan fresh(author+sync split+plan review)→Plan各原入口resume→显式StartCoding fresh→Coding native resume/retry→Review fresh(code/group/internal)→Review resume`。同家不同stage native id不交叉。Plan同步split必须纳入fresh真实支持和resume独立证据；若协议不足保持该resume格Unknown+零spawn并报告未满足该格验收，不能缩成只验Plan流式、不能用fresh伪装resume。产品确认走真实HTTP/WS gate，#13 Fake默认只用已有provider_select，不直写状态或能力。
- [ ] **Step 4：Main执行四家真实矩阵。** 从worktree根运行（新增开关只选测试与证据落点，不向provider注入政策）：

```bash
LC_GATEWAY_E2E=1 cargo test --locked --test it_web lcg_live_claude_five_stages_fresh_resume -- --ignored --nocapture --test-threads=1
LC_GATEWAY_E2E=1 cargo test --locked --test it_web lcg_live_codex_five_stages_fresh_resume -- --ignored --nocapture --test-threads=1
LC_GATEWAY_E2E=1 cargo test --locked --test it_web lcg_live_pi_five_stages_fresh_resume -- --ignored --nocapture --test-threads=1
LC_GATEWAY_E2E=1 cargo test --locked --test it_web lcg_live_kimi_five_stages_fresh_resume -- --ignored --nocapture --test-threads=1
```

每个case输出`evidence_root/cell`与实际result；四家×五阶段×fresh/resume至少40主格，Plan/split与Review子入口另列，不能合并隐藏。版本记录必须重新读取当前真实CLI（旧报告2.1.283/0.155.1/0.86.1/2.0.2仅起点，不直接钉支持）。任一缺证据格记录Unknown/Denied+reason，成功支持格具全部断言才PASS；环境不可运行须报告BLOCKED，不能删格宣布change全通过。
- [ ] **Step 5：提交harness。** `git add tests/it_web.rs tests/it_web/web_lc_gateway_multi_provider/mod.rs tests/it_web/web_lc_gateway_multi_provider/harness.rs tests/it_web/web_lc_gateway_multi_provider/live_matrix.rs && git commit -m "test: 建立四家 LC 五阶段真实 fresh 与 resume 矩阵"`；现场各家不commit报告，Main收口落盘。

**每格证据形态：** `cell.json`含stage/action/role/entrypoint/provider/version/gateway+wire dialect/native id/session/run；`provider-events.jsonl`含真实argv或脱敏RPC/ACP请求响应、tool_call/tool_result/approval；`frozen-facts.json`含policy_id/revision/raw-body/capability row/projection/trust/适用bundle digest与canonical cwd/target/git identity；`pre-snapshot.json`/`post-snapshot.json`、D4 baseline引用、`boundary-attempts.jsonl`、`result.json`与sha256清单。敏感token/API key/home无关trust条目不落盘；provider-start真实wire不能空argv占位，子进程PID/时间线需可追溯。


## Task 11：越界写、D4与失败零spawn真实验收（tasks.md 4.2；REQ-LCG-07）

**Files：** 11a 2–4h：Create `tests/it_web/web_lc_gateway_multi_provider/failure_matrix.rs`、`tests/it_web/web_lc_gateway_multi_provider/boundary_matrix.rs`；Modify `tests/it_web/web_lc_gateway_multi_provider/mod.rs`（10a后串行include）；四家现场11b–11e每片≤4h，Evidence `cadence/reports/lc-gateway-multi-provider/<provider>/boundary/`、`failures/`。汇总 `cadence/reports/2026-10-03_验收报告_LC网关多provider写边界_v1.0.md` 与Task6同一report，唯一Main整合owner；不并改一个报告。

**Interfaces：** Consumes6 boundary/D4与10harness；Produces `LiveLcGatewayHarness::run_boundary_and_failure_matrix(provider:ProviderName)->Result<LiveMatrixEvidence,LiveMatrixFailure>`同schema。spawn统计session/协议child、extension/MCP后代、native方法次数，availability `--version`另记不冒充session。完整fixture失败证据都逐格保留；不能观察OS保护则Unknown。

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

**Files：** 12a/12b各2–4h；Create `tests/it_web/web_lc_gateway_multi_provider/policy_dependency.rs`、`tests/it_web/web_lc_gateway_multi_provider/direct_comparison.rs`；Modify `tests/it_web/web_lc_gateway_multi_provider/mod.rs`（串行）；Evidence `cadence/reports/lc-gateway-multi-provider/dependency-8/`、`direct-comparison/`；Main汇总 `cadence/reports/2026-10-03_验收报告_LC网关多provider真实E2E_v1.0.md`。Read-only #8 change与`src/web/handlers/aggregate_initialization/production_dependencies.inc.rs`、root receipt/recipe，不能为了跑矩阵手工发布政策。

**Interfaces：** Consumes10/11格证据、#8 `canonical_root/policy_id`+raw UTF-8 SHA-256/artifact/receipt/revision链，固定Claude recipe与4家direct。Producestest-only `LiveLcGatewayHarness::assert_policy_publication_chain()->Result<(),LiveMatrixFailure>` 与总矩阵结论；没有新policy publisher/fixture API。

- [ ] **Step 1：写失败测试。** `lcg_t12_missing_publication_blocks_every_provider_without_fixture_seed`、`lcg_t12_body_revision_receipt_drift_blocks_before_spawn`、真实`lcg_live_fresh_lc_policy_publication_recipe_and_direct_comparison`。断言：

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
assert_eq!(direct_after, direct_before);
```

- [ ] **Step 2：Main运行红测试。** `cargo test --locked --test it_web lcg_t12 -- --nocapture`；新消费gate断言应红；若#8未产品化，真实成功case必须标外部依赖BLOCKED，不fixture物化后改成绿。缺件/漂移负向case不依赖#8成功交付，可先执行。
- [ ] **Step 3：实现只读验收与汇合。** 新fresh fixture全部由产品API/真实Claude recipe与#8生产者提供根政策正文/locator/artifact/receipt；consumer只校验`policy_id`文件原字节等于`artifact.policy_text`、raw SHA-256等于artifact/receipt.policy_digest，独立`rule_digest`校验AGENTS原字节，不在网关重定/执行producer聚合。成功E2E前置是#8明确提交与无seed fresh/存量重冻结证据已由Main验收，再从新LC起跑10/11；旧v1.1手工物化不能替代。四家真实direct argv/cwd/permission/output与baseline一致，recipe在normal全Unknown时仍固定凭据/原recipe evidence。逐格关联REQ/任务/证据，缺件waiting不可改为allow。
- [ ] **Step 4：Main真实验证与契约校验。** `LC_GATEWAY_E2E=1 cargo test --locked --test it_web lcg_live_fresh_lc_policy_publication_recipe_and_direct_comparison -- --ignored --nocapture --test-threads=1`；预期#8完全产品化后fresh链无手工seed且4家direct对照一致。`openspec validate lc-gateway-multi-provider` 与 `openspec status --change lc-gateway-multi-provider`校验规划/契约，不当作CLI验收。旧报告#8/#10/#13台账只交叉引用，#10/#13层1仍原样非目标。
- [ ] **Step 5：提交测试/交付报告。** `git add tests/it_web/web_lc_gateway_multi_provider/mod.rs tests/it_web/web_lc_gateway_multi_provider/policy_dependency.rs tests/it_web/web_lc_gateway_multi_provider/direct_comparison.rs && git commit -m "test: 锁定政策发布前置与 LC recipe direct 对照"`；Main落总报告，报告不暂存。若#8未交付，Task12验收状态BLOCKED，已完成负向证据保留；任务不能勾完成或归档。


## Task 13：Direct/GitFinalize/人工StartCoding零回归（tasks.md 5.1；REQ-LCG-02）

**Files：** 13a/13b各2–3h；Modify/Test `tests/it_web/provider_gateway_envelope.rs`（dual stack/direct输出对照）、`src/product/work_item_split_engine/tests/engine_gateway_guard.rs`（direct sync）、`src/web/coding_start_parts/tests.inc.rs`（人工StartCoding）、`src/web/coding_ws_handler/tests/sc_start_guard.rs`（先advance门）、`src/web/handlers/aggregate_initialization/tests_parts/root_safety.inc.rs`（D3 recipe隔离）；Test `tests/it_web/web_repository_initialization/operation_http.rs`（六步GitFinalize既有锁）。此Task测试独占，Task7/9改同名文件完成后再运行；同源角色链case归8，不并改automation实现。

**Interfaces：** Consumes既有raw `ProviderAdapter::run`、`StreamingProviderAdapter::start/run_streaming`、`StartCodingCommand`、`CodingStartOrigin::Manual`、单仓six-step/四命令/GitFinalize。Produces实名回归断言；生产接口保持不变，不为测试新造direct fallback。

- [ ] **Step 1：写失败回归。** `lcg_t13_direct_four_providers_keep_exact_args_cwd_permission_and_output`、`lcg_t13_sync_direct_never_uses_lc_bridge`、`lcg_t13_ready_plan_does_not_start_other_target_without_manual_command`。扩展root-safety断言normal matrix Unknown不改变recipe隔离；具体断言：

```rust
assert_eq!(direct_args_after, direct_args_before);
assert_eq!(direct_cwd_after, direct_cwd_before);
assert_eq!(direct_output_after, direct_output_before);
assert_eq!(direct_permission_after, direct_permission_before);
assert_eq!(lc_bridge_calls_for_direct, 0);
assert_eq!(other_target_attempt_count_after_confirmed_plan, before_count);
assert_eq!(provider_start_count_before_explicit_start_coding, 0);
assert_eq!(repository_initialization_steps[5], "git_finalize");
```

- [ ] **Step 2：Main运行红测试。** `cargo test --locked --test it_web lcg_t13_direct_four_providers_keep_exact_args_cwd_permission_and_output -- --nocapture`、`cargo test --locked --lib lcg_t13`。新增LC邻接身份/参数对照若先PASS如实记baseline绿，不制造假红或改生产行为使其红；本工作包是兼容锁验收，不是新增功能。
- [ ] **Step 3：完善回归fixture。** 使用既有单仓builder/legacy与初始化脚本事件，观察实际调用/cwd/输出/步骤；Fake仅确定性测试替身，四家真实对照由Task12承担，不标真实支持。保留 `shared_executor_cannot_reach_repository_registration_or_git_finalize`、`manual_start_coding_keeps_sc_ready_gate`、`start_coding_before_advance_ready_is_rejected_with_sc_coding_requires_advance` 原行为；不得从ready Plan新增自动多target派工。若发现实现回归，退回拥有该实现文件的Task修复并范围复审，13不另改边界。
- [ ] **Step 4：Main运行绿回归。** `cargo test --locked --test it_web provider_gateway_envelope -- --nocapture`、`cargo test --locked --test it_web web_repository_initialization -- --nocapture`、`cargo test --locked --lib engine_gateway_guard`、`cargo test --locked --lib manual_start_coding_keeps_sc_ready_gate`、`cargo test --locked --lib shared_executor_cannot_reach_repository_registration_or_git_finalize`；新旧均PASS，单仓six-step与LCfive-step仍分离。
- [ ] **Step 5：提交。** `git add tests/it_web/provider_gateway_envelope.rs src/product/work_item_split_engine/tests/engine_gateway_guard.rs src/web/coding_start_parts/tests.inc.rs src/web/coding_ws_handler/tests/sc_start_guard.rs src/web/handlers/aggregate_initialization/tests_parts/root_safety.inc.rs tests/it_web/web_repository_initialization/operation_http.rs && git commit -m "test: 锁定单仓 direct 与显式 StartCoding 兼容"`；13a/13b按本段实际文件分别提交。


## Task 14：存量迁移等待面与运维证据（tasks.md 5.2；REQ-LCG-01/03/04）

**Files：** 14a 2–4h：Modify `src/product/logical_codebase/provider_capability_store.rs`（存量读取/evidence失效测试）、`src/product/logical_codebase/provider_admission_preflight_tests.inc.rs`（waiting材料/allowed actions）、`src/web/handlers/automation_gateway_preflight.rs`（诊断投影测试，仅8完成后串行）；14b 1–2h：Evidence `cadence/reports/2026-10-03_验收报告_LC网关多provider能力迁移与运维_v1.0.md`、`cadence/reports/lc-gateway-multi-provider/capability-migration/`。Read-only当前14工作包/REQ/spec及HEAD预算，不改OpenSpec tasks状态（由Main验收后勾选）。报告同时承载运维说明与永久行为变更记录，不另建重复changelog。

**Interfaces：** Consumes2旧matrix Unknown及root-recipe隔离、3 waiting、5 Codex限制、6/10/11真实evidence；Produces当前精确 `provider/version/OS/action/entrypoint/fresh-or-resume/state/reason/evidence_ref`支持矩阵与操作顺序：补前置→真实probe→校验证据→产品Revalidate/Retry→完整fresh admission。不得提供手工JSON改Confirmed操作或新“迁移默认allow”。

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

1. **Spec coverage：** `tasks.md`14个工作包逐一映射Task1–14；Spec七条requirements/全部scenario覆盖：映射/Unknown/fresh-vs-resume→1/2/14；admission/漂移/cwd/双栈→1/3/8；projection/logical roots/boundary→4/5/6/11；Codex三场景→5/8/11；resume四家/Unknown/fingerprint→9/10/12；guard/多违规/false标志→7/8/13；真实支持/越界缺证据→10/11/12。单仓、recipe、手工StartCoding非目标以13锁定；#8外部依赖明确等待，没有范围缺口用fixture遮盖。
2. **Step scan：** 每Task均有Files/Interfaces/五步骤、失败测试名+Rust断言、红绿定向命令和commit；真实10–12有ignored名、开关/命令/证据路径与blocked语义。13baseline可先绿，不伪造红；原生握手已启动失败与spawn前零child区分。完整路径机械检查仅未存在Create新模块/新证据产物，消除省略后误指tests/policy_session.rs。穷举enum当前callers在1a即时迁移保持可编译；matrix/schema后续2b再迁，不留半编译提交。
3. **Type consistency：** provenance保留原`CapabilityEvidence`，三态唯一复用`ProviderCapabilityEvidence`；record字段保留`version/adapter_dialect/evidence`避免新旧拼写漂移；新增`ProviderActionCapability/ProviderPolicyProjector/ProviderPolicyProjection/CodexSandboxProjection/ProviderBoundaryPlan/LcProviderStartAudit`均有唯一产出Task。cwd/root与target/git identity独立；normal matrix与root-recipe evidence不混用；policy raw正文消耗#8原接口，不另造sidecar。
4. **Review Focus：** 五风险逐一落实名新测试：version→`lcg_t02_cli_version_drift_invalidates_row`/`lcg_t05_version_or_protocol_cwd_drift_invalidates_projection`/`lcg_t14_cli_upgrade_keeps_history_and_blocks_until_reprobe`；竞态→`lcg_t03_validate_then_each_frozen_dimension_drifts_zero_spawn`；单仓误门→`lcg_t08_single_repository_skips_lc_predicates`/`lcg_t13_direct_four_providers_keep_exact_args_cwd_permission_and_output`；digest/physical边界→`lcg_t04_projection_digest_changes_on_target_role_tool_or_config`/6/11；resume旧指纹→`lcg_t09_legacy_audit_missing_projection_cannot_resume_lc`/10/12。target git commit与`.git`指针保护及phase capability隔离已采用Main定点裁决。
5. **Proportion：** 计划保留路径、签名、契约固定值、测试断言和现场证据形态，没有生产函数体。七条spec+14包+权威设计与报告底座是完整输入；算法只钉digest/挂载/root-target/同步runtime中不可随意选择的边界。模板代码块只占少数，真实现场费用不化成开发支持结论。原草稿错误已经在计划接地表纠正，未更改获批契约。

## 审查关注与实施前门

| 定点发现/裁决 | 计划落点与验证要求 |
|---|---|
| 旧provenance同名冲突、二态resume丢reason | Task2保持provenance名字、复用真正三态、legacy只DTO读取；Denied roundtrip和未知版本不allow |
| recipe与正常会话共用capability导致误伤 | Main确认phase-aware最小分流；Task2/3/1c保持凭据/原recipe evidence，不豁免capability、不升级normal矩阵 |
| Coding负向`.git`与既有commit职责 | Main确认保护root/non-target metadata及target指针；Task6/11受控target commit为正向，冻结授权git-dir不宽授整个.git |
| #8原草稿虚构sidecar/locator trait | Task3/12只消费canonical root/policy_id/raw body/artifact/receipt链；#8产品化前全部成功E2E等前置，无fixture补件 |
| sync Pi/Kimi当前两槽拒绝、裸CLI缺root cwd/guard | Task1选validated streaming-to-sync bridge复用真实四家，direct/task-run不改；timer/输出/零legacy bridge断言 |
| resume adapter当前drift→fresh/request-id回填 | Task9 LC严格确认、不清id fresh；direct旧恢复语义原样，握手失败kill与零spawn分开 |
| 并行源码交叉/现场home竞争 | 批次表和各Task精确Files冻结；gateway/streaming/mod/fixture公共面串行，四家现场独占证据与trust锁 |
| #8 producer与consumer、early与spawn时序 | Main确认只消费最终policy_text原字节及独立rule_digest；Task3/8早期只读资格无未来D4，实际spawn完整事实复验没有豁免 |


实施前Main并行接地审查与一致性裁决；本计划编写者只做上述技能自审。开放证据事实限于当前CLI native边界/protocol cwd/root discovery、namespace/OS与MCP外部写面、多版本/大LC新成本、#8实际产品化状态；每项均有Task6/10–12/14的真实probe或显式waiting门，不是实现占位。未支持环境/版本不能默认allow，所有可到达的功能与证据采集均按任务完成，缺外部材料如实BLOCKED。


