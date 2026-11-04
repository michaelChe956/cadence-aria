# 设计：聚合冷启动可用化与审计预算口径

> 完整技术方案见 `cadence/designs/2026-11-02_技术方案_块2_聚合冷启动与审计预算_v1.0.md`（owner 2026-11-02 审阅通过，2026-11-03 补决策；§7 全部拍板）。
> 本文件是 OpenSpec 层的**决策记录**：背景 / 决定 / 理由 / 代价。逐条覆盖技术方案 §3 的七大块（B-1 / B-2 / B-3 / B-4 / K-1 / K-2 / K-3 + §3.6 provider 安装更新 + §3.7 B-5 残项）。
> 落点与验收标准见技术方案与 `tasks.md`；本文件**不重新设计方案**，只把已过审的结论翻译成决策条目。

## 决策 1：provider 按 owner 顺序自动选择，在 operation 创建前完成并冻结（B-1）

**背景**：provider 被三重写死 —— `src/web/handlers/aggregate_initialization/handlers.inc.rs:98` 的 `provider: "claude_code".to_string()`、`src/product/logical_codebase/coordinator_provider_turn.inc.rs:44-56` 的硬构造 `ProviderRef::claude_code(...)`（**真正决定 provider 的地方**）、`handlers.inc.rs:612` 的 trust 门准入清单 `&[ProviderName::ClaudeCode]`。其中 `input.provider` 字段全仓唯一读者是测试断言（`aggregate_initialization_store_tests.inc.rs:315`）→ 它只是记账字段，执行链路无人读。现场结果是 `provider command not found: claude` → 预检 denied，即使该机装了 pi / kimi / codex 也一次都不试。

**决定**：新建 `src/product/logical_codebase/provider_selection.rs`，按 owner 顺序 `Pi > KimiCode > ClaudeCode > Codex`（`AGGREGATE_PROVIDER_CANDIDATE_ORDER`）逐个探测，选中**第一个**可用的 provider；**选择发生在 begin 之前**（operation 创建之前），选中后全程只用一个 provider（中途换人会让聚合产物割裂）。判定分两层：

| 层 | 判什么 | 数据来源 | 不通过时 |
|---|---|---|---|
**L1** 命令可用 | 该 provider CLI 在本机可用 | `ProviderHealthSnapshot.entry(&p)` 的 `available` + `reason_code`（六值：`command_missing` / `timeout` / `non_zero_exit` / `version_unparseable` / `version_too_low` / `io_error`） | `CliMissing{code, reason}` |
**L2** 本 LC 能力已核验 | 该 LC 的 `PlanningReadOnly.launch == Confirmed`，且 `capability_snapshot_ref` 与运行时约定一致 | `ProviderCapabilityStore::for_lc(paths, lc_id).get(project_id, ref_type)` + `row.action_matrix.row(&PlanningReadOnly).launch` | `Unverified` / `Denied{reason}` / `EvidenceStale` |

只判 `SessionPolicyAction::PlanningReadOnly`（这是聚合 driver 唯一使用的 action，`coordinator_provider_turn.inc.rs:120`）。

L1 采用「读快照 + 条件刷新」：读 `latest_diagnostic()` 的逐家 `available`（`provider_health.rs` 的刷新已并发探完四家）为主路径；**仅当** `generation == 0`（从未探过）/ `degraded()` / `checked_at` 早于 `now - 10min` 时才 await 一次 `refresh()`。

`degraded() == true` 时**不创建 operation**，返回 `aggregate_initialization_provider_health_degraded` —— 因为 `provider_availability_gate.rs:107-112` 里 degraded 是一票否决，若无视 degraded 选了 pi，operation 会先创建、再在 spawn 前被 gate 拒成 `provider_unavailable: Pi: provider health state is degraded`，用户看到一次莫名其妙的失败。

选中结果落两处：`operation.input.provider`（wire 名 `pi` / `kimi_code` / `claude_code` / `codex`，执行链路唯一来源）与新增 `operation.provider_selection: Option<AggregateProviderSelection>`（完整四家候选表 + 判定依据 + 规则版本，回答「为什么」）。新字段沿用既有先例（`full_recipe_requested` / `skills_initialization` 都是 `#[serde(default)]` 加在 byte-stable 记录上），候选表**按 owner 顺序排列**（不是按结果排序）。

**理由**：
- owner 的核心诉求是「新机器上一次跑通」；写死单一 provider 让「装了别的 CLI 的机器」完全无路可走
- 选择器只是**提前把 gateway 会拒绝的理由读出来**，gateway 仍然自己 fail-closed → 准入不放宽一个字节
- 条件刷新兼顾「新机器 ≤5s 一次」与「之后零延迟」，且不违反 `start_lc_aggregate_initialization` 的「同步只做 durable 写 + spawn」契约（每次现场刷新最坏 ~5s 阻塞 HTTP）

**代价**：
- L1 快照可能过期 → 用条件刷新兜底，但仍有窗口
- trust 语义随 provider 变化：选 Pi / ClaudeCode → `requires_workspace_trust` 过滤后为空（`provider_trust.rs:923-926`）→ trust 门 no-op，与今天 Claude 行为等价（零回归）；选 KimiCode / Codex → 触发 workspace trust 登记，未 trusted 时走既有 `aggregate_initialization_trust_waiting` 链路
- 约 5 处「必须保持 Claude」的测试断言需重新评估（`coordinator_tests_contract.inc.rs:107-110`、`coordinator_tests_profile_trust.inc.rs:637-641`）→ 改为断言「aggregate recipe provider == operation.input.provider」（改断言口径，不改语义）

## 决策 2：选择器与 gateway 共用同一个能力判定函数（B-1 §3.1.2）

**背景**：`src/product/logical_codebase/production_policy_resolvers.rs:301-318` 的 `require_supported` 只放行 `ProviderCapabilityEvidence::Confirmed`，其余走同一格式串 `{action:?} launch cell is {other:?}`；另有记录缺失分支（`:262-266`，文案 `capability record missing`）与 snapshot 漂移分支（`:283-289`，文案 `capability snapshot mismatch`）。若选择器另写一套判定，两处迟早漂移。

**决定**：不新造判定 —— 把 gateway 那三条**逐字复刻**并抽到 `provider_capability_store.rs` 的一个只读方法上，让 gateway 与选择器共用同一函数：

