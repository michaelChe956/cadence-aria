# Tasks: fix-aggregate-cold-start-and-audit-budget

> **收口注记（2026-11-03）**：本 change 已于 2026-11-03 归档，delta 已同步进主 specs
> （5 份 capability；含新建 `logical-codebase-audit-budget` 与 `provider-cli-installation`；
> `validate --specs` 78 passed / 0 failed）。
> 勾选口径与逐条文件:行级证据见 `cadence/notes/2026-11-03_收口_行数登记表改动清单_v1.0.md` §M.1。
>
> **剩余未勾 6 条及原因**（2026-11-04 补记：12.1 已核验补勾，「待统一绿测」类现为 **0** 条；出处 `cadence/notes/2026-11-04_待绿测5条核验补勾_v1.0.md` §1.5）：**2026-11-07 补记：11.10 / 12.3 / 12.4 / 12.5 / 12.6 / 12.7 已实测通过并补勾（本文件 §11 / §12 证据行）；本 change 剩余未勾 0 条**
> · **12.1** —— ✅ 2026-11-04 收口统一绿测核验后补勾完毕，不再计入未勾（同上 §1.5）
> · **12.3–12.7**（5 条）真机验收 —— **⏳ 待实做（Main 真机验收批次：受控环境操控 + 浏览器实测；逐条做、逐条勾、逐条留证据）**（**2026-11-07 补记：12.3 / 12.4 / 12.5 / 12.6 / 12.7 已实测通过并补勾**）
>   ⇒ 本机四家 provider（`pi` / `codex` / `kimi` / `claude`）+ `codegraph` **均已安装**（`command -v` 实测：前四家在 `/home/michaelche/.local/share/fnm/aliases/default/bin/` 与 `/home/michaelche/.kimi-code/bin/`，`codegraph` 同 fnm 路径、全局包 `@colbymchenry/codegraph@1.6.2`）
>   ⇒ **不是环境不具备，是尚未执行**；下列 5 条走**受控环境操控**（临时改 PATH / 临时造目录，**每条做完复原**）
>   逐条怎么做：
>   · **12.3**「只装 pi 的机器」 ⇒ 临时把 `PATH` 掩到**只暴露 `pi`**（用临时 PATH 覆盖 + 软链白名单目录），跑聚合初始化 → 中文指引 +「核验 Pi」（**做完复原 PATH**）（✅ 2026-11-07 已勾）
>   · **12.4**「全程只用 Pi 的三个 turn」 ⇒ 用**真实 `pi`**（已装）跑满三个 turn 并核对 `input.provider == "pi"`；**需确认 `pi` 已登录** —— 若未登录，则该条明确记为「**需登录态**」（登录态是唯一可能真缺的外部前提，其余均不缺）（✅ 2026-11-07 已勾）
>   · **12.5**「移除 pi 后重试」 ⇒ 临时从 `PATH` **掩掉 `pi`**（其余三家保留）后重试，逐家点名走 fallback（**做完复原 PATH**）（✅ 2026-11-07 已勾）
>   · **12.6**「无 codegraph 的干净机器」 ⇒ 临时把 `codegraph` 从 `PATH` **移开**后初始化 → **一条**中文提示 + 装好重试（**做完复原 PATH**）（✅ 2026-11-07 已勾）
>   · **12.7**「成员仓有大构建产物」 ⇒ **造一个真的大产物目录**（本地生成足量构建产物塞进成员仓）→ 触发行数/体积预检失败面：排行呈现 + 两条出路（✅ 2026-11-07 已勾）
> · **11.10** 聚合初始化失败面（`BootstrapFailureLine.tsx`）仍无【安装】按钮，只有【重新检测】；（✅ 2026-11-07 已勾 —— owner 真机实测通过；以下 2026-11-04 旧口径注记按原文保留存证，不代表当前状态）
>   registration wizard 面**已有** codegraph 安装按钮
>   （`LogicalCodebaseRegistrationWizard.tsx:543-552` ⇒ `ProviderInstallActionButton target="codegraph"`）
>   ⇒ **聚合初始化面的【安装】按钮待立项补齐**，本轮不勾
>   ⇒ **2026-11-04 口径补记（owner 真机实测后裁定，只补注记、不动勾选框、不改条目原文）**：
>     · owner 裁定「**codegraph 不展示在「Provider 能力核验」面板**」⇒ 已实施并落地：
>       该面板内 codegraph 的渲染行已删除（`ProviderCapabilityCard.tsx`），面板只剩四家 provider
>       （Claude Code / Codex / Pi / Kimi Code）的【未探测 + 核验按钮】；
>       实施依据＝codegraph 本就没有 capability 探针，只按安装面呈现 ⇒ 移出后该面板语义更纯
>     · **本次移出【不影响】11.10 的安装机制本身**：冷启动失败面 / 通知面的【安装】入口
>       （`CodegraphInstallEntry.tsx`）与登记向导的 codegraph 前置引导**均未动**，
>       API 字段与 11.10 的「codegraph 纳入同一安装机制（阿里镜像）」也未动
>     · ⇒ **11.10 仍保持未勾**：其「安装按钮出现在 codegraph 缺失的失败面上」这一条
>       （聚合初始化失败面 `BootstrapFailureLine.tsx` 仍只有【重新检测】）**尚未落地**，待立项补齐（**2026-11-07 已被上方 11.10 补勾取代**）
>     · 证据：`cadence/notes/2026-11-04_前端_一键核验按钮位置与添加代码库布局说明_v1.0.md`、
>       `cadence/notes/2026-11-04_真机验收发现_冷启动推进与聚合索引与codegraph与布局说明_v1.0.md`

