# 任务补勾清单｜fix-aggregate-cold-start-and-audit-budget（块2）v1.0

- 重判人：`TaskRecheckBlock2`；日期：2026-11-03 17:28；worktree：`/home/michaelche/workspace/github/cadence-aria/.worktrees/bugfix-b-11-02`
- 对象：`openspec/changes/fix-aggregate-cold-start-and-audit-budget/tasks.md`（85 条，逐条按**当前代码实况**重判，不按 09:34 对证快照）
- 编辑记录：只改勾选框 38 处（`- [ ]` → `- [x]`），编辑锚 `#75F4` → `#4303` → `#4BEE`；任务文字/编号/顺序/行数零改动
- 勾选判据：①实现产物在当前代码里存在（文件/函数/文案/测试用例，全部亲验）⇒ 打勾；「待统一绿测」类（代码+用例已落地、只差绿测输出）按派工口径打勾（绿测是交付门不是任务门）

---

## 1. 统计
| 项 | 重判前（09:34 对证后） | 重判后（本次，含 v1.1 追裁） |
|---|---|---|
| 总条数 | 85 | 85 |
| 已勾 `- [x]` | 34 | **74**（+40） |
| 未勾 `- [ ]` | 51 | **11** |

**新勾 40 条**：1.1~1.6（6）、2.1/2.2/2.4/2.5（4）、3.1~3.6（6）、4.2/4.4/4.5（3）、5.2/5.3/5.4（3）、6.8（1）、7.5/7.7/7.8/7.12（4）、8.6/8.7（2）、9.5（1）、10.4/10.6（2）、11.1/11.2/11.3/11.4/11.6/11.7/11.9/11.11（8）

> v1.1 追裁（controller IRC 2026-11-03 17:3x）：**7.7 补勾**（复核 `start_streaming_failure` 命中，见 §2 行 7.7）；**8.6 补勾**（controller 裁定：接受片5 的 no-op 默认 + 显式注入形态，原任务文字过规格）。两处勾选框由 controller 侧同步落盘，本清单同步更新口径。

**未勾 11 条三类分布**：

| 类别 | 数量 | 条目 |
|---|---|---|
| **部分完成**（子项缺口，代码实况亲验） | **3** | 5.6（环境类 recheck 指向未接线）、8.4（②防御性子码映射未实现）、11.10（失败面未挂 codegraph 安装按钮） |
| **收尾/真机未发生** | **8** | 12.1（全量绿测）、12.2（不改清单核对动作）、12.3~12.7（真机五项）、12.8（归档前独立审查） |


**「报告说做了、代码里找不到」的条目：0**（对证清单的「未证实=0」复核后仍成立；反向情况存在——见 §4「报告未覆盖但代码已落地」9 条）。

---

## 2. 逐条对照表（85 条，行号为 2026-11-03 17:1x~17:2x 亲验时点）

### §1 provider 选择器（1.1~1.6，全勾）