```rust
pub enum LaunchCellState {
    Confirmed,
    Unverified { detail: String },   // 记录缺失 / cell=Unknown
    Denied { reason: String },        // cell=Denied{reason}
    EvidenceStale { detail: String }, // snapshot ref 漂移
}
impl ProviderCapabilityStore {
    pub fn launch_cell_state(&self, project_id: &str, provider: ProviderRefType,
                             action: SessionPolicyAction) -> Result<LaunchCellState, ProductStoreError>;
}
```

`require_supported` 改调它，并**保留原有错误前缀与文字形态**（`provider_capability_launch_not_confirmed: ...`）。

**理由**：
- 这是本方案最容易做错的地方：两处各写一套 → 选择器放行、gateway 拒绝（或反之）→ 用户拿到「选了却用不了」或「拦错了」的失败面
- gateway 的 fail-closed 语义与稳定码已有测试锁定（只断言 `starts_with(PROVIDER_CAPABILITY_LAUNCH_NOT_CONFIRMED)` 与 `contains("capability record missing")`）→ 抽函数不改文案形态即不破测试

**代价**：`production_policy_resolvers.rs` 与 `provider_capability_store.rs` 之间多一个共享方法；`require_supported` 的错误文案形态成为**对外契约**，改它需同步改既有测试。

## 决策 3：能力未核验时引导用户显式核验，不自动触发探针（B-1 §3.1.4）

**背景**：新机器上四家全 `Unknown` 时，若平台自动触发核验，真实 probe 会启动 provider 跑 LLM 轮次 → operation 创建被阻塞分钟级并花掉 provider 额度。`provider_capability_revalidate.rs:1-10` 的契约明写「不点击不运行任何探针」。

**决定**：**不自动触发**。正确行为是**不启动**，并把「去点核验」变成主文案 + 一个按钮：

```
1. 点「聚合初始化」→ 选择器：L1 过（pi 装了），L2 = Unverified
   → 不创建 operation，返回 aggregate_initialization_provider_needs_verification
   → details 带：建议核验的 provider（按顺序第一家 L1 过的）、四家候选表、核验入口、预计时长
2. 前端渲染「核验 Pi」按钮（既有组件与 API：ProviderCapabilityCard.tsx:213-225、provider-capability.ts:48-59）
3. 点 → POST .../capability-revalidate {provider_type:"pi"} → 真实 probe 三 action
   → 导入 durable Confirmed，落盘 capability_snapshot_ref = "cap_managed_snapshot"
4. 点「重试」→ 选择器 L2 = Confirmed → 选 Pi → begin（provider 冻结为 pi）
```

失败面只指向**单个「核验 X」**按钮（owner 决策 9）—— 票3「一键核验全部」留在块3，块2 上线时那个按钮还不存在，文案**不得**指向它。

**理由**：
- 自动触发（同步）与「不点击不运行任何探针」的既有契约**直接冲突** → 必须改契约才能做 → 超出本方案范围
- 自动触发（异步）要给五步状态机加前置态，而 `aggregate_initialization.rs:161-169` 的状态机是 byte-stable durable 协议，改动风险高，且把「冷启动」变成「自动花额度」
- 引导方案用户两次点击、不花 provider 额度，且不改任何既有契约

**代价**：新机器首次可用需要用户多两步（点核验 → 等 → 点重试）；真实 probe 耗时未量化（技术方案 §8 未验证项 2：真机记录墙钟后再把区间写进文案定稿）。

## 决策 4：provider 走 request，不进 driver 字段（B-1 §3.1.6）

**背景**：`GatewayFactoryProviderTurnDriver` 是 **state 级单例**（`production_dependencies.inc.rs:872-876`），`for_lc` 不派生它 → 若把 provider 放进 driver 字段，同一进程里两个 LC 会互相覆盖（跨 LC 串味）。

**决定**：provider **必须走 request**，闭环三处：

1. `aggregate_initialization_coordinator.rs:276-300` → `AggregateProviderTurnRequest` 增 `provider: &'a ProviderName`
2. `coordinator_lifecycle.inc.rs:790` `run_provider_turn` → 从 durable 操作记录解出 provider 再传（`provider_from_wire(&operation.input.provider)?`，未知串 fail-closed）→ **「重试不换人」成为结构性保证**
3. `coordinator_provider_turn.inc.rs:44-70` → 改成按 `ProviderName` 构造，**保留 admission 预检**：`with_provider(gateway, provider, ref)` 与 `with_provider_and_admission(gateway, provider, ref, paths)`，用 `ProviderRef::from_provider_name`（`provider_gateway.rs:470-487`）—— 四家显式映射、Fake fail-closed、**无 `_ =>` 静默回退**

旧 `claude_code` / `claude_code_with_admission` **删除**（clean cutover），迁移其调用点（含测试夹具）。过期注释必须顺手删：`coordinator_provider_turn.inc.rs:41-43`（Codex 路由阻断）与 `provider_gateway.rs:467-469` —— 实现侧 `enforce_route_policy` 已是 `Ok(())` 纯桩、`provider_gateway.rs:1557-1561` 明说规则已退役、`:2961-2964` 说该判别码不再产生。不删的话下一个改这块的人会再次得出「aggregate 只能用 Claude」。

**理由**：
- driver 是 state 级单例 → 字段方案在多 LC 并存时会串味，而串味的后果是「A 的 LC 用 B 的 provider 跑聚合」→ 产物错乱且难以归因
- 「重试不换人」是聚合产物一致性的前提：从 durable 记录解出 provider 后，重试天然沿用同一个 provider

**代价**：
- 多一层 wire 解析（`provider_from_wire`），未知 provider 名必须 fail-closed（不能回退 Claude）
- 测试夹具与 5 处断言需改写

## 决策 5：能力状态先可机读，再按三态分流失败面（B-2 §3.2）

**背景**：`src/cross_cutting/provider_capabilities.rs:12-24` 的 `ProviderCapabilityEvidence` 已是三态（`Confirmed` / `Denied { reason }` / `Unknown`），但错误出口把三态压平成同一句，`Denied` 的真实 reason 被塞进 Debug 括号，`Unknown` 也走这一句 → 用户无法分辨「没核验」与「真的不行」。现场：`provider_capability_launch_not_confirmed: PlanningReadOnly launch cell is Unknown`。

**决定**：两步走。