> owner 决策已全部拍板；技术方案见 `cadence/designs/2026-11-02_技术方案_块2_聚合冷启动与审计预算_v1.0.md`（owner 2026-11-02 审阅通过，2026-11-03 补决策）。
> 精确文件、命令、测试与提交步骤由 `writing-plans` 写入 `cadence/plans/`；本文件只记高层工作包，每个工作包映射到相关 requirement。

## 1. provider 选择器：候选顺序与两层判据

映射 requirement：REQ-BOOT-07、REQ-PCR-01

- [x] 1.1 新建 `src/product/logical_codebase/provider_selection.rs`，钉定候选顺序常量 `AGGREGATE_PROVIDER_CANDIDATE_ORDER = [Pi, KimiCode, ClaudeCode, Codex]`，并把「只判 `PlanningReadOnly`」写成选择器的唯一动作口径
- [x] 1.2 实现 L1（命令可用）：读 provider health 快照的 `available` + 六值 `reason_code`；实现「仅当 `generation == 0` / `degraded()` / `checked_at` 早于 `now - 10min` 才 await 一次 `refresh()`」的条件刷新
- [x] 1.3 实现 L2（能力已核验）：复用 `ProviderCapabilityStore::for_lc(paths, lc_id).get(...)` 的 `action_matrix.row(&PlanningReadOnly).launch`
- [x] 1.4 在 `provider_capability_store.rs` 抽出只读方法 `launch_cell_state(project_id, provider, action) -> LaunchCellState`（`Confirmed` / `Unverified` / `Denied` / `EvidenceStale`），并让 `production_policy_resolvers.rs` 的 `require_supported` 改调它、**保留既有错误前缀与文案形态**
- [x] 1.5 判定「无可用候选」时**不创建 operation**，分别返回 `aggregate_initialization_no_provider_available` / `aggregate_initialization_provider_needs_verification` / `aggregate_initialization_provider_capability_denied` / `aggregate_initialization_provider_health_degraded`
- [x] 1.6 用例：四家皆可用且已核验 → 选 `pi`；`pi` 命令缺失 → 跳过并记原因；四家皆可用但都未核验 → 需求核验且 `request_count == 0`（不产生 operation）；`pi`=`Denied{reason}` 而 `kimi`=`Unknown` → 仍选到 `kimi` 且候选表如实区分两种状态；`degraded()==true` → 不创建 operation

## 2. 选择器接线与冻结落账

映射 requirement：REQ-BOOT-07

- [x] 2.1 在 `src/web/handlers/aggregate_initialization/handlers.inc.rs` 的 dependencies 与 manifest 就位之后插入选择（顺序本身是契约），把 `provider: "claude_code".to_string()` 改为 `selection.selected.clone()`
- [x] 2.2 `ensure_recipe_trust_ready` 增参 `selected: &ProviderName`，准入清单由 `&[ProviderName::ClaudeCode]` 改为 `std::slice::from_ref(selected)`（trust 语义后果：Pi/ClaudeCode → 门 no-op；KimiCode/Codex → 走既有 `aggregate_initialization_trust_waiting`）
- [x] 2.3 新增 durable 字段 `operation.provider_selection: Option<AggregateProviderSelection>`（`#[serde(default, skip_serializing_if = "Option::is_none")]`），含四家候选表 + 判定依据 + 规则版本；候选**按 owner 顺序**排列，不按结果重排
- [x] 2.4 `AggregateInitializationOperationDto` 增 `provider_selection`，前端「聚合初始化」卡片显示一行摘要（本次使用 X；跳过：Y（未核验）、Z（未安装））
- [x] 2.5 用例：operation JSON 里 `input.provider == "pi"`、`provider_selection.candidates` 四家齐全且各有原因