| 任务 | 结论 | 代码位置（亲验） |
|---|---|---|
| 1.1 | ✅ 新勾 | `provider_selection.rs:44`（`AGGREGATE_PROVIDER_CANDIDATE_ORDER = [Pi, KimiCode, ClaudeCode, Codex]`）、`:14-15`（doc 钉死「L2 只判 `PlanningReadOnly`」唯一口径） |
| 1.2 | ✅ 新勾 | `provider_selection.rs:227`（`health_snapshot_needs_refresh(snapshot, health.degraded(), now)` 条件刷新）、`:372-374`（`generation == 0` / `degraded()` / `checked_at` 早于 10min 三条件）、`:183`（`reason_code()`）、`:378`（`health_reason_code`）、`:70`（HEALTH_STALE_AFTER_MINUTES=10） |
| 1.3 | ✅ 新勾 | `provider_selection.rs:238`（`ProviderCapabilityStore::for_lc(paths.clone(), lc_id)`）、`:96`（`launch_cell_state_from_record` 读 `action_matrix.row(&action).launch`）、`:119`（`launch_cell_state` 载入后判定） |
| 1.4 | ✅ 新勾 | `provider_selection.rs:141-160`（`launch_rejection_error`：前缀 `PROVIDER_CAPABILITY_LAUNCH_NOT_CONFIRMED` 与 `launch cell is unknown/denied; reason:` 既有文案形态逐字保留，EvidenceStale 文案原样）；`production_policy_resolvers.rs:305-317`（`require_supported` 改调 `selection::launch_cell_state_from_record` 按态分流）。落点为新模块（块2 Plan §2.1 裁决，语义一致） |
| 1.5 | ✅ 新勾 | `provider_selection.rs:52-62`（四个 `REASON_CODE_*` 常量）、`:186-189`（`reason_code()` 四臂映射）；不建 operation：`tests_parts/selection_failure.inc.rs:209`（`assert_no_operation_created`）+ `:336/:354`（no-cli/degraded 两用例断言未创建） |
| 1.6 | ✅ 新勾 | `provider_selection.rs` 测试：`:600`（四家皆核验选首位 pi）、`:636`（pi 缺失跳过选下一家）、`:723`（全未核验 → NeedsVerification 且不建 operation）、`:761`（Denied 与 Unknown 候选如实区分）、`:793`（degraded 不建 operation）；共 10 个内联测试 |

### §2 选择器接线与冻结落账（2.1/2.2/2.4/2.5 新勾；2.3 保持）

| 任务 | 结论 | 代码位置（亲验） |
|---|---|---|
| 2.1 | ✅ 新勾 | `handlers.inc.rs:100`（manifest 后 `select_aggregate_provider_blocking` 插入）、`:128`（`provider: selection_table.selected.clone()`）；`claude_code` 字面量全文件 0 残留（grep 亲验）。同步桥接 `:667-682`（专用线程 + current-thread runtime） |
| 2.2 | ✅ 新勾 | `handlers.inc.rs:633`（`ensure_recipe_trust_ready(..., selected: &ProviderName)`）、`:649`（`std::slice::from_ref(selected)`）、`:646-648` 注释（Pi/ClaudeCode 过滤后为空 → 门 no-op；KimiCode/Codex 进登记） |
| 2.3 | ✅ 保持 | （片1 报告 §1.2；`aggregate_initialization.rs:462/:526-530/:380-411`，对证清单已勾） |
| 2.4 | ✅ 新勾 | `dto/aggregate_initialization.rs:19-51`（`provider_selection` → `aggregate_provider_selection_dto`）；前端 `AggregateInitializationCard.tsx:115`（`SKIPPED_REASON_LABELS` 四态）、`:126`（`providerSelectionSummary`：本次使用 X；跳过：Y（原因））、`:285-288`（摘要行 + `data-testid`） |
| 2.5 | ✅ 新勾 | `tests_parts/selection_failure.inc.rs:399-420`（`selected_pi_skips_trust_gate_and_freezes_provider`：断言 `operation.input.provider == "pi"` + 候选表落账）、`:245`（`candidates[0]["provider"] == "pi"`）、`:349`（no-cli 用例断言 candidates.len()==4 四家齐全） |

### §3 provider 走 request（3.1~3.6 全勾）