**第一步（可机读）**：在 `provider_gateway.rs:2973-2975` 常量旁新增稳定词表 —— `capability_launch_state_token(evidence)` → `"confirmed"` / `"denied"` / `"unknown"`（**禁止依赖 Debug 输出**）。`production_policy_resolvers.rs:314-317` 改为按态分流（`Denied` 额外带 reason）。

**第二步（分类器）**：

```rust
pub enum CapabilityLaunchRejection {
    Unverified { action: String },              // cell=unknown 或 记录缺失
    Denied { action: String, reason: String },  // cell=denied
    EvidenceStale { detail: String },           // capability snapshot mismatch
}
pub fn classify_capability_launch_rejection(detail: &str) -> Option<CapabilityLaunchRejection>;
```

**第三步（三路文案 + 三类分诊）**：`aggregate_initialization_coordinator.rs:117-153` 的 `error_record()` 对 `ProviderTurn{reason}` 先过一次分类器：

| 分类 | `reason_code` | 中文主文案 | 指引 |
|---|---|---|---|
**未核验** | `aggregate_provider_capability_unverified` | 「这台机器上 **{display}** 已安装，但本逻辑代码库还没有核验它的能力。」 | `details.revalidate = { ui_path, button_label, method, path, body, expect }` |
**真实拒绝** | `aggregate_provider_capability_denied` | 「**{display}** 在本机实测不支持这一步需要的能力（原因：{reason}）。请按优先顺序改用下一家：**{next_display}** —— 先核验 {next_display}，再重试。」 | `details.fallback_provider` + `details.revalidate`（指向 next） |
**证据过期** | `aggregate_provider_capability_evidence_stale` | 「**{display}** 上次核验的是本机 CLI 的旧版本，证据已作废。请重新核验 {display} 后再重试。」 | `details.revalidate` |
都不匹配 | 既有 `aggregate_{step}_failed` | **保持原样**（零回归） | — |

三类分诊（用户视角 → 技术判据 → 下一步）：

| 用户视角 | 技术判据 | 主文案 | 下一步 |
|---|---|---|---|
**你的环境要处理** | `reason_code ∈ {command_missing, version_too_low}` | 「**{display}** 没安装 / 版本太低：{install_hint}」 | 复用 `providers.rs:81-110` 的 `install_hint` 原文；装完点「重新检测」（`POST /api/providers/recheck`） |
**产品缺陷** | `degraded()==true` / `evidence_stale` | 「核验记录与本机状态不一致（内部状态异常）」 | 给错误原文 + 「重试」；连续两次仍如此 → 报 issue，附机器码 |
**重试即可** | `timeout` / `non_zero_exit` / trust waiting | 「探测超时，稍后重试」 | 「重试」按钮；trust 类走既有 `aggregate_initialization_trust_waiting` |

**机器码降级**：`stderr_summary`（`aggregate_initialization.rs:196-197` 本来就存在且未被占用）→ 放**完整原始 detail**，走既有 `details.stderr_summary`（`dto:103-110`）→ **不改 DTO 形状**。

**逐家点名要求**：`NeedsVerification` / `Denied` / `NoCliAvailable` 的 `details.candidates` 必须四家齐全、按 owner 顺序，每条含 `{provider, display_name, cli_available, cli_reason_code, capability_state, action_hint}`，`action_hint` 是**可执行动作**（「未安装：`npm i -g @pi/cli`」/「未核验：点『核验 Pi』」/「实测拒绝：改用 Kimi Code」）。

**理由**：
- 「未核验」与「实测不支持」是两类完全不同的用户动作：前者点一下核验即可，后者必须换 provider 或改环境 → 压平成一句话必然指错路
- 分类器是**纯字符串分类**，不新增判定逻辑 → 与决策 2 的共用函数不冲突；不匹配的 detail 落回既有码 → 零回归
- 既有测试只断言 `starts_with(...)` 与 `contains("capability record missing")` → 改文案形态不破测试

**代价**：
- `error_record()` 与文案表新增依赖；「不匹配 → 保持原样」这条零回归分支必须保留（否则所有未分类失败都改文案）
- 逐家点名要求 `details.candidates` 四家齐全 → DTO 体积变大，但这是「为什么不能用」的必要解释

## 决策 6：失败面文案单一来源，机器码降级（B-3 §3.3）

**背景**：三层叠加导致 `no stderr summary` 恒空：执行层 `coordinator_provider_turn.inc.rs:267-374` 的 `consume_turn` 在 11 条失败分支里有 **6 条把已捕获的输出直接丢弃**（status failed / aborted / permission timeout / stream closed / cancelled / timed out）；记录层 `aggregate_initialization_coordinator.rs:155-168` + `aggregate_initialization.rs:201-210` 把 `stderr_summary` 硬编码 `None`；**投影层** `bootstrap_projector.inc.rs:344` 与 `:586` **只读 `stderr_summary`，从不读 `action`** —— 而 `action` 里本来就带着真原因。

**决定**：新建 `src/product/logical_codebase/failure_copy.rs`（纯函数、无 I/O、可单测）作为**失败面文案唯一来源**：

```rust
pub struct FailureCopy {
    pub title_zh: String,      // 一句话说清是什么问题，点名对象
    pub triage: FailureTriage, // Environment / ProductDefect / RetryOnly
    pub action_zh: String,     // 点哪里 / 跑哪条命令 / 改哪个文件 + 要等多久
    pub machine_line: String,  // 机器码 + 原始 detail，永远保留，作为次要行
}
pub enum FailureTriage { Environment, ProductDefect, RetryOnly }
pub fn describe(record: &AggregateInitializationErrorRecord) -> FailureCopy;
```

**WP-A（主修复，改动小、见效最快）**：`failure_copy.rs` 新建 + `bootstrap_projector.inc.rs:334-347` 与 `:575-589` **共用同一函数**（禁止第二份拼接）+ `bootstrap_types.inc.rs:58-63` 的 `BootstrapFailure` 增 `#[serde(default)] pub triage: Option<String>;` + `LogicalCodebaseBootstrapCard.tsx:314-318` 主行渲染 `failure.detail`，`原因 {reason_code}` 降级为折叠区。