## 3. provider 走 request：driver 参数化与 clean cutover

映射 requirement：REQ-BOOT-07

- [x] 3.1 `AggregateProviderTurnRequest` 增 `provider: &'a ProviderName`；`run_provider_turn` 从 durable 操作记录解出 provider（`provider_from_wire`，未知串 fail-closed）后传入 → 「重试不换人」成为结构性保证
- [x] 3.2 `coordinator_provider_turn.inc.rs` 改为按 `ProviderName` 构造 `ProviderRef`（用 `ProviderRef::from_provider_name`，四家显式映射、Fake fail-closed、**无 `_ =>` 静默回退**），并**保留 admission 预检**（`with_provider` / `with_provider_and_admission`）
- [x] 3.3 删除旧的 `claude_code` / `claude_code_with_admission` 构造器（clean cutover），迁移全部调用点（含测试夹具）；`production_dependencies.inc.rs` 改调新构造器
- [x] 3.4 删过期注释（`coordinator_provider_turn.inc.rs:41-43` 的 Codex 路由阻断表述、`provider_gateway.rs:467-469`），避免下一个改这块的人再次得出「aggregate 只能用 Claude」
- [x] 3.5 改写约 5 处「必须保持 Claude」的断言（`coordinator_tests_contract.inc.rs:107-110`、`coordinator_tests_profile_trust.inc.rs:637-641` 等）为断言「aggregate recipe provider == operation.input.provider」（改口径不改语义）
- [x] 3.6 用例：begin 后把 health 改成只有 `claude_code` 可用再 `execute_remaining` → 仍用 `pi`；未知 provider 名 fail-closed

## 4. 能力状态可机读与拒绝分类

映射 requirement：REQ-PCR-01

- [x] 4.1 在 `provider_gateway.rs` 的稳定码常量旁新增稳定词表函数 `capability_launch_state_token(&ProviderCapabilityEvidence) -> &'static str`（`confirmed` / `denied` / `unknown`），**禁止依赖 Debug 输出**
- [x] 4.2 `production_policy_resolvers.rs` 的 launch 分格门改为按态分流（`Denied` 额外带 reason）
- [x] 4.3 新增分类器 `classify_capability_launch_rejection(detail) -> Option<CapabilityLaunchRejection>`（`Unverified` / `Denied{reason}` / `EvidenceStale`），三分支来源分别是「记录缺失」「分格三态」「snapshot 漂移」
- [x] 4.4 不匹配时**保持既有 `reason_code` 与 `action` 逐字节相同**（零回归锁）
- [x] 4.5 用例：`"... launch cell is unknown"` → `Unverified`；`"... launch cell is denied; reason: ..."` → `Denied`；`"... capability record missing"` → `Unverified`；`"capability snapshot mismatch"` → `EvidenceStale`；无关串 → `None` 且产出与改造前逐字节相同

## 5. 失败面三态分流与逐家点名

映射 requirement：REQ-PCR-02、REQ-BOOT-08

- [x] 5.1 `error_record()` 对 `ProviderTurn{reason}` 先过分类器，按三态产出**稳定 `reason_code`**（`aggregate_provider_capability_unverified` / `_denied` / `_evidence_stale`）；**中文主文案由片2 的 `failure_copy` 在投影 / DTO 层渲染**，`error_record()` MUST NOT 产出中文（片1 不得反向依赖片2，否则破坏依赖图、零回归锁无法逐字节断言）——依据 `spec.md:111`（同一文案来源、MUST NOT 各处分别维护同义文案）+ 块2 Plan **D-5**
- [x] 5.2 `details.revalidate` 携带 `{ ui_path, button_label, method, path, body, expect }`；`Denied` 分支额外带 `details.fallback_provider`（指向优先顺序中的下一家）
- [x] 5.3 指引只指向**单个**「核验 X」入口，**不得**指向块3 的一键核验全部（块2 上线时该按钮不存在）
- [x] 5.4 「需要核验」「实测拒绝」「无可用」三类失败的 `details.candidates` 四家齐全、按 owner 顺序，每家含 `{provider, display_name, cli_available, cli_reason_code, capability_state, skipped_reason, action_hint, warning}`，`action_hint` 必须是可执行动作；`warning: Option<String>` 承载「已知 CLI 命令不可解析」等提示（`None` = 未发现异常，**不跳过也不改排序**，`#[serde(default, skip_serializing_if = "Option::is_none")]`）——`warning` 依据块2 Plan **D-14** / 风险项 **R11**
- [x] 5.5 机器码降级：原始 detail 走既有 `details.stderr_summary`（**不改 DTO 形状**），主文案不得出现机器码
- [x] 5.6 三类分诊落地：环境类（`command_missing` / `version_too_low`）复用 `providers.rs:81-110` 的 `install_hint` 原文并指向「重新检测」（`POST /api/providers/recheck`）；产品缺陷类（`degraded()` / `evidence_stale`）给错误原文 + 「重试」；重试类（`timeout` / `non_zero_exit` / trust waiting）给「重试」按钮