| 任务 | 结论 | 代码位置（亲验） |
|---|---|---|
| 3.1 | ✅ 新勾 | `aggregate_initialization_coordinator.rs:419`（`pub provider: &'a ProviderName`）；`coordinator_lifecycle.inc.rs:890`（`run_provider_turn` 顶部 `self.frozen_provider(project_id, operation_id, step)?`）；`coordinator_frozen_provider.inc.rs`（durable 解出 + 未知串 fail-closed → ProviderTurnDiagnosed，step:None） |
| 3.2 | ✅ 新勾 | `coordinator_provider_turn.inc.rs:45`（`with_provider`）、`:60`（`with_provider_and_admission`，admission 预检保留）；`provider_gateway.rs:467-481`（`from_provider_name` 四家显式 match + `Fake => Err(UnsupportedCapability)`，无 `_ =>` 静默回退） |
| 3.3 | ✅ 新勾 | 旧构造器 `GatewayBackedAggregateProviderTurnDriver::claude_code*` 全仓 grep **0 命中**（亲验；残存 `ProviderRef::claude_code(...)` 属 `ProviderRef` 类型的正常构造器，非 driver 旧构造器）；`production_dependencies.inc.rs:181`（`from_provider_name`）、`:203/:251`（改调 `with_provider_and_admission`/`with_provider`） |
| 3.4 | ✅ 新勾 | `coordinator_provider_turn.inc.rs:41-44`（旧 Codex 路由阻断注释已删，现为「片4（REQ-BOOT-07，provider 走 request）」新注释）；`provider_gateway.rs:467`（旧 467-469 注释位现为 `from_provider_name` 函数） |
| 3.5 | ✅ 新勾 | `coordinator_tests_contract.inc.rs:104-114`（断言 `aggregate recipe provider must equal operation.input.provider` + `:159` `selection.selected == operation.input.provider`）；`coordinator_tests_profile_trust.inc.rs:636-643`（同口径改写） |
| 3.6 | ✅ 新勾 | `coordinator_tests_selection.inc.rs:196`（`frozen_provider_survives_execute_and_retry_without_reselection`：begin 后 health 变化仍用 pi）、`:250`（`unknown_frozen_provider_fails_closed_without_fallback`）；`coordinator_tests_contract.inc.rs:131`（`frozen_provider_selection_table_survives_execution`）、`coordinator_tests_selection.inc.rs:162`（`begin_without_selection_freezes_none_and_with_selection_freezes_table`） |

### §4 能力状态可机读（4.2/4.4/4.5 新勾；4.1/4.3 保持）

| 任务 | 结论 | 代码位置（亲验） |
|---|---|---|
| 4.2 | ✅ 新勾 | `production_policy_resolvers.rs:305-317`（`require_supported` 按四态分流；Denied 臂经 `launch_rejection_error` 携带 reason） |
| 4.4 | ✅ 新勾（待统一绿测） | `aggregate_initialization_store_tests.inc.rs:768`（`error_record_keeps_existing_reason_codes_and_actions` 零回归锁在盘） |
| 4.5 | ✅ 新勾（待统一绿测） | `capability_rejection.rs:194/:205/:219/:230/:239`（unknown→Unverified、denied+reason、record missing→Unverified、snapshot mismatch→EvidenceStale、无关串→None 五用例） |

### §5 失败面三态分流（5.2/5.3/5.4 新勾；5.1/5.5 保持；5.6 未勾）

| 任务 | 结论 | 代码位置（亲验） |
|---|---|---|
| 5.2 | ✅ 新勾 | `handlers.inc.rs:798-830`（`capability_revalidate_entry` 六字段 `ui_path/button_label/method/path/body/expect` 逐字在）、`:782-789`（`details.fallback_provider` 仅 Denied = owner 顺序下一家 wire 名，末家 null） |
| 5.3 | ✅ 新勾 | `handlers.inc.rs:794-797`（注释「MUST NOT 指向一键核验全部」）+ `ui_path: "/workbench"` 单家核验入口；块2 域内「一键核验全部」引用 0 命中 |
| 5.4 | ✅ 新勾 | `handlers.inc.rs:749-751`（`details.candidates` = 选择器候选表整体下发，owner 顺序）；`dto/aggregate_initialization.rs:122-127`（provider/display_name/cli_available/cli_reason_code/capability_state/skipped_reason/action_hint/warning 全字段镜像）；`aggregate_initialization.rs:415-416`（`warning` 带 `#[serde(default, skip_serializing_if)]`）；`provider_selection.rs:263-297`（action_hint 五态全为可执行动作） |
| 5.6 | ❌ 未勾 | 见 §3 未勾明细 |

### §6 文案单一来源（6.8 新勾；6.1~6.7 保持）