**WP-B（结构层）**：
1. `src/cross_cutting/diagnostic_redaction.rs`（新建，抽自 `src/product/repository_store/initializer.rs:433-475`）把 `sanitize_and_truncate` + `redact_sensitive_assignments` 提升为公共函数，并**新增脱敏规则**
2. `aggregate_initialization.rs:191-199` 增 `cause: Option<String>`；`aggregate_initialization_coordinator.rs:155-168` 的 `error_record()` 按变体填 `stderr_summary` 与 `cause`
3. `aggregate_initialization_coordinator.rs:60-87` 的 `SkillsPreparation` / `Preflight` / `ProviderTurn` 三变体各增 `diagnostic: Option<TurnDiagnostic>`；新增 `EnvironmentPrecondition { code, reason, retryable }`（B-4 复用）
4. `coordinator_provider_turn.inc.rs:267-374` **6 条丢弃输出的分支补 `output.summary()`**；`:502-520` 启动期失败若底层是 `ProviderAdapterError` 则取 `code` / `stderr` / `stdout`
5. `production_dependencies.inc.rs:19-24` 的 `SkillsPreparation` 保留**结构化 stage**（`skills_unavailable_{stage}` / `skills_update_failed_*` / `skills_sync_failed_{layer}`）
6. `coordinator_preflight.inc.rs:35-130` 增加「聚合根可写」探测（在 `canonical_root` 下 `create` 一个 `.aria-probe-<nonce>` 再删）→ 失败为 `EnvironmentPrecondition { code: "aggregate_root_not_writable" }`
7. `coordinator_lifecycle.inc.rs:749-771` machine_skills 成功后校验四条命令的**链接层**（`shared/codex/claude` 三层的 `~/.claude/skills/{pre-check,rule-config,mcp-configuration,project-rules-examples}/SKILL.md` 必须存在）→ 缺失为 `EnvironmentPrecondition { code: "skills_commands_missing" }`
8. `src/web/handlers/dto/aggregate_initialization.rs:97-115` 的 `details` 增 `"cause"`；`message` 改为 `describe(&record).title_zh`（原文案挪到 `details.action`）

**截断与脱敏**：截断 `4096` 字节（与 `AGGREGATE_TURN_OUTPUT_LIMIT` 同值）+ **40 行上限**，超出标 `…[truncated]`。必去：既有凭据规则（`k=v` 且 key 大写后含 `KEY`/`TOKEN`/`SECRET`/`PASSWORD`）+ 新增 `Bearer <token>`、`Authorization: *`、`sk-[A-Za-z0-9_-]{16,}`、`ghp_*`、`xoxb-*`、`AKIA[0-9A-Z]{16}`、`--api-key <v>` / `--token <v>`。身份类路径：`$HOME` 前缀 → `~`；聚合根绝对前缀 → `<聚合根>`；成员仓内相对路径保留。保留命令名、文件名、错误码；控制字符仍剥离。不落全量 stdout（`BoundedOutput` 已有界）。

**成因子码**（哪些能机器判定）：

| 记录里的候选原因 | 真实归属步骤 | 能机器判定 | 判定手段 | `cause` |
|---|---|---|---|---|
provider 未登录 | **只在三个 provider turn**，`machine_skills` 里不可能 | **半能** | stderr 正则 `not logged in\|please run /login\|unauthorized\|invalid api key\|401`；**未命中就诚实标 `unknown`** | `provider_unauthorized` / `unknown` |
CLI 不在 PATH | provider turn 启动前 + 本方案前移到冷启动前置探测 | **能** | 复用 `ProviderErrorCode::ProviderCommandMissing` | `provider_command_missing` |
该仓缺少四条命令 | 三个 provider turn（根因在 machine_skills 的链接层） | **能** | 改动 #7 | `skills_commands_missing` |
聚合根不可写 | `aggregate_preflight`（确定性） | **能** | 改动 #6 | `aggregate_root_not_writable` |
Cadence-skills 源拉取/更新失败（**记录漏掉的真实高频成因**） | `machine_skills` | **能** | 改动 #5 的 `stage` | `skills_unavailable_clone` / `skills_update_failed_*` |
链接层同步失败 | `machine_skills` | **能** | `CadenceSkillsError::SyncFailed{layer,skill}` | `skills_sync_failed_<layer>` |
HOME 不可用 | 依赖装配期 | **能** | `cadence_skills_home_unavailable` | 装配期错误，不进本记录 |

**理由**：
- 「是什么问题 + 怎么操作」是用户唯一真正需要的两件事；机器码是给人排障用的，**不能当主文案**
- 文案单一来源是防止「投影层 / DTO / 通知三处各拼一份」的最低成本手段 —— 拼三份必然漂移
- 改投影层就能立刻消灭 `no stderr summary`，不必等结构改造 → WP-A 先做（见效最快），WP-B 再补根因
- 顺带纠正记录里的归因偏差：`machine_skills` 是**确定性代码、不跑 provider**（`coordinator_lifecycle.inc.rs:335` → `AggregateSkillsPreparation::prepare_skills`），所以「provider 未登录」不可能是它的失败原因
- 脱敏与截断独立成公共模块（`diagnostic_redaction.rs`）→ 块3 票3 等其它失败面复用同一套，不用各写各的

**代价**：
- 新增 4 个 durable 字段（`error.cause` / `error.provider` / `BootstrapFailure.triage` / `preflight.codegraph_available` 等），一律 `#[serde(default, skip_serializing_if = "Option::is_none")]` → 老记录反序列化必须仍成功
- 「未命中就诚实标 `unknown`」意味着**有些失败仍然说不清** —— 这是刻意的诚实，不是遗漏
- 验收线：`grep -c "no stderr summary" src/` → **0**

**与块1 的边界**：块1（`fix-automation-enrollment-provider-swap`）的失败面分诊在**自动化授权**域（`EnrollmentError::Conflict` + `ConflictKind`），与本 change 的 `logical_codebase` 域**不重叠**；本 change 不改块1 的任何文件，也不复用其判别位。

## 决策 7：codegraph 维持致命 + 前置探测 + 只报一次（B-4 §3.4）

**背景**：`codegraph_cli.rs:304-316` 把 `CommandMissing` 映射为 `AggregateIndexError::Degraded { code: "codegraph_missing" }`，而 `operation.rs:468-482` 的落地状态对 `IndexApplicationMode::Initialize` **无视错误类型**一律落 `Failed`，`Degraded` 分支根本没参与判定 → 「Degraded 却致命」的表象。owner 已裁定：**冷启动（无 LKG）致命是对的**（`openspec/specs/logical-codebase-aggregate-index/spec.md` 早有规定：有 LKG 继续只读投影并标记 degraded，无 LKG 标记 missing/failed）；`Degraded` 服务的是「有 LKG 的 rebuild/sync」场景，**那条路径不动**。要改的是「谁先发现 + 报几次 + 用什么码」。