## 6. 失败面文案单一来源（WP-A）

映射 requirement：REQ-BOOT-08、REQ-BOOT-10、REQ-BOOT-11、REQ-BOOT-12、REQ-BOOT-13

- [x] 6.1 新建 `src/product/logical_codebase/failure_copy.rs`，**一次写全**六个布局规则与七个对外稳定码（`aggregate_root_is_git` / `aggregate_root_member_outside_root` / `aggregate_root_member_symlink_escape` / `aggregate_root_nested_worktree` / `aggregate_root_overlap` / `aggregate_index_layout_unsupported` / `aggregate_index_member_coverage_failed`）及其内部反查码；`FailureCopy { title_zh: String, reason_zh: String, action_zh: String, triage: FailureTriage, machine_line: String }` 五字段全部必填、非Option、非空，`FailureTriage { Environment, ProductDefect, RetryOnly }`；`describe(FailureInput) -> FailureCopy`（纯函数、无 I/O、可单测）；码表外未知码由同一来源给中性三段兜底（不 panic、不伪装成七个布局码）；块3 MUST NOT 追加写本文件
- [x] 6.2 `bootstrap_projector.inc.rs:334-347` 与 `:575-589` **共用同一函数**渲染 detail（禁止第二份拼接）；链A 经 `BootstrapFailure.copy` 下发同源成品（初始化两处 + 索引失败调用点；无 action 的布局事实仍命中已知模板，不降级为通用重试）
- [x] 6.3 `BootstrapFailure` 增 `#[serde(default)] pub triage: Option<String>;`（值只由 `copy.triage` 序列化派生，不另查表）与 `#[serde(default)] pub copy: Option<FailureCopy>;`；新产生的已知失败（尤其七个布局码）必须为 `Some(copy)`
- [x] 6.4 链B：`AggregateRootPreflightError::copy()` 只做输入适配并委派同一 `describe`（不自持表）；`src/web/handlers/support_parts/product_store_error.inc.rs:9-31` 的 `aggregate_root_api_error` 保留既有 code/message/details.reason，追加 `error.copy()` 成品到 `details.copy`
- [x] 6.5 Rust DTO 逐字段搬运：`src/web/types/logical_codebase_bootstrap.rs` 新增 `FailureCopyDto`、`BootstrapFailureDto.copy` 与 `From<&Projection>`（product 加字段不会自动透出，必须手工镜像五字段）
- [x] 6.6 TS 同构：`web/src/api/types/logical-codebase-bootstrap.ts` 增 `FailureCopyDto` 与 `BootstrapFailureDto.copy`；五字段必填，`reason_zh` MUST NOT 改成可选
- [x] 6.7 前端 `LogicalCodebaseBootstrapCard.tsx` 主行渲染三段成品与 `failure.detail`，`原因 {reason_code}` 降级为折叠区，`triage` 存在时渲染中文标签
- [x] 6.8 验收：`grep -c "no stderr summary" src/` → **0**；最早记录（`stderr_summary: None` + `action` 空）也有兜底中文文案且不含 `no stderr summary`；七个布局码逐条命中具体规则依据与可执行动作；Rust DTO、`details.copy` 与 TS 五字段同构，缺 `reason_zh` 或缺 copy 不算通过

## 7. 失败面结构层与脱敏（WP-B）

映射 requirement：REQ-BOOT-13