| 任务 | 结论 | 代码位置（亲验） |
|---|---|---|
| 6.8 | ✅ 新勾 | `grep -rn 'no stderr summary' src/` → **0 命中**（本次亲跑）；最早记录兜底由 `failure_copy.rs` 中性三段兜底 + `mark_undefined_cause` 覆盖（`:210-227`）；七码逐条命中 = `failure_copy.rs` 13 内联测试；Rust DTO（`logical_codebase_bootstrap.rs`）/`details.copy`（`product_store_error.inc.rs`）/TS（`logical-codebase-bootstrap.ts`）五字段同构（6.5/6.6 已勾亲验复用） |

### §7 结构层与脱敏（7.5/7.7/7.8/7.12 新勾；7.1~7.4/7.6/7.9~7.11 保持）

| 任务 | 结论 | 代码位置（亲验） |
|---|---|---|
| 7.5 | ✅ 新勾 | `aggregate_initialization_coordinator.rs:83`（`SkillsPreparationDiagnosed{reason,retryable,cause}`）、`:102-107`（`ProviderTurnDiagnosed{step,reason,retryable,provider,diagnostic:TurnDiagnostic}`）、`:132-139`（`TurnDiagnostic{cause,output}`）；旧变体 `ProviderTurn`/`SkillsPreparation`/`Preflight` 形状未动（`:218-247` error_record 各臂原样）；`cause=unknown` 诚实标注：`failure_copy.rs:210-227`（`mark_undefined_cause`：标题加「无法确定具体成因」+ 候选清单 + 「列的是候选不是结论」）；脱敏/截断链 = 7.1~7.3 已勾；`no stderr summary` 0 命中。**对证清单曾判「半」，本次按代码实况闭合**（unknown 兜底 + Diagnosed 载体均已落地，且任务原文尾部即 D-8/D-9 批准形态） |
| 7.7 | ✅ 补勾（v1.1 复核） | `coordinator_frozen_provider.inc.rs:38-70`（`start_streaming_failure`：`ProviderGatewayError::Adapter` 臂 → `cause = adapter.code.as_str()`（如 `provider_unauthorized`）+ `diagnostic.output = sanitize_turn_output(stdout + "\n" + stderr)` 走块2 唯一脱敏入口；非 adapter 错误 cause/output 双 None、reason 形态不变）；生产调用点 `coordinator_provider_turn.inc.rs:523`（`start_streaming` 启动期 `.map_err(start_streaming_failure)`）；测试 `coordinator_tests_selection.inc.rs:296`（`start_failing_adapter_records_code_subcode_and_sanitized_output`，`:334` 断言 cause `Some("provider_unauthorized")`）。**v1.0 误判更正**：当时只 grep 了 `ProviderAdapterError` 直名（被 `start_streaming_failure` 封装遮蔽），controller 指路后复核命中——B2S4 收尾已实现 |
| 7.8 | ✅ 新勾 | `production_dependencies.inc.rs:29-31`（`Unavailable{stage}`→`skills_unavailable_{stage}`、**`UpdateFailed{stage}`→`skills_update_failed_{stage}`**、`SyncFailed{layer}`→`skills_sync_failed_{layer}`）、`:56-61`（`skills_digest_failed`）；全族落 `aggregate_machine_skills_failed` + cause，无「provider 未登录」归因。**对证清单曾记录 UpdateFailed 丢 stage，13:18 时点代码已补齐（代码实况为据，片5 报告未覆盖该修复）** |
| 7.12 | ✅ 新勾（待统一绿测） | `diagnostic_redaction.rs` mod tests 六用例：`seven_secret_rules_do_not_leak_plaintext`（含 `Authorization: Bearer` + `ANTHROPIC_API_KEY=sk-ant-…`）、`known_path_prefixes_are_localized_relative_paths_kept`（home→`~`）、`oversized_diagnostics_are_truncated_with_marker`（超限→`…[truncated]`）、`command_names_and_error_codes_survive_redaction`、`legacy_sanitize_and_truncate_contract_is_kept`、`ansi_escapes_are_stripped` |

### §8 codegraph 前置探测（8.6/8.7 新勾；8.1~8.3/8.5 保持；8.4 未勾）