**决定**：
1. **落点**：`advance_remaining` 开头（`coordinator_lifecycle.inc.rs:307-340`），在 `machine_skills` 之前、第一个 provider turn 之前 → `self.probe.ensure_ready().map_err(|e| self.persist_execution_failure(project_id, operation_id, None, e))?`。`step: None` → 操作直接 Failed、**不把任何 step 标 Failed** → 保证「一点报错」而不污染步骤状态机
2. **不放在「UI 第一步 identity 之前」**：`identity` / `manifest_checkout` 是**投影**（每次读 durable 事实现算，不落盘），在上面加探测等于给只读投影加副作用；真正的执行入口只有 `advance_remaining`
3. **探测方式**：复用 `CodeGraphCli::verify_version()`（`codegraph_cli.rs:104-123`，已跑 `codegraph --version` 并校验 `>=1.6.0 && <2.0.0`）→ 注入 `Arc<dyn AggregateEnvironmentProbe>`，生产实现包一层 `CodeGraphCli`，**用与 `index` 同一个 `CodeGraphCli` 实例**构造（保证「探测的 CLI」与「建索引的 CLI」是同一份配置）
4. **探测结果落 durable 证据**：`AggregatePreflightSnapshot`（已持久化到 `preflight.json`）加 `codegraph_available: bool` + `codegraph_version: Option<String>`（均 `#[serde(default)]`）
5. **三处码统一**：`aggregate_preflight` → `codegraph_missing`（新变体，`reason_code` 直取 `code`）；`pre_check` provider turn → 前置探测后**不会发生**（万一绕过探测 → stderr 含 `command not found: codegraph` → 子码 `provider_command_missing`）；`aggregate_index_active` → 投影识别 warning 含 `codegraph_missing` → `reason_code = "codegraph_missing"`
6. **只报一次**：recipe 侧（探测失败 → operation Failed、step 全 `Pending`）；索引侧（`handlers.inc.rs:143-161` 与 `logical_codebase_bootstrap.rs:135-141`/`:356-360` 只在 `coordinator.execute(...)` 返回 `Ok` 之后才 spawn 索引构建 → 探测失败时索引构建根本不触发）；投影侧兜底（`project_member_index_step` 在 `latest.error.reason_code ∈ {codegraph_missing, codegraph_version_mismatch}` 时投影为 `NotStarted` + `allowed_actions: []` + `failure: None`）；通知去重（`project_notices` 按 key 去重，key 已含 `reason_code` → 自然只剩一条）

**理由**：
- 真降级方案要新做「索引未就绪」全局态 + 补建入口，违背 spec 的 LKG 语义，风险转移到更晚、更难排查的位置；维持现状（三步各报）给出三条红、两条零信息 → 不可接受
- 「前置探测 + 致命」只阻断真正依赖它的链路，出口明确（装 CLI 约 1 分钟）
- 探测与索引共用同一个 `CodeGraphCli` 实例 → 不会出现「探测说可用、索引说版本不匹配」的两套真相

**代价**：
- 探测失败时用户看不到「哪一步失败」，只能看到「一条中文提示 + 安装命令」→ 必须把文案与安装指引写清楚（与决策 9 的安装能力联动）
- `preflight.json` 的 schema 扩两个字段（向后兼容，`#[serde(default)]`）

## 决策 8：审计预算口径改为「本次变更量」+ 跳过整个 `.git/` + 流式读（K-1 / K-2 / K-3 §3.5）

**背景**：

| | 现状 | 证据 |
|---|---|---|
**K-1** | 报错把「整份快照累计已读字节」写在「刚扫到的那个文件」后面，且不报该文件自身多大 | `root_recipe_receipt_store.inc.rs:735-753` 的 `charge_bytes`：`{path}`=越线文件、`{bytes}`=累计、`len` 未进文案 |
**K-2** | `.git/` 被计入预算（唯一跳过条件是 gitignore/info-exclude 忽略，而 `.git` 默认不被忽略，代码里只被**分类**为 `MemberGit`） | `:581-600` 分类 + `:786-791`（仅 `ignores.is_ignored` 才跳过）；codegraph 排除清单**含** `**/.git/`（`aggregate_index/exclude.rs:19`）→ 同仓两套口径 |
**K-3** | 64 MiB 对真实多仓天然偏小；`with_snapshot_budget()` **生产零调用**（硬取 `::DEFAULT` = 20_000 条目 / 64 MiB） | `root_recipe_receipt_types.inc.rs:60-64` + `root_recipe_receipt_auditor.inc.rs:20-24` |

实测体积（本仓，约 1 年 / 113 提交）：`.git` 39 MB（objects 37 MB）+ 源码约 17 MB。对 4-5 年、30+ 提交的老仓：`.git` 300 MB~1 GB + 源码 30~150 MB → **64 MiB 上限的余量 ≈ 0**。

**决定**（owner 决策 1 / 4 / 6 / 7）：

**K-1 文案改法**：新增结构化错误变体 `ProductStoreError::AuditSnapshotBudgetExceeded`（Display 保留现状英文机器行，不破坏既有消费方），携带 `AuditSnapshotBudgetFailure { max_bytes, cumulative_bytes, over_line_path, top_groups, member_summaries }`。分组统计是必须的 —— 文案要承载「哪个大项占得多」就得有分组数据。`over_line_path` 只用于定位（「越线时正在处理的文件」），**不是「这个文件超标」**。

**K-2**：遍历时凡路径任一段为 `.git` → **整棵子树跳过**（不计条目、不计字节、不进快照），与 codegraph 的 `**/.git/` 口径统一。`classify_member_git`（`:581-602`）的 `MemberGit` / `MemberWorktree` 分类**保留**（用于「命令试图改 .git」的判定），只是遍历时不再进入 `.git` 子树。

**K-3（元数据差分法，owner 2026-11-03 定实现方式）**：

```
现在：扫描第 1 遍（读内容算哈希）→ 扫描第 2 遍（读内容算哈希）→ 对比
改后：扫描第 1 遍（只记元数据：路径+大小+mtime，不读内容）
      扫描第 2 遍（只记元数据）
      → 对比两份元数据，找出「大小或 mtime 变了」的文件
      → 只对这些文件读内容算哈希 → 得出变更清单
```