- [x] 7.1 新建 `src/cross_cutting/diagnostic_redaction.rs`（抽自 `src/product/repository_store/initializer.rs:433-475`），把 `sanitize_and_truncate` + `redact_sensitive_assignments` 提升为公共函数
- [x] 7.2 补脱敏规则：`Bearer <token>`、`Authorization: *`、`sk-[A-Za-z0-9_-]{16,}`、`ghp_*`、`xoxb-*`、`AKIA[0-9A-Z]{16}`、`--api-key <v>` / `--token <v>`；身份类路径：`$HOME` → `~`、聚合根绝对前缀 → `<聚合根>`，成员仓内相对路径保留；命令名/文件名/错误码保留
- [x] 7.3 截断：`4096` 字节（与 `AGGREGATE_TURN_OUTPUT_LIMIT` 同值）+ 40 行上限，超出标 `…[truncated]`
- [x] 7.4 `aggregate_initialization.rs` 增 `cause: Option<String>`（`#[serde(default, skip_serializing_if = "Option::is_none")]`）；`error_record()` 按变体填 `stderr_summary` 与 `cause`
- [x] 7.5 失败面结构与成因标注（依据 **REQ-BOOT-13** + 块2 Plan §14 **D-8/D-9**）：失败面 **MUST NOT** 出现 `no stderr summary` 这类无信息文案；进入失败面的原始输出须先**脱敏 + 截断标注**（凭据类 → `[REDACTED]`、home 前缀 → `~`、聚合根绝对前缀 → `<聚合根>`；上限 4096 字节 / 40 行，超出以 `…[truncated]` 结尾）；**可判定的成因 SHALL 记为结构化 `cause`**（`skills_unavailable_*` / `skills_update_failed_*` / `skills_sync_failed_*` / `skills_digest_failed` 等族），不可判定时 SHALL 诚实标为 `unknown`，并列出可能的候选供用户自查（MUST NOT 把候选当结论）、MUST NOT 编造成因；机制形态 = **新增 `*Diagnosed` 变体 + 结构化 `cause`**（`ProviderTurnDiagnosed { step, reason, retryable, provider, diagnostic: TurnDiagnostic }`、`SkillsPreparationDiagnosed { reason, retryable, cause }`），旧变体 `ProviderTurn` / `SkillsPreparation` / `Preflight` 形状一字不动，**MUST NOT** 要求每个既有枚举变体都带 `diagnostic` 字段（D-9：`Preflight` 20 处字面量跨 5 文件、块2 内无消费者，不加字段也不加变体）；另新增 `EnvironmentPrecondition { code, reason, retryable }`（`code` 即最终 `reason_code`）
- [x] 7.6 `coordinator_provider_turn.inc.rs` 的 6 条丢弃输出分支补 `output.summary()`（cancelled / timed out / permission timeout / status failed / status aborted / stream closed），并设对应 `cause`
- [x] 7.7 启动期失败：底层是 `ProviderAdapterError` 时取 `code` / `stderr` / `stdout` → `cause = code.as_str()`、`diagnostic.output = sanitize(...)`
- [x] 7.8 `SkillsPreparation` 保留结构化 stage（`skills_unavailable_{stage}` / `skills_update_failed_*` / `skills_sync_failed_{layer}`），MUST NOT 把这类确定性失败归因为「provider 未登录」
- [x] 7.9 `coordinator_preflight.inc.rs` 增加「聚合根可写」探测（`canonical_root` 下 `create` 一个 `.aria-probe-<nonce>` 再删）→ `EnvironmentPrecondition { code: "aggregate_root_not_writable" }`
- [x] 7.10 `coordinator_lifecycle.inc.rs` 在 machine_skills 成功后校验四条命令的链接层（`shared/codex/claude` 三层的 `~/.claude/skills/{pre-check,rule-config,mcp-configuration,project-rules-examples}/SKILL.md`）→ 缺失为 `EnvironmentPrecondition { code: "skills_commands_missing" }`
- [x] 7.11 `src/web/handlers/dto/aggregate_initialization.rs` 的 `details` 增 `"cause"`；`message` 改为 `describe(&record).title_zh`（原文案挪到 `details.action`）
- [x] 7.12 用例：脱敏单测（`Authorization: Bearer …` + `ANTHROPIC_API_KEY=sk-ant-…` + `/home/alice/work/root/x` → 不含明文、home 前缀变 `~`）；长度单测（输入 100 KB → 以 `…[truncated]` 结尾且长度 ≤ 4096 + 15）

## 8. codegraph 前置探测与只报一次

映射 requirement：REQ-BOOT-09、REQ-IND-05