| 任务 | 结论 | 代码位置（亲验） |
|---|---|---|
| 8.7 | ✅ 新勾（待统一绿测） | `environment_probe.rs:414`（缺失→Failed+`codegraph_missing`+`assert_all_steps_pending`）、`:449`（版本不匹配同理）、`:477`（可用→记入 durable 快照、五步推进）；投影单测 = `bootstrap_projector.inc.rs` 的 `codegraph_environment_precondition_blocks_index_steps_with_one_notice`（8.5 已勾）；另 `coordinator_tests_environment.inc.rs:223`（探测失败零下游 step/索引） |
| 8.4 | ❌ 未勾 | 见 §3 未勾明细 |
| 8.6 | ✅ 补勾（v1.1 controller 裁定） | **controller 裁定：接受片5 的 no-op 默认 + 显式注入形态（原任务文字过规格）**。实况：`aggregate_initialization_coordinator.rs:528`（`environment_probe: Option<Arc<dyn>>` 默认 None = 探测 no-op，生产 `production_dependencies.inc.rs:997` 注入真实探针）；`StubEnvironmentProbe::available("1.6.0")`（`environment_probe.rs:163`）经 `with_environment_probe` 显式注入（`coordinator_tests_environment.inc.rs:77`）。显式注入比隐式默认 stub 更可控（不会在生产路径悄悄吞探测）；「既有测试零回归」由 None 语义结构性达成 |

### §9 审计快照口径（9.5 新勾；9.1~9.4 保持）

| 任务 | 结论 | 代码位置（亲验） |
|---|---|---|
| 9.5 | ✅ 新勾（待统一绿测） | `root_recipe_receipt_snapshot_tests.inc.rs:13`（`.git/HEAD` 不在 entries 且不计预算）、`:61`（只对变更文件读内容）、`:106`（越界写入仍被检出）、`:129`（存量大产物本次无改动不失败）；另 `:173/:369/:434` 三条扩展用例在盘 |

### §10 流式读与预算（10.4/10.6 新勾；10.1~10.3/10.5 保持）

| 任务 | 结论 | 代码位置（亲验） |
|---|---|---|
| 10.4 | ✅ 新勾 | `failure_copy_table.inc.rs:316-321`（`aggregate_audit_budget_exceeded` 三段：不是某个文件特别大 + 两条出路（清理大目录 / `.git/info/exclude`）+ 不要改 `.gitignore`）；排行与成员仓出口已闭合：`audit_snapshot_budget.rs:27`（`AUDIT_BUDGET_EXCEEDED_REASON_CODE`）、`:107-110`（Display 渲染 `top_groups:`/`member_summaries:`）、`product_store_error.inc.rs:433-435`（`AuditSnapshotBudgetExceeded => details.message = failure.to_string()`）；`error_record` 臂（coordinator `:175-183` cause+diagnostic）；守卫用例 `root_recipe_receipt_snapshot_tests.inc.rs:389`（`budget_diagnostic_carries_groups_and_member_summaries`）。**对证清单曾判「半」（排行无渲染出口），13:22 时点后代码已闭合（代码实况为据，片6 报告未覆盖）** |
| 10.6 | ✅ 新勾（待统一绿测） | `root_recipe_receipt_snapshot_tests.inc.rs:211`（载荷携带 top_groups+member_summaries）、`:267`（超大文件提前中止不整读）、`:299`（旧 receipt 全量快照仍可反序列化） |

### §11 安装与更新机制（11.1/11.2/11.3/11.4/11.6/11.7/11.9/11.11 新勾；11.5/11.8 保持；11.10 未勾）