**流式读**：改成边读边累加、边算哈希，超预算立刻停 → 内存峰值 = 块大小（64 KiB 级），不会 OOM（现状是 `std::fs::read` 发生在 `charge_bytes` **之前** → 4 GB 文件先吃 4 GB 内存，40 GB 文件直接 OOM）。

**理由**：
- 预算爆的根因不是「文件多」，是「**读了太多字节**」—— 而读字节是为了算内容哈希 → 只哈希「可能变了的文件」就读得少了
- **仍然扫全仓两遍、仍然对比出所有变更** → 「recipe 写到 allowlist 之外」这个检测能力**完整保留**（`classify_change` 的 `UnknownPath` 分类不依赖全量内容哈希）
- 不用事件驱动（inotify）：跨平台成本高（Windows 无等价机制）+ 可能漏事件（缓冲区溢出）→ 会让审计保证变弱；元数据差分法无新依赖、无漏检风险
- 跳过整个 `.git/` 与 codegraph 的排除口径统一 → 同仓不再有两套口径

**代价**：
- **owner 已确认接受**：平台**检测不到成员仓的 git 提交状态**了；既有守卫测试 `root_recipe_receipt_tests.inc.rs:866-912`（断言 `.git/HEAD` 必须可观测）需按新口径改写
- 元数据差分法理论上可能漏「大小与 mtime 都没变」的改动 —— mtime 为纳秒精度，实际不发生
- 预算语义改变后，「快照总量」不再是上限口径 → `with_snapshot_budget()` 的**生产接线仍不做**（本方案改的是预算语义，不是加配置项；线上下调不再需要）
- 「本批最低要求是文案与指引」已被 owner 的 K-3 决策覆盖 → 本批同时把口径改对

**用户面文案（K-2 落地前的原文 —— 已过时，落地实况见本节末注）**：

> **聚合初始化的预检失败了**
>
> **原因**：平台要对逻辑代码库（LC）做一次**完整文件快照**，用来证明后面的沙箱步骤没有乱写文件。这个快照有 **64 MiB** 上限；当前这个 LC 聚合根里**没有被忽略规则排除掉的内容加起来已经超过 64 MiB**。
>
> **注意：不是某个文件特别大。** 报错里点到名的那个文件只是快照扫到它时刚好越线。
>
> **当前 LC 快照构成（大项排行）**：
> - `pms/.git`：21.0 MB
> - `pms/target`：45.2 MB
> - 其余源码与未忽略产物：约 0.3 MB
>
> **怎么做（约 2 分钟，在成员仓目录下做）**
> 1. 看谁占的：`du -sh .git target node_modules dist 2>/dev/null | sort -h`（每个成员仓都跑一次）
> 2. 把**不影响你工作**的大目录排除掉 —— 用 `.git/info/exclude`：
>    ```bash
>    cd <成员仓>
>    printf '/target/\n/.git/\n' >> .git/info/exclude
>    ```
>    **不要改 `.gitignore`** —— 它是被跟踪文件，改它会撞平台的写入范围门禁。
> 3. 回平台点「重新初始化」

「不要改 `.gitignore`」在本仓成立（已核实三条证据）：平台代码注释已定此规则（`evidence_token.rs:12-18`、`:304-306`）+ 对照实现 `ensure_repo_exclude()` 把忽略行写进 `<common-gitdir>/info/exclude`；改 `.gitignore` 会让成员仓变 dirty（`registration_coordinator.inc.rs:631` → `NeedsAttention, "dirty_worktree"`）且 dirty 进入 LC 规划上下文指纹；审计自己会读 `.git/info/exclude`（`root_recipe_receipt_gitignore.inc.rs:242-270`、`:284-300`，支持 worktree gitfile 解析 `gitdir:`）→ 写进 exclude 对预算是**真生效的**。

**用户侧指引要去掉 `/.git/` 那一行**：K-2 之后平台已不看 `.git`（§7.1 连锁要求）。

**⇒ 落地实况（2026-11-03 复核）**：生产文案已在 `src/product/logical_codebase/failure_copy_table.inc.rs:319` 改写为「清理不再需要的大目录（如构建产物、依赖缓存），或把大目录写进 `<LC根>/.git/info/exclude`。不要改 .gitignore（它会进仓改变项目契约）」—— **不含 `/.git/` 行、也不含 `.git` 体积排行**。⇒ 上面引用的整块文本是 **K-2 落地前的原文**，仅作决策背景保留，**不是现行文案**。

## 决策 9：provider / codegraph 的安装与更新放在能力探针处，异步作业 + 手动命令兜底，不自动 sudo（§3.6）

**背景**：冷启动的失败面会指向「没安装 / 版本太低」→ 但平台不提供安装能力，用户只能自己去查文档。owner 2026-11-03 裁定把安装/更新按钮放到**能力探针那块**（决策 10、11、12、15、16）。

**决定**：

**安装源**（核实自 `Cadence-devbox/devbox/Dockerfile` + `versions.env`）：

| provider | npm 包 | 命令名 | 镜像 / 来源 |
|---|---|---|---|
Claude Code | `@anthropic-ai/claude-code` | `claude` | npm（阿里镜像） |
Codex | `@openai/codex` | `codex` | npm（阿里镜像） |
Pi | `@earendil-works/pi-coding-agent` | `pi` | npm（阿里镜像） |
**Kimi Code** | **不是 npm 包** | `kimi` | **官方安装脚本** `https://code.kimi.com/kimi-code/install.sh` |
codegraph | `@colbymchenry/codegraph` | `codegraph` | npm（阿里镜像） |

**npm 安装形态**（照抄 devbox 已验证做法）：`npm install -g --prefix <可写前缀> --omit=optional --no-audit --no-fund "<pkg>@<版本>"`，环境带 `npm_config_registry`（默认 `https://registry.npmmirror.com`，可被 `NPM_REGISTRY` 覆盖）。**必须一个包一次安装**，不得合并成一次 `npm install -g a b c d`（devbox 2026-10-28 实测：五个包放同一次安装会长时间无进展 >20 分钟、磁盘零增长）。

**按钮显示规则**（放在能力探针那块）：