- [x] 8.1 新增 `AggregateEnvironmentProbe` trait（`ensure_codegraph() -> Result<String, AggregateInitializationError>`），生产实现包一层 `CodeGraphCli`，并**用与索引构建同一份 CLI 配置**装配（runner + executable 字面量一致）——依据 `spec.md:190`「探测 SHALL 与索引构建使用**同一份** CLI 配置」+ 块2 Plan **D-16**（`CodeGraphCli` 无 `Clone`、索引侧按值取所有权，故探针按同一份配置自建；**MUST NOT** 要求「同一个 `CodeGraphCli` 实例」）
- [x] 8.2 探测落点：`advance_remaining` 开头（`machine_skills` 之前、第一个 provider turn 之前），失败时 `persist_execution_failure(..., step: None, ...)` → **不把任何 step 标 Failed**
- [x] 8.3 `AggregatePreflightSnapshot` 增 `codegraph_available: bool` + `codegraph_version: Option<String>`（均 `#[serde(default)]`），并写进 `preflight.json`
- [x] 8.4 三处码统一：`aggregate_preflight` → `codegraph_missing`（新变体，reason_code 直取 `code`）；`pre_check` provider turn → 前置探测后不会发生（万一绕过 → stderr 含 `command not found: codegraph` → 子码 `provider_command_missing`）；`aggregate_index_active` → 投影识别 warning 含 `codegraph_missing` → `reason_code = "codegraph_missing"`
- [x] 8.5 投影侧兜底：`project_member_index_step` 与 `project_aggregate_index_step` 在 `reason_code ∈ {codegraph_missing, codegraph_version_mismatch}` 时投影为 `NotStarted` + `allowed_actions: []` + `failure: None`；`project_notices` 只产出一条
- [x] 8.6 测试装配注入 stub（默认 `Ok("1.6.0")`，既有测试零回归）
- [x] 8.7 用例：探测返回缺失 → operation Failed + `reason_code == "codegraph_missing"` + **所有 step 仍为 `Pending`**；版本不匹配同理；探测可用 → 五步正常推进；投影单测 → `member_index` 与 `aggregate_index_active` 为 `NotStarted` 且通知只 1 条

## 9. 审计快照口径：跳过 .git/ + 元数据差分

映射 requirement：REQ-AUD-01、REQ-AUD-02

- [x] 9.1 遍历时凡路径任一段为 `.git` → 整棵子树跳过（不计条目、不计字节、不进快照）；`classify_member_git` 的 `MemberGit` / `MemberWorktree` 分类**保留**（用于「命令试图改 .git」的判定）
- [x] 9.2 预算口径改为本次变更量：两遍扫描只记元数据（路径 + 大小 + mtime），对比两份元数据找出变化文件，**只对变化文件读内容算哈希**
- [x] 9.3 **保留全仓两遍扫描**（不得改成只扫改动路径）→ 「recipe 写到 allowlist 之外」的检测能力完整保留；MUST NOT 改用 inotify 等事件驱动
- [x] 9.4 改写既有守卫测试 `root_recipe_receipt_tests.inc.rs:866-912`（`.git/HEAD` 可观测断言）→ 按新口径改写（owner 已确认接受该代价：平台检测不到成员仓 git 提交状态）
- [x] 9.5 用例：`.git/HEAD` 不在快照 entries；改一个文件 → 变更清单只含它；越界写入仍被检出；存量构建产物很大但本次无改动 → 不因存量体量失败

## 10. 流式读与预算失败面文案

映射 requirement：REQ-AUD-03、REQ-AUD-04

- [x] 10.1 新增结构化错误变体 `ProductStoreError::AuditSnapshotBudgetExceeded`（Display 保留现状英文机器行，不破坏既有消费方），携带 `AuditSnapshotBudgetFailure { max_bytes, cumulative_bytes, over_line_path, top_groups, member_summaries }`
- [x] 10.2 把预算失败结构放到**独立小模块**（`json_store` 不反向依赖 `logical_codebase`），实施时二选一并保持依赖方向单向
- [x] 10.3 文件内容改为流式读取：分块读 + 边读边累加 + 边算哈希，越预算立刻停（内存峰值 = 块大小）；MUST NOT 先整读后计费
- [x] 10.4 失败面文案成品：说明「不是某个文件特别大」+ 大项排行 + 成员仓归属 + 两条出路（清理大目录 / 写 `.git/info/exclude`）+ 明确「不要改 `.gitignore`（被跟踪文件 + 写入范围门禁）」
- [x] 10.5 用户指引 MUST NOT 再要求把 `/.git/` 写进排除文件（K-2 后平台已不看 `.git`）
- [x] 10.6 用例：预算失败 → 变体携带 `top_groups` 与 `member_summaries`；4 GB 文件 → 内存不爆且提前中止；旧 receipt（带 before/after 全量快照）仍可反序列化