| 任务 | 结论 | 代码位置（亲验） |
|---|---|---|
| 11.1 | ✅ 新勾 | `provider_installation/source_table.rs:41/48/55/69`（`@anthropic-ai/claude-code`/`@openai/codex`/`@earendil-works/pi-coding-agent`/`@colbymchenry/codegraph`）、`:19`（`KIMI_INSTALL_SCRIPT_URL = https://code.kimi.com/kimi-code/install.sh`）、镜像默认阿里 + `NPM_REGISTRY` 覆盖 |
| 11.2 | ✅ 新勾 | `provider_installation/runner.rs:5-7`（doc 钉死形态）、`:73`（`npm_config_registry`）、`:79-83`（`--prefix`/`--omit=optional`/`--no-audit`/`--no-fund` 单包 argv）；单包一次安装（argv 形态无多包合并） |
| 11.3 | ✅ 新勾 | `registry_version.rs:41-101`（`dist-tags.latest` packument 解析）；`cross_cutting/semver.rs:16-17/59-69`（`pre_release: Option<String>`，「None = 正式版（高于任何预发布）」，`+build` 剥离）；Kimi 源表无 npm_package ⇒ 不做版本判定 |
| 11.4 | ✅ 新勾 | `registry_version.rs:1`（doc「dist-tags.latest 查询 + 24h durable 版本缓存」）、`:54`（reqwest GET）、`:106`（24h 缓存优先解析器） |
| 11.6 | ✅ 新勾 | `app.rs:27-39` 三路由（`GET /api/providers/install-targets`、`POST /api/providers/{target}/install`、`GET /api/providers/install/{job_id}`——D-6 裁决 `{target}` 通配以容纳 codegraph，install-targets/install/{job_id} 逐字保留）；`job_store.rs:1-5`（durable 状态机 `running → completed|failed` 终态不可迁移，先落盘再返回）、`:73/:105/:125`；装完重探测 = `runner.rs:9/:197`（`HealthRefreshProber`→`provider_health.refresh`）；台账 A6（三端点 vs 四端点）controller 已备案 |
| 11.7 | ✅ 新勾 | `failure_classify.rs:13-19`（权限/网络/包名/其它四类）、`:79-85`（各自中文提示 + `{manual_command}`）、`:130`（EACCES→管理员命令**不带 sudo 前缀**，MUST NOT 自动 sudo 的守卫用例） |
| 11.9 | ✅ 新勾 | `runner.rs:9-10`（doc「安装 ≠ 核验：不写 capability、不触发核验探针」）、`:167`（`PostInstallProber` trait 只管 health refresh）、`:478`（`SpyProber` 断言不触能力） |
| 11.10 | ❌ 未勾 | 见 §3 未勾明细 |
| 11.11 | ✅ 新勾 | `src/web/handlers/providers.rs:83-108`（四家 install_hint 已换新文案：npm 包名 + 默认镜像 registry.npmmirror.com + `NPM_REGISTRY` 覆盖 + Kimi 官方脚本命令）、`:275-277`（断言同步改） |

### §12 回归、门禁与真机验收（12.1~12.8 全部未勾）

| 任务 | 结论 | 代码位置/证据 |
|---|---|---|
| 12.1 | ❌ 未勾 | 全量绿测未取得：17:00 Main 通报 `cargo check --lib --tests` 刚复验 0 error，三簇（coding_workspace_engine/web/workspace_engine）夹具修复仍在途；全量 `cargo test --all-targets` + pnpm 三连是 Main 收口动作 |
| 12.2 | ❌ 未勾 | 「明确不改」清单的核对动作未完整执行；本次抽查两点保持原样（`root_recipe_receipt_auditor.inc.rs:38` `with_snapshot_budget` 在、`aggregate_index/codegraph_cli.rs:44` `Degraded` 语义未改），其余三点（核验契约/已初始化判定/B-5 残项 R1R2）未逐一核对留痕 |
| 12.3 | ❌ 未勾 | 真机验收未执行（交付说明 §:283 明示「真机验收 12.3~12.7 五项未执行」） |
| 12.4 | ❌ 未勾 | 同上 |
| 12.5 | ❌ 未勾 | 同上 |
| 12.6 | ❌ 未勾 | 同上（代码前置件已就绪：码表 `codegraph_missing` :251-254 + 8.5 投影兜底） |
| 12.7 | ❌ 未勾 | 同上（10.4 排行链本轮已闭合，真机仍未跑） |
| 12.8 | ❌ 未勾 | 归档前独立审查未发生；proposal/design/五份 spec delta/tasks 均在盘（`openspec/changes/fix-aggregate-cold-start-and-audit-budget/{specs/*, tasks.md, design.md, proposal.md}`） |