| 探针结果 | 本地状态 | 显示 |
|---|---|---|
未安装（`command_missing`） | 无 | **【安装】** 按钮 + 手动命令 |
已安装，探针失败（能力不支持 / 未核验） | 有 | 不显示安装；显示探针失败原因 + 「核验」按钮（若已核验则显示「更新」） |
已安装，版本落后 | 有 | **【更新】** 按钮 + 当前版本 vs 最新版本 |
已安装，版本最新 | 有 | 不显示任何按钮 |

**「有新版」判定**：npm registry 的 `dist-tags.latest` + **精确 semver 比较**（`major.minor.patch` + 可选预发布后缀；预发布版视为**低于**对应正式版）。**Kimi 例外**：非 npm 包 → 无法查 registry 最新版本 → 只在**未安装**时显示「安装」，**不显示「更新」**。缓存：**durable 24 小时**（包名 → latest 版本 + 查询时间戳）。

**异步安装作业**（安装可能几分钟 → 不阻塞 HTTP）：

```
POST .../providers/{provider}/install     → 立即返回 job_id
GET  .../providers/install/{job_id}        → 轮询进度与结果
```

作业状态 durable → 页面刷新不丢进度。流程：记 `running` → 跑命令（npm 三家+codegraph / Kimi 官方脚本一家）→ 成功后**重新探测**该 provider 的版本与可用性（`provider_health` 的 refresh）→ 记 `completed` + 结果。

**失败分类与兜底**：

| 失败类型 | 判定 | 提示 |
|---|---|---|
**权限不足** | stderr 含 `EACCES` / `permission denied` / `sudo` 提示 | 「装不上（权限不足）。请用管理员权限运行：`<命令>`」 |
**网络 / 镜像不可达** | stderr 含 `ETIMEDOUT` / `ECONNREFUSED` / `ENOTFOUND` / `E404`（registry） | 「装不上（网络问题）。请检查网络或换镜像源后手动运行：`<命令>`」 |
**包名不存在** | registry 返回 `E404` 且响应体含 `Not found` | 「装不上（找不到这个包）。请手动安装：`<命令>`」 |
**其它** | 其它非零退出 | 「安装失败：`<stderr 摘要>`」 |

兜底命令**一律给用户**：npm 三家 + codegraph → `npm install -g --registry=<registry> <pkg>`；Kimi → `curl -fsSL https://code.kimi.com/kimi-code/install.sh | bash`。**不自动 sudo**（owner 裁定：平台不替用户提权；非交互环境下 sudo 密码提示会卡住进程；命令文本里带 `sudo` 前缀由用户自己决定）。Kimi 走远程 shell 脚本，风险等级与 npm 包不同 → **UI 上必须明确告知「将执行来自 code.kimi.com 的安装脚本」**。

**与既有能力的关系**：`provider_health`（命令可用性 + 版本探测）**复用**（安装后重新探测）；`provider_availability_gate` **不变**；`/api/providers/status` / `/api/providers/recheck` **复用**（安装完成的「重新检测」按钮走既有端点）；`providers.rs:83-107` 的 `install_hint` **扩充**（补 npm 包名 / Kimi 脚本地址 / 阿里镜像）；`provider_capability_revalidation` 的单家核验 **不变**（**安装 ≠ 核验**，装完还要探针）。

**理由**：
- 「没安装」是失败面最常见的用户侧原因 → 把它做成一个按钮 + 一条命令，比只给文案有效
- 装 CLI 要几分钟 → 必须异步；durable 作业状态 → 页面刷新不丢进度
- 阿里镜像默认 + `NPM_REGISTRY` 覆盖：阿里镜像主要服务国内，写死会让海外用户点「安装」必然失败
- 精确 semver 比较 + 24 小时 durable 缓存：每次打开能力卡都查一遍 registry 不合适

**代价**：
- 引入 npm registry 查询（复用既有 `reqwest`，带 json + rustls）
- 四个包在阿里镜像上的最新版本是否与官方一致**未验证**（技术方案 §8 未验证项 1）；npm 全局安装到 `--prefix` 后命令是否在 PATH 上**未验证**（未验证项 2）；Kimi 官方脚本在国内的可达性**未验证**（未验证项 3）；npm 安装耗时**未验证**（未验证项 4，决定异步作业默认超时与进度提示粒度）
- 「更新」按钮依赖 registry 数据的时效性 → 缓存过期时可能漏报新版本（24 小时窗口内可接受）

## 决策 10：B-5 残项 R1 / R2 都不做，保持原样（§3.7）

**背景**：B-5（登记预检耦合）主体已落地并归档（`openspec/changes/archive/2026-11-02-fix-aggregate-lc-registration-gates/`，commit `5faa802` 是 HEAD `827a990` 的祖先），只剩两项残项：

- **R1**：「重新初始化（聚合）」没有写入预览也没有自动备份 → owner 手工改过的 `AGENTS.md` / `.claude/rules/*` 会被四条命令按默认模板重写，**无预览、无备份、无回退入口**
- **R2**：workbench 仍吞 `details`（`IssueLifecycleWorkbenchParts.tsx:628-630`）；bootstrap action 冲突把详情放 `details.detail` 而登记路径用 `details.reason` → 口径不一致

**决定**：**R1 与 R2 都不做，保持原样**（owner 裁定 2026-11-03）。这两项**不是缺陷，是已知取舍**。实施时**不得**顺手把它们做进来（超出本方案范围），也不得因为「反正改到同一片代码」而扩大范围。

**理由**：owner 已明确裁定；把它们做进来会引入写入预览 / 备份 / 回退入口这一整套新语义（属另一块），且与本 change 的「让冷启动一次跑通」目标无关。

**代价**（明确记录，供后续知情）：
- R1 不做 → owner 手工改过的 `AGENTS.md` / `CLAUDE.md` / `.claude/rules/*` 在点「重新初始化（聚合）」时仍会被四条命令按默认模板重写，无预览、无备份、无回退入口
- R2 不做 → workbench 侧动作失败仍只显示通用英文 message

## 明确不改的部分