## 11. provider / codegraph 的安装与更新机制

映射 requirement：REQ-PCI-01 ~ REQ-PCI-06

- [x] 11.1 固化安装源表：npm `@anthropic-ai/claude-code` / `@openai/codex` / `@earendil-works/pi-coding-agent` / `@colbymchenry/codegraph`；Kimi Code 走官方脚本 `https://code.kimi.com/kimi-code/install.sh`
- [x] 11.2 npm 安装命令形态：`npm install -g --prefix <可写前缀> --omit=optional --no-audit --no-fund <pkg>@<版本>`，环境带 `npm_config_registry`；默认阿里镜像，`NPM_REGISTRY` 可覆盖；**一个包一次安装**（不得合并成一次 `npm install -g a b c d`）
- [x] 11.3 「有新版」判定：查 registry `dist-tags.latest` + 精确 semver 比较（预发布低于正式版）；补通用 semver 比较（可复用 `provider_health.rs` 的 `parse_version_token` 与 `kimi_version_supported` 范式）；Kimi 不做版本判定
- [x] 11.4 版本查询结果 durable 缓存 24 小时（包名 → latest + 查询时间戳）；registry 查询复用既有 `reqwest`
- [x] 11.5 按钮显示规则（能力探针那块）：未安装 → 【安装】+ 手动命令；已安装但探针失败 → 不显示安装，显示原因 + 「核验」（已核验则「更新」）；版本落后 → 【更新】+ 当前 vs 最新；版本最新 → 无按钮
- [x] 11.6 异步安装作业：`POST .../providers/{provider}/install` 立即返回 job_id；`GET .../providers/install/{job_id}` 轮询；作业状态 durable（刷新不丢进度）；流程 = running → 执行 → **装完重新探测**（`provider_health` refresh）→ completed + 结果；另列静态路径 `GET /api/providers/install-targets`：一次拿五类对象（四家 provider + codegraph）的本地版本 + 最新版本 + 应显示动作（能力卡一次渲染即可决定安装/更新/核验/无按钮；独立静态路径避免与 `install/{job_id}` 的通配段冲突）——依据块2 Plan **D-6**（原两条路径逐字保留）
- [x] 11.7 失败分类与兜底：权限不足 / 网络或镜像不可达 / 包名不存在 / 其它四类，各自中文提示 + **手动命令**（npm 类带 `--registry=<registry>`；Kimi 为官方脚本命令）；MUST NOT 自动 sudo
- [x] 11.8 Kimi 执行前 UI 必须明确告知「将执行来自 code.kimi.com 的安装脚本」
- [x] 11.9 安装 ≠ 核验：装完仍需走既有显式核验通道；MUST NOT 因安装成功自动标记能力已确认，也 MUST NOT 自动触发核验探针
- [x] 11.10 codegraph 纳入同一安装机制（阿里镜像），安装按钮出现在 codegraph 缺失的失败面上
      ✅ 2026-11-07 owner 真机实测通过（owner 回报并授权勾选；owner 侧操作端口 5173 开发期 / 4317 后端）。★证据文件待补★ —— 本行证据来自 owner 回报，未附截图/日志文件；owner 提供后再补链接。
- [x] 11.11 扩充 `providers.rs:83-107` 的 `install_hint`（补 npm 包名 / Kimi 脚本地址 / 阿里镜像）

## 12. 回归、门禁与真机验收

映射 requirement：REQ-BOOT-07 ~ REQ-PCI-06（全部）