---

## 3. 未勾 11 条明细（缺什么 / 归属）

| 任务 | 未勾原因 | 缺口归属 |
|---|---|---|
| **5.6** | 三类分诊**部分落地**：产品缺陷类（`degraded`/`evidence_stale`→ProductDefect+原文+重试）与重试类（`timeout`/`trust_waiting`→RetryOnly「重试」）已在 `failure_copy_table.inc.rs` 码表 + `FailureTriage` 全量落表；**缺**：环境类（`command_missing`/`version_too_low`）「复用 `providers.rs:81-110` install_hint 原文并指向 `POST /api/providers/recheck`」——全仓负查 `providers/recheck` 在块2 失败面文案域 **0 命中**（唯一出现点 `app.rs` 是既有路由定义）；候选表 `action_hint` 只写「装好后重试本步」，未复用 install_hint 原文 | 片3/片4 残留 → handler/码表侧补接线（候选表 action_hint 域归 REQ-PCR-02，controller ①-补E 已裁其不在 REQ-BOOT-08 单一来源范围，但「指向 recheck」这一具体要求仍无人落） |
| **8.4** | 三处码统一**两落一缺**：①`codegraph_missing` 直取 code ✅（`environment_probe.rs:167-170` + error_record EnvironmentPrecondition 臂）；③投影侧以 **operation 错误码**为键 ✅（`bootstrap_projector.inc.rs:88/:568/:650`，controller 已裁与「warning 含 codegraph_missing」口径等价）；**缺**②「pre_check 万一绕过 → stderr 含 `command not found: codegraph` → 子码 `provider_command_missing`」的特定映射（全仓该子串只作为 `failure_copy.rs:214` 候选清单文案出现；`provider_retry.rs:1276` 是既有 `ProviderErrorCode` 枚举映射，非 stderr 文本推导）。②被 8.2 前置探测结构性阻断（防御性路径，正常不可达） | 若保留任务文字需补 stderr→子码映射（片5/防御性小改）；若按「探测后不可达」裁废则需改任务文字（controller 裁） |
| **11.10** | **部分落地**：codegraph 已纳入安装机制（`source_table.rs:69` + `install-targets` 含 codegraph 对象 + `ProviderCapabilityCard.tsx:336-354` 独立安装行）；**缺**：「安装按钮出现在 codegraph 缺失的失败面上」——`LogicalCodebaseBootstrapCard.tsx`（失败面卡片，424 行）`codegraph`/`install` **双 0 命中**（亲验），按钮只在能力卡 | 片7B/后续前端批次（或 controller 裁「能力卡即满足」） |
| **12.1** | 全量单测绿 + 行数门禁 + 隔离纪律测试 + 前端 vitest 全绿：绿测未取得（编译 17:00 刚恢复，三簇夹具在途；Main 收口时统一跑） | controller 收口 |
| **12.2** | 「明确不改」清单核对动作未完整执行（本次仅抽查 2/5 点保持原样） | controller 收口 |
| **12.3~12.7** | 真机验收五项未执行（无报告无证据；交付说明 §:283 明示未执行） | 收口 / 真机环境 |
| **12.8** | 归档前独立审查未发生 | controller 收口 |

---

## 4. 「报告未覆盖但代码已落地」条目（代码实况为据打勾）

以下 9 条的实现证据在既有切片报告的「未完成/施工中」章节里缺席或被记为待做，但当前代码实况已闭合（全部亲验）：