| 项 | 原因 |
|---|---|
`with_snapshot_budget()` 的生产接线 | 本方案改的是**预算语义**（变更量），不是加配置项；线上下调不再需要 |
provider 能力核验的「不点击不运行任何探针」契约 | 不变（本方案只做引导，不自动触发） |
`codegraph_missing` 在 rebuild/sync 下的 Degraded 语义 | 不变（只改冷启动路径） |
`aggregate_root_ownership_conflict` 兼容映射臂 | 不删（历史兼容；已无生产者） |
「已初始化」判定文件（`AGENTS.md` + `.claude/rules/language.md`） | **不改**（owner 裁定：用户自己决定要不要重新初始化） |
`MachineSkills` 是否跑 provider | 不改（它是确定性代码） |
自动重写 / 人工返修账本合并 | 不涉及（属块1） |
`Cadence-skills` 上游仓库的完整性校验 | 跨仓事项，本方案用「machine_skills 后校验链接层」兜住 |
一键核验全部（票3） | 不在本批（本批只做单 provider 核验 + 指引）；只定义契约边界；块3 票3 复用本 change 的安装/更新机制（REQ-PCI-*）与作业机制 |
B-5 残项 R1 / R2 | owner 裁定不做，保持原样 |
provider 适配层、网关信封、流式输入结构、工具策略 | 不改 |
运行中不许改 payload 的 fail-closed 语义 | 不改（属块1） |

## owner 决策记录（2026-11-02 / 2026-11-03 已全部拍板）

| # | 决策 | 结论 |
|---|---|---|
1 | K-2 `.git/` 跳过范围 | **整个 `.git/` 跳过**（owner 2026-11-02）→ 2026-11-03 **确认接受代价**：平台检测不到成员仓的 git 提交状态；一个既有测试（`root_recipe_receipt_tests.inc.rs:866-912`）按新口径改写 |
2 | B-4 codegraph 致命 vs 降级 | **维持致命 + 前置探测 + 只报一次** |
3 | Pi 优先的 skills 假成功 | **无需改动** —— `Cadence-skills` 的 `install.sh:401-404` 明确 `pi` 与 `Kimi Code` 都用 `~/.agents/skills`（= Aria `paths.rs:22` 的 `shared_skills_root`），pi/kimi 读得到 |
4 | K-3 预算模型 | **立项 + 与本批一起实施**（只统计本次变更量）；**2026-11-03 定实现方式：元数据差分法**（不用 inotify 事件驱动） |
5 | 「已初始化」判定文件 | **不改**（用户自己决定要不要重新初始化） |
6 | 超大文件先读后计费 | **改**（流式读，超了立刻停） |
7 | K-3 实现方式 | **元数据差分法**（扫描两遍只记「路径+大小+mtime」，对比元数据找出变更文件，只对变更文件读内容算哈希）；仍扫全仓两遍 → 「写到 allowlist 外」的检测能力**完整保留** |
8 | B-5 残项（R1 / R2） | **都不做，保持原样** → 本 change 只覆盖 B-1 / B-2 / B-3 / B-4 / K-1 / K-2 / K-3 七条 + provider 安装/更新能力 |
9 | B-2 失败面指向的按钮 | **单个「核验 Pi」**；一键核验全部留在**块3**，块2 上线时那个按钮还不存在，文案**不得**指向它 |
10 | provider 安装/更新按钮的位置 | 放到**能力探针那块**：探针不通过 → 显示「安装」；本地已有 → 不显示安装；版本落后 → 显示「更新」。块3 票3 复用同一套 |
11 | 「有新版」怎么判断 | **精确比较**（semver）+ 查 npm registry 的 `dist-tags.latest` |
12 | 安装执行方式 | **A + B 兜底**：平台先跑安装；失败（权限 / 网络）→ 把**手动命令**给用户。**不自动 sudo** |
13 | npm 镜像源 | 默认**阿里镜像**（`registry.npmmirror.com`），允许环境变量 `NPM_REGISTRY` 覆盖 |
14 | 「查最新版本」的缓存 | **durable 缓存 24 小时** |
15 | codegraph 安装 | 同样纳入安装按钮机制，**用阿里镜像** |
16 | Kimi Code 的安装源 | **官方脚本** `https://code.kimi.com/kimi-code/install.sh`；**UI 上必须明确告知「将执行来自 code.kimi.com 的安装脚本」**，失败给手动命令 |

## 决策带来的连锁要求

| 要求 | 原因 | 落地位置 |
|---|---|---|
选择器的 L2 判定必须与 gateway 共用同一函数 | 否则两处各写一套，迟早漂移 | 技术方案 §3.1.2 / 决策 2 |
不得自动触发 capability 核验 | 与 `provider_capability_revalidate.rs:1-10` 的契约直接冲突 | 技术方案 §3.1.4 / 决策 3 |
provider 必须走 request 不能走 driver 字段 | driver 是 state 级单例，会跨 LC 串味 | 技术方案 §3.1.6 / 决策 4 |
K-2 会让平台看不到成员仓 git 状态 | owner 已确认接受 | 技术方案 §3.5.3 / 决策 8 |
K-3 的元数据差分法必须保留全仓两遍扫描 | 「写到 allowlist 外」的检测能力依赖它 | 技术方案 §3.5.4 / 决策 8 |
流式读与 K-3 在同一处改 | 都作用在「读什么」 | 技术方案 §3.5.5 / 决策 8 |
用户侧指引要去掉 `/.git/` 那一行 | K-2 后平台已不看 `.git` | 技术方案 §3.5.7 / 决策 8 |
B-5 残项不得顺手做进来 | owner 裁定不做 | 技术方案 §3.7 / 决策 10 |
B-2 文案只指单个「核验 X」 | 一键在块3，块2 上线时不存在 | 技术方案 §3.2.3 / 决策 5 |
npm 必须一个包一次装 | devbox 实测：五个包放同一次 `npm install` 会长时间无进展（>20 分钟、磁盘零增长） | 技术方案 §3.6.2 / 决策 9 |
Kimi 走官方脚本，UI 必须告知来源 | 远程 shell 脚本与 npm 包风险等级不同 | 技术方案 §3.6.1 / 决策 9 |
安装失败不自动 sudo | 平台不替用户提权；非交互环境 sudo 会卡住 | 技术方案 §3.6.7 / 决策 9 |
安装完成后要重新探测版本与可用性 | 装完不等于能用 | 技术方案 §3.6.6 / 决策 9 |
块3 票3 复用同一套安装/更新机制与作业机制 | 避免两套 | 技术方案 §3.6.5 / §3.6.6 / 决策 9 |
新 durable 字段一律 `#[serde(default, skip_serializing_if = "Option::is_none")]` | 老记录必须仍能反序列化 | 技术方案 §5 |