- [x] 12.1 全部单测绿（含行数门禁 `large_file_guard`）；隔离与纪律测试（`assert_aggregate_scope_discipline` 等）绿；前端 vitest 全绿
  - ✅ 2026-11-04 补勾（收口统一绿测实测）：`cargo test --locked --lib` → 5964 passed / 0 failed / 7 ignored / 0 measured，101.33s（`assert_aggregate_scope_discipline` 等隔离与纪律测试为 lib 单测，定义于 `src/product/logical_codebase/coordinator_provider_turn.inc.rs:678`，含于其中）；`cargo test --locked --all-targets` → lib 5964/0（99.91s）、large_file_guard 2 passed / 0 failed（product_source_and_test_files_stay_under_line_limit ok；行数门禁 109 条登记 · 超自身上限 0 · 未登记且超 1200 的 0 · 路径升序 · cap 全 >1200）；`pnpm test` → 260 files / 2662 tests passed / exit 0（29.18s）。仅剩 2 条红均为【有意登记】：① it_core 197 passed / 1 failed = `workspace_ws_abort_with_pi_reaches_cancelled_state_and_stops_output`（`tests/it_core/workspace_ws_integration/part_04.rs:291`；断言钉 adapter 侧观察点 (ProviderType::Pi, ProviderPermissionMode::Auto)，而 validated 启动按设计只外借 launch_view() 的 role/working_dir/prompt ⇒ 无等价可观察点 ⇒ 已裁定暂留红、待立项；出处：`cadence/notes/2026-11-03_全量失败收尾_it_core_ws_integration_v1.0.md` §6、`cadence/notes/2026-11-03_越线拆分_it_core_part05_v1.0.md`）；② it_web 593 passed / 1 failed = `evidence_query::invalid_query_returns_422_and_uninitialized_index_returns_503`（`tests/it_web/evidence_query/part_03.rs:43`；环境态：codegraph 1.6.2 对未建索引目录恒 exit 0 ⇒「未建索引 ⇒ 503」本机不可复现，产品契约另由 `src/web/handlers/evidence_error_mapping.rs` 的单测钉住；出处：`cadence/notes/2026-11-03_全量失败收尾_it_web_派A3_v1.0.md` §3.1）
- [x] 12.2 明确不改：`with_snapshot_budget()` 生产接线、能力核验「不点击不运行探针」契约、rebuild/sync 的 Degraded 语义、「已初始化」判定文件、B-5 残项 R1 / R2（owner 裁定不做，保持原样）
- [x] 12.3 真机：只装 pi 的机器点聚合初始化 → 中文指引 + 「核验 Pi」按钮，**不出现**英文 `launch cell is Unknown`
      ✅ 2026-11-07 owner 真机实测通过（owner 回报并授权勾选；owner 侧操作端口 5173 开发期 / 4317 后端）。受控环境：临时 PATH 覆盖/移开对应可执行文件，测毕已复原 PATH。★证据文件待补★ —— 本行证据来自 owner 回报，未附截图/日志文件；owner 提供后再补链接。
- [x] 12.4 真机：点核验 → 点重试 → 三个 provider turn 依次完成，**全程只用 Pi**；operation JSON 里 `input.provider == "pi"`、`provider_selection.candidates` 四家齐全
      ✅ 2026-11-07 owner 真机实测通过（owner 回报并授权勾选；owner 侧操作端口 5173 开发期 / 4317 后端）。★证据文件待补★ —— 本行证据来自 owner 回报，未附截图/日志文件；owner 提供后再补链接。
- [x] 12.5 真机：移除 pi 后重试 → 失败面逐家点名「Pi 未安装」+ 另外三家各自缺什么；**不再**一句话归因到 claude
      ✅ 2026-11-07 owner 真机实测通过（owner 回报并授权勾选；owner 侧操作端口 5173 开发期 / 4317 后端）。受控环境：临时 PATH 覆盖/移开对应可执行文件，测毕已复原 PATH。★证据文件待补★ —— 本行证据来自 owner 回报，未附截图/日志文件；owner 提供后再补链接。
- [x] 12.6 真机：无 codegraph 的干净机器点「准备」→ **一条**中文提示（含 `npm i -g @colbymchenry/codegraph` 与「装好后点重试」），无三连红；装好后重试 → 推进到下一个真实失败面
      ✅ 2026-11-07 owner 真机实测通过（owner 回报并授权勾选；owner 侧操作端口 5173 开发期 / 4317 后端）。受控环境：临时 PATH 覆盖/移开对应可执行文件，测毕已复原 PATH。★证据文件待补★ —— 本行证据来自 owner 回报，未附截图/日志文件；owner 提供后再补链接。
- [x] 12.7 真机：成员仓有大构建产物 → 预检失败面含大项排行 + 成员仓 + 两条出路；按指引排除后重试 → 预检通过
      ✅ 2026-11-07 owner 真机实测通过（owner 回报并授权勾选；owner 侧操作端口 5173 开发期 / 4317 后端）。★证据文件待补★ —— 本行证据来自 owner 回报，未附截图/日志文件；owner 提供后再补链接。
- [x] 12.8 契约与留痕：本 change 的 proposal / design / 五份 spec delta / tasks；归档前补独立审查结论