| 任务 | 报告时点状态 | 当前代码实况 |
|---|---|---|
| 2.1/2.2 | `实施_2_4` §未完成-1「handlers 选择器接线未落地」（11:40 时点） | `handlers.inc.rs:100/:128/:633/:649` 已接线（mtime 11:25，报告 11:40 收尾时未回看；worker 在报告截断后已补） |
| 2.5 / 3.6 的 handler 侧用例 | `实施_2_4` §未完成-2「handler 测试未落地」 | `tests_parts/selection_failure.inc.rs`（20.7 KB，mtime 11:09）7 用例在盘 |
| 7.8 | 对证清单：「`UpdateFailed{stage}` 落入通用 `skills_preparation_failed` 丢 stage」 | `production_dependencies.inc.rs:30` 已映射 `skills_update_failed_{stage}`（mtime 13:18） |
| 7.5 | 对证清单判「半」（Preflight 无诊断载体） | 任务原文尾部即 D-8/D-9 批准形态（Preflight 不加字段），`mark_undefined_cause` unknown 兜底已在（`failure_copy.rs:210-227`） |
| 10.4 | 对证清单/片6 报告：「top_groups 无用户面渲染出口」 | Display 渲染（`audit_snapshot_budget.rs:107-110`）+ `product_store_error.inc.rs:433` 接线 + 码表三段（`failure_copy_table.inc.rs:316-321`）+ 守卫用例（`:389`） |
| 11.1~11.4/11.6/11.7/11.9/11.11 | 片7A 报告「施工中/验收未取得」（11:07 时点） | 代码与测试全部在盘（`provider_installation/*`、`provider_installation.rs`、`app.rs:27-39`、`providers.rs:83-108`，mtime ≤11:07，之后无变动 = 已稳定）；按「待统一绿测」口径打勾 |

## 5. 与 09:34 对证快照的差异汇总

- 对证清单 51 条未勾中，**40 条改判已勾**：落3（1.1~1.6/4.2/3.4）、落4（2.1/2.2/2.4/2.5/3.1/3.2/3.3/3.5/3.6/5.2/5.3/5.4）、待绿测（4.4/4.5/7.12/8.7/9.5/10.6）、半（7.5/7.8/10.4 改判闭合；7.7 经 controller 指路复核改判已实现、8.6 经 controller 追裁接受形态；5.6/8.4/11.10 维持）、落7A（11.1/11.2/11.3/11.4/11.6/11.7/11.9/11.11）、6.8（门禁亲跑 0 + 用例在盘）。
- 对证清单的「落收」12.1~12.8（8 条）全部维持未勾（绿测/真机/归档审查均未发生）。
- 「报告说做了代码找不到」：0 条（与对证清单一致）。

## 6. 收尾自证（17:28 首轮 / 17:34 追裁后复核，均实跑输出）

```
$ ls -la openspec/changes/fix-aggregate-cold-start-and-audit-budget/tasks.md
-rw-r--r-- 1 michaelche michaelche 22969 Nov  3 17:34 tasks.md

$ wc -l openspec/changes/fix-aggregate-cold-start-and-audit-budget/tasks.md
149 openspec/changes/fix-aggregate-cold-start-and-audit-budget/tasks.md

$ grep -c '^- \[x\]' …tasks.md → 74
$ grep -c '^- \[ \]' …tasks.md → 11
（74 + 11 = 85 ✓；行数 149 与编辑前一致，未加删行）
```

未勾 11 条编号：5.6、8.4、11.10、12.1、12.2、12.3、12.4、12.5、12.6、12.7、12.8。

> v1.1 落盘说明：7.7/8.6 两处勾选框在本次复核期间由 controller 侧同步落盘（本 agent 两次编辑均因文件先行变更被拒、复核后确认与裁定一致）；本清单仅同步口径与证据。

## 7. 边界声明

- 只改了 `tasks.md` 的 38 处勾选框；新建本清单文件；未改源码 / spec / design / proposal / Plan / 技术方案 / 其它 change 的 tasks.md / 既有 notes。
- 未 `git add` / `git commit`。
- 判定所涉源码文件 mtime 均在 13:22 前后及更早（`handlers.inc.rs` 11:25、`provider_selection.rs` 11:11、`production_dependencies.inc.rs` 13:18、`provider_installation/*` 10:11-11:07），截至 17:28 无新变动，**不在三簇修复（夹具）在途面内**，结论稳定；唯一在途风险 = 统一绿测若暴露实现缺陷，按派工口径将对应条目改回未勾。
