# task 对证清单｜fix-aggregate-cold-start-and-audit-budget（块2）v1.0

- 对证人：`TaskVerifyBlock2`；日期：2026-11-03；worktree：`/home/michaelche/workspace/github/cadence-aria/.worktrees/bugfix-b-11-02`
- 对象：`openspec/changes/fix-aggregate-cold-start-and-audit-budget/tasks.md`（85 条，逐条对证，非抽查）
- 判定依据：片1/片2/片5/片6/片7A/片7B 六份实施报告（片3/片4 无报告）+ 亲自查证代码（全部亲验，行号为 2026-11-03 上午查证时点；**树是移动靶**——片3/片4/片7A worker 正在并行落地，`provider_selection.rs` 在本次对证期间（09:39→09:47）两次更新）
- 勾选动作：只改 `- [ ]` → `- [x]`，未动任何任务文字/编号/顺序/其它段落（编辑锚 `#1E46` → `#CA48`，34 处单行替换）

---

## 1. 统计

| 项 | 数量 |
|---|---|
| 总条数 | **85**（组1~组12） |
| 已勾选 `- [x]` | **34** |
| 未勾选 `- [ ]` | **51** |

未勾选 51 条的三类分布（按任务书口径，cargo 依赖项并入「待切片落地」）：

| 类别 | 数量 | 条目 |
|---|---|---|
| **待切片落地**（含「待统一绿测」子类） | **43** | ① 片3 未收口：1.1~1.6、4.2、3.4（8 条）；② 片4 未收口：2.1、2.2、2.4、2.5、3.1、3.2、3.3、3.5、3.6、5.2、5.3、5.4（12 条）；③ 片2 残留未做：7.7（1 条）；④ 片7A 施工中：11.1、11.2、11.3、11.4、11.6、11.7、11.9、11.11（8 条）；⑤ 组12 收尾（controller）：12.1~12.8（8 条）；⑥ **待统一绿测**（代码+用例已落、crate 因他片在途不编译，绿测未取得）：4.4、4.5、7.12、8.7、9.5、10.6（6 条） |
| **部分完成** | **8** | 5.6、6.8、7.5、7.8、8.4、8.6、10.4、11.10 |
| **未证实**（报告称已做但代码找不到） | **0** | 无 |

> 备注（与派工单的差异）：派工单把片5 列为「在途」，但盘上已有 `2026-11-03_实施_块2_片5_v1.0.md`（v1.0，09:26）且代码全部亲验在盘，故片5 的**代码型任务按勾选规则正常打勾**（7.9/7.10/7.8*/8.1/8.2/8.3），其「验收状态」章节为空占位 → 片5 的**用例型任务（8.7）与零回归承诺按待绿测处理**。7.8 因映射口径与任务原文不符归部分完成（见 §3）。
> 佐证：controller 09:46:04 统一全量 `cargo test`（`/tmp/aria-full-lib-test.log`）= **202 个编译错误、零测试结果**（块3片1 改 `logical_codebase_id` 类型连带 104 处 + 片7A `runner.rs` 缺 import 等，已派 6 个清扫 worker）——所有 Rust 绿测依赖条目按任务书归「待切片落地/待绿测」。

---

## 2. 逐条对照表（85 条）

结论代号：✅已勾选 ｜ 落3=待片3 ｜ 落4=待片4 ｜ 落7A=待片7A ｜ 落收=待 controller 收尾 ｜ 绿=待统一绿测 ｜ 半=部分完成 ｜ 落2=片2 残留

| 任务 | 结论 | 证据出处（报告:行 → 代码:行，均亲验） |
|---|---|---|
| 1.1 | 落3 | 无片3 报告。代码：`provider_selection.rs:44`（`AGGREGATE_PROVIDER_CANDIDATE_ORDER`）09:47 在途出现（09:39 首见），无报告无绿测 |
| 1.2 | 落3 | 同上：`provider_selection.rs:206/219`（`select_aggregate_provider`）；L1 条件刷新逻辑在途未证 |
| 1.3 | 落3 | 同上：`provider_selection.rs:96/119`（`launch_cell_state_from_record`/`launch_cell_state`）；无报告 |
| 1.4 | 落3 | `production_policy_resolvers.rs:311-316` 已改调 `selection::launch_cell_state_from_record`（diff 亲验）、`provider_capability_store.rs:734` 已 `pub(crate)`；但方法落点在新模块（非 `provider_capability_store.rs`），「保留错误前缀」无绿测、无报告 |
| 1.5 | 落3 | `provider_selection.rs`（09:47 版）含选择期结构；四失败码端到端未见报告 |
| 1.6 | 落3 | 用例；`provider_selection.rs` 内 9 个 test 在盘但无绿测、无报告 |
| 2.1 | 落4 | `handlers.inc.rs:98` 仍 `provider: "claude_code".to_string()`（亲验），无选择插入（grep `select_aggregate_provider` 0 命中） |
| 2.2 | 落4 | `handlers.inc.rs:591-615` `ensure_recipe_trust_ready` 无 `selected` 参，准入清单仍 `&[ProviderName::ClaudeCode]`（亲验） |
| 2.3 | ✅ | 片1 报告 §1.2（L31）→ `aggregate_initialization.rs:462`（字段+serde）、`:526-530`（builder）、`:380-411`（候选表结构含 rule_version/decided_at/action_hint，doc 注明 owner order） |
| 2.4 | 落4 | DTO 侧 片1 已落（报告 §1.2 L35）；前端摘要行已在盘（`AggregateInitializationCard.tsx:126/283-288`、TS `aggregate-initialization.ts:73`，亲验）——但 2.4 归片4，片4 无报告且接线未完 → 待片4 收口 |
| 2.5 | 落4 | 用例；`coordinator_tests_selection.inc.rs`（09:14）已出现但无报告无绿测 |
| 3.1 | 落4 | 片1 报告 §5-1（L171-172）明确「`AggregateProviderTurnRequest.provider` 判归片4」；调用点迁移未见报告 |
| 3.2 | 落4 | `coordinator_provider_turn.inc.rs:41/58` 旧 `claude_code*` 构造器仍在（亲验），`with_provider*` 未见 |
| 3.3 | 落4 | 同上：旧构造器未删、`production_dependencies.inc.rs` 未改调新构造器 |
| 3.4 | 落3 | 两处过期注释已在工作区删除（`coordinator_provider_turn.inc.rs:41-43`、`provider_gateway.rs:466-469`，git diff 亲验）——但片3 无报告，属在途中间态 |
| 3.5 | 落4 | 断言改写未见（`coordinator_tests_contract.inc.rs` 等无报告） |
| 3.6 | 落4 | 用例；无报告 |
| 4.1 | ✅ | 片1 报告 §1.1（L23）→ `capability_rejection.rs:41-48` 显式 match 三态 token、文档明言禁 Debug（:36-39）、7 个内联测试。⚠️ 落点为新模块 `capability_rejection.rs` 而非任务原文的 `provider_gateway.rs` 常量旁（Plan §2.1 裁决落点，语义一致） |
| 4.2 | 落3 | `production_policy_resolvers.rs:311-316` 按态分流已在盘（diff 亲验，`launch_rejection_error` 承载 Denied reason）；片3 无报告无绿测 |
| 4.3 | ✅ | 片1 报告 §1.1/§2.3（L23/L94）→ `capability_rejection.rs:99` 分类器 + `:51` 三态枚举 + 5 个映射用例 |
| 4.4 | 绿 | 零回归锁测试已写入（`aggregate_initialization_store_tests.inc.rs` 的 `error_record_keeps_existing_reason_codes_and_actions`，片1 报告 §2.3-5/L98）；片1 报告 §3 验收输出为空占位（L107-109），crate 现不编译 → 待统一绿测 |
| 4.5 | 绿 | 分类器 5 用例已写入 `capability_rejection.rs`（片1 报告 §2.3-2/L94）；无绿测输出 |
| 5.1 | ✅ | 片1 报告 §1.2/§4.1（L30/L128-130）→ `aggregate_initialization_coordinator.rs:316-337`（`provider_turn_failure_code` 三态映射）+ `failure_copy_table.inc.rs:254-271`（三能力码中文三段，亲验） |
| 5.2 | 落4 | `dto/aggregate_initialization.rs` grep `revalidate` 0 命中（亲验） |
| 5.3 | 落4 | 码表 action 已是单家「点「核验 {provider}」」（`failure_copy_table.inc.rs:269`，且块2 文件无「一键核验全部」引用——该按钮属块3 的 `ProviderCapabilityBatchBar.tsx`）；但 `details.revalidate` 入口归片4 未落 |
| 5.4 | 落4 | 候选表结构 片1 已落（`aggregate_initialization.rs:393-411` 含 action_hint 等）；「三类失败的 details.candidates」产出依赖片3 选择器+片4 handler |
| 5.5 | ✅ | 片1 报告 §1.2（L37）+ 片2 报告 §1.9（L119-121）→ `dto/aggregate_initialization.rs:161`（stderr_summary 键保留）、`:157`（message=title_zh）、`BootstrapFailureLine.tsx:61-62`（机器码仅次要行） |
| 5.6 | 半 | FailureTriage 三分类+重试类（`*_timeout`/`trust_waiting`→RetryOnly「重试」）与产品缺陷类（`evidence_stale`/`degraded`→ProductDefect+错误原文+重试）已在 `failure_copy_table.inc.rs`（§2 码表，片2 报告 L187-313）；**环境类「复用 providers.rs:81-110 install_hint 原文+指向 POST /api/providers/recheck」未见接线**（码表与 dto 均无 recheck 指向；`providers.rs:42` recheck 端点系既有）。依赖片3/片4 候选表 action_hint 与前端 |
| 6.1 | ✅ | 片2 报告 §1.1（L16-46）→ `failure_copy.rs:59-64`（五字段全 String/枚举非 Option）+ `failure_copy_table.inc.rs:17-61`（六规则→七码+内部别名）+ `:81-127`（七码三段文案）+ `failure_copy.rs:196`（中性兜底）/`:262/281/290/354`（describe 家族）；13 个内联测试 |
| 6.2 | ✅ | 片2 报告 §1.5（L78-84）→ `bootstrap_projector.inc.rs:864-880`（`operation_failure`=describe+record_diagnostic+render_detail 单一实现）、5 处 `BootstrapFailure::new/with_copy`（:110/265/693…）、`:683-684` layout 显式分支 |
| 6.3 | ✅ | 片2 报告 §1.4（L62-76）→ `bootstrap_types.inc.rs:73-75`（triage/copy 双字段 serde default）；triage 只由 `copy.triage` 派生（`BootstrapFailure::new/with_copy` 单一构造入口） |
| 6.4 | ✅ | 片2 报告 §1.6/§1.7（L86-102）→ `registration.rs:252-258`（copy() 委派 describe）+ `product_store_error.inc.rs:25-34`（code/message/reason 保留 + details.copy 追加） |
| 6.5 | ✅ | 片2 报告 §1.10（L123-125）→ `logical_codebase_bootstrap.rs:28-46`（FailureCopyDto 五字段手工镜像 + `From<&FailureCopy>`）、`:56-58`（BootstrapFailureDto.triage/copy） |
| 6.6 | ✅ | 片2 报告 §1.11/§3（L127-129/341-378）→ `logical-codebase-bootstrap.ts:49-65`（五字段必填，`reason_zh: string` 非 `?` 非 null） |
| 6.7 | ✅ | 片2 报告 §1.12-1.13（L131-142）→ `BootstrapFailureLine.tsx:22-81`（主行三段+`<details>` 折叠+triage 中文标签+机器码次要行）+ `LogicalCodebaseBootstrapCard.tsx:24/320`；前端绿测 4+3 通过（报告 §8.1 L443-464） |
| 6.8 | 半 | `no stderr summary` 全仓门禁 **0 已跑绿**（片2 报告 §8.2 L466-481 真实输出）；Rust DTO/details.copy/TS 五字段同构已亲验（见 6.5/6.6 行）；但「最早记录兜底文案」「七码逐条命中」由 cargo 测试断言（`failure_copy.rs` 13 测试），报告 §8.3 Rust 验收全红系他片在途（L483-508）→ 该部分待统一绿测 |
| 7.1 | ✅ | 片2 报告 §1.2/§1.3（L48-58）→ `diagnostic_redaction.rs:1-21` 新模块 + `initializer.rs:433-435`（唯一实现收敛，本地两份已删、改 `use` 委派） |
| 7.2 | ✅ | 片2 报告 §1.2（L50-53）→ `diagnostic_redaction.rs:71-94`（sk-/ghp_/gho_ 等/xoxb-/AKIA/--api-key/--token/--password/--secret）、`:123-201`（Authorization Bearer/Basic/Token；聚合根→`<聚合根>`、home→`~`） |
| 7.3 | ✅ | 同上 → `diagnostic_redaction.rs:17-21`（4096/40/`…[truncated]`）+ `:210-253`（字节+行双截断） |
| 7.4 | ✅ | 片1 报告 §1.2（L31）→ `aggregate_initialization.rs:199-208`（cause serde 形态）+ `aggregate_initialization_coordinator.rs:167-300`（error_record 六元组按变体填 cause/stderr_summary，亲验逐臂） |
| 7.5 | 半 | 按 Plan §0-11 D-8 纪律**未给三热变体加字段**：新变体 `ProviderTurnDiagnosed{provider, diagnostic: TurnDiagnostic}`（coordinator:101-110）、`SkillsPreparationDiagnosed{cause: String}`（:82-90，非 TurnDiagnostic 形态）、`EnvironmentPrecondition`（:112）均在盘（片1 报告 §1.2 L30）；**Preflight 变体无任何诊断载体**（20 处字面量保持原样）——与任务原文「三变体各增 diagnostic」形态不同，待归档口径对齐（owner 已在 Plan 批准 D-8/D-9） |
| 7.6 | ✅ | 片2 报告 §1.15（L144-149）→ `coordinator_provider_turn.inc.rs:268-340`（六分支收敛 `turn_failure`，CANCELLED/TIMED_OUT/PERMISSION/STATUS_FAILED/STATUS_ABORTED/CLOSED）+ `:570-595`（cause 子码 + `sanitize_turn_output(&output.summary())`，git diff 亲验） |
| 7.7 | 落2 | 片2 报告未提及启动期诊断（§1.15 仅六分支）；代码 `coordinator_provider_turn.inc.rs` 的 `ProviderAdapterError` 引用全在测试假件区（:728-846，grep 亲验），无 `cause=code.as_str()`/`diagnostic.output=sanitize(stderr/stdout)` 映射。片2 已收工未做，**需 controller 裁度归属** |
| 7.8 | 半 | 片5 报告 §1.8（L80-91）→ `production_dependencies.inc.rs:19-33`（`skills_unavailable_{stage}`/`skills_sync_failed_{layer}`/`skills_digest_failed` 映射，亲验）；「MUST NOT 归因 provider 未登录」成立（全部落 `aggregate_machine_skills_failed`+cause）；**但 `CadenceSkillsError::UpdateFailed{stage}`（types.rs:51-56 存在）落入通用 `skills_preparation_failed` 丢 stage**，与任务原文 `skills_update_failed_*` 不符（Plan §6.2 契约亦漏此变体） |
| 7.9 | ✅ | 片5 报告 §1.1/§1.4（L19/L49-56）→ `environment_probe.rs:89-120`（`.aria-probe-<pid>-<nonce>` 建删+残留清理）+ `coordinator_preflight.inc.rs:72` 调用 |
| 7.10 | ✅ | 片5 报告 §1.1/§1.3（L20/L46）→ `environment_probe.rs:32/122-146`（三层×`AGGREGATE_SKILL_COMMANDS` 四命令 SKILL.md 校验）+ `coordinator_lifecycle.inc.rs:420-440`（machine_skills 成功后、step:None） |
| 7.11 | ✅ | 片1 报告 §1.2（L37）+ 片2 报告 §1.9（L119-121）→ `dto/aggregate_initialization.rs:157`（message=`describe().title_zh`）、`:164`（原文案→details.action）、`:162`（details.cause） |
| 7.12 | 绿 | 脱敏/长度用例已写入 `diagnostic_redaction.rs` mod tests（6 测试，含 Bearer/sk-ant/`~` 前缀/4096+15 截断，:276 起亲验）；片2 报告 §8.3 该过滤器被片6/7A 编译错挡住 → 待统一绿测 |
| 8.1 | ✅ | 片5 报告 §1.1/§1.8（L16-17/L86-90）→ `environment_probe.rs:44-70`（trait+生产实现）+ `production_dependencies.inc.rs:900-935`（同 `state.command_runner` 克隆+同 `"codegraph"` 装配）。⚠️ 按 Plan D-16 裁决为「同一 CLI 配置」而非同一实例（`CodeGraphCli` 无 Clone、禁改 aggregate_index），与任务原文「同一个实例」表述有偏差（owner 已批准 D-16） |
| 8.2 | ✅ | 片5 报告 §1.3（L44-46）→ `coordinator_lifecycle.inc.rs:370-395`（load_manifest 后、machine_skills 前；失败 `persist_execution_failure(..., None, error)` → 不标 step Failed，注释明言索引不触发） |
| 8.3 | ✅ | 片5 报告 §1.2（L27-31）→ `aggregate_initialization_coordinator.rs:482-495`（Default derive + 两字段 `#[serde(default)]`）+ `coordinator_preflight.inc.rs:136-140`（`with_codegraph_evidence` 落值进快照/preflight.json） |
| 8.4 | 半 | ①`codegraph_missing` 直取 code ✅（`environment_probe.rs:72` + error_record EnvironmentPrecondition 臂）；②「pre_check 万一绕过→`command not found: codegraph`→子码 `provider_command_missing`」**未实现**（全仓 grep 无该映射；`provider_command_missing` 仅作 failure_copy 候选清单出现）；③投影侧以 **operation 错误码**为键（controller 裁决①，片2 报告 §1.17 L156-185）而非任务原文的「warning 含 codegraph_missing」——阻断效果等价但口径不同 |
| 8.5 | ✅ | 片2 报告 §1.17（L156-185）→ `bootstrap_projector.inc.rs:560-580/644-660`（两步 NotStarted+空 allowed_actions+failure None）+ `:803-814`（恰好 1 条同源通知）；用例 `codegraph_environment_precondition_blocks_index_steps_with_one_notice` 已写入（绿测待统一） |
| 8.6 | 半 | `StubEnvironmentProbe` 存在（`environment_probe.rs:148`）并用于 `coordinator_tests_environment.inc.rs`；**`dependencies.inc.rs` 未注入默认 stub**（grep Stub/probe 0 命中，亲验；片5 报告 C3 记录结构性不可行：coordinator 私有字段+不可 Clone，请 controller 裁决）；「既有测试零回归」无绿测 |
| 8.7 | 绿 | 片5 报告 §1.7（L69-78）4 用例已写入 `coordinator_tests_environment.inc.rs`（:135/:177/…亲验）；片5 报告 §6 验收状态空占位（L133-136）→ 待统一绿测 |
| 9.1 | ✅ | 片6 报告 §1/§1.1（L19/L43-66）→ `root_recipe_snapshot.inc.rs:237`（任一段 `.git` 整跳）+ `:154-180`（`collect_git_face` 只记元数据）+ `root_recipe_receipt_store.inc.rs:584`（`classify_member_git` 保留） |
| 9.2 | ✅ | 同上 → `:103-146`（`scan_meta`/`walk_meta` 只记元数据）+ `:257-303`（`diff_meta`/`meta_equivalent`）+ `:325`（`describe_changes` 只对变更读内容）；auditor `:77-126`（亲验） |
| 9.3 | ✅ | 片6 报告 §1.2（L101）→ `root_recipe_receipt_auditor.inc.rs:77/:124`（before/after 两遍全仓扫描保留）；无 inotify |
| 9.4 | ✅ | 片6 报告 §1.3（L109-119，逐条改写理由）→ `root_recipe_receipt_tests.inc.rs:210/747/893/954/1089`（五守卫用例已按新口径改写，函数名与断言亲验；`747` 即原 `:866-912` 用例的新形态 `receipt_snapshot_budget_gate_charges_only_this_windows_change_volume`） |
| 9.5 | 绿 | T1-T9 已写入 `root_recipe_receipt_snapshot_tests.inc.rs:13/61/106/129/211/267/299`（亲验）；片6 报告 §2-§4「待验收输出回填」（L141-154 空占位）→ 待统一绿测 |
| 10.1 | ✅ | 片1 报告 §1.1/§1.2（L24/L32）→ `json_store.rs:28-34`（`#[error("product_store_audit_snapshot_budget_exceeded: {0}")]` 英文机器行）+ `audit_snapshot_budget.rs:32-70`（六字段载荷） |
| 10.2 | ✅ | 片1 报告 §1.1（L24）→ `audit_snapshot_budget.rs` 全文件 **0 个 `use crate`**（亲验），注释明言防反向依赖；`json_store.rs:33` 单向引用 |
| 10.3 | ✅ | 片6 报告 §1.1（L88-95）→ `root_recipe_snapshot.inc.rs:90`（64KiB 块）+ `:498-530`（`stream_file_digest`：分块读+`*charged += read`+`hasher.update`+越限立即 break，亲验无整读） |
| 10.4 | 半 | 码表文案已在（`failure_copy_table.inc.rs:316-321`：「不是某个文件特别大」+两条出路（清理大目录/`.git/info/exclude`）+「不要改 .gitignore」）；**但「大项排行+成员仓归属」未真正到达失败面**：`AuditSnapshotBudgetFailure` 的 Display 只渲染 `累计/上限/path`（`audit_snapshot_budget.rs:73-81`），`top_groups`/`member_summaries` 无任何用户面渲染出口（码表仅写「大项排行与成员仓见下方诊断」，而该臂 `stderr_summary=None`、`product_store_error` other 臂只放 `failure.to_string()`）——片6 报告 §1.3 自认「本片只保证载荷结构」 |
| 10.5 | ✅ | 全 `src/` grep `/.git/` 排除指引 0 命中（亲验）；码表仅建议把大目录写 `.git/info/exclude`（`:319`），无「把 `/.git/` 写进排除文件」表述 |
| 10.6 | 绿 | 用例已写入（`budget_failure_payload_carries_groups_and_member_summaries`:211 / `huge_file_read_stops_at_budget_without_reading_it_whole`:267 / `legacy_receipt_with_content_digests_still_deserializes`:299，亲验）；绿测未取得 |
| 11.1 | 落7A | `source_table.rs:11-19/78-89/212-223` 在盘（四 npm 包+Kimi 脚本+镜像）；片7A 报告状态「施工中」（L5），§2/§3 占位（L44-50） |
| 11.2 | 落7A | `runner.rs:82-86/529` npm argv 形态在盘；同上无验收 |
| 11.3 | 落7A | `semver.rs:75` `compare_semver` 09:47 刚落（B2S7A 正在途替换 stub，Main IRC 通报 runner.rs 12 条 E0433 待修）；`registry_version.rs:54-101` dist-tags 在盘 |
| 11.4 | 落7A | `registry_version.rs:15-16/106-123`（24h TTL+durable 缓存）在盘；无验收 |
| 11.5 | ✅ | 片7B 报告 §1.2（L22-36）→ `ProviderInstallActionButton.tsx:1-12`（四态规则注释+实现 :162-219）+ `manualInstallCommand`；绿测 11 用例（报告 §2.4/§3.1 L94-119） |
| 11.6 | 落7A | `job_store.rs:73-125`（create/get/complete/fail，状态机注释 :3-5）+ `provider_installation.rs:251-308`（端点）+ `app.rs:27-35`（路由）在盘；片7A 施工中无验收 |
| 11.7 | 落7A | `failure_classify.rs` 在盘（四类+中文+手动命令，`:225` Kimi 命令无 sudo）；无验收 |
| 11.8 | ✅ | 片7B 报告 §1.2（L32）→ `ProviderInstallActionButton.tsx:8-10`（告知文案+确认前置+`acknowledge_remote_script`）+ 后端守卫 `provider_installation.rs:261-268`（400 `install_acknowledgement_required`）；绿测 |
| 11.9 | 落7A | `runner.rs` `PostInstallProber`/`HealthRefreshProber` 在盘（片7A 报告 §1 L17）；「不写 capability/不触发探针」的 spy 断言无绿测 |
| 11.10 | 半 | 前端：能力卡 codegraph 独立安装行已落（`ProviderCapabilityCard.tsx:336-354`，绿测覆盖）；后端纳入安装机制在途（片7A）；**「安装按钮出现在 codegraph 缺失的失败面上」未挂载**——`LogicalCodebaseBootstrapCard`（失败面）无该按钮（grep 亲验；`RegistrationWizard` 的引用属块3 面） |
| 11.11 | 落7A | `providers.rs:83-108` install_hint 已换新文案（npm 包名/镜像/Kimi 脚本，亲验）+ `:276` 断言同步改；片7A 施工中无验收 |
| 12.1 | 落收 | 需全量绿测+前端全绿；09:46 统一全量 = 202 编译错误零结果 |
| 12.2 | 落收 | controller 收尾核验项（「明确不改」清单的核对动作未见报告） |
| 12.3 | 落收 | 真机验收，无报告无证据 |
| 12.4 | 落收 | 同上（依赖片3/片4 落地） |
| 12.5 | 落收 | 同上 |
| 12.6 | 落收 | 同上（文案/单条通知的代码面已就绪：码表 `codegraph_missing` + 8.5，但真机验收未做） |
| 12.7 | 落收 | 同上（且 10.4 的排行呈现链未闭合） |
| 12.8 | 落收 | 归档前独立审查，未发生 |

---

## 3. 未勾选条目的具体原因（逐条）

**待片3（8 条，无实施报告；`provider_selection.rs` 于本次对证期间在途出现（09:39→09:47 两版），以下按中间态记录）**
- 1.1/1.2/1.3：选择器模块在途（常量/L1 条件刷新/L2 判定函数已在盘但无报告、无任何绿测；无法证实「只判 PlanningReadOnly 唯一口径」「10min 条件刷新」等行为）
- 1.4：`require_supported` 改调新函数已在盘（diff 亲验），但①方法落点与任务原文不同（`provider_selection.rs` 而非 `provider_capability_store.rs`）②「保留既有错误前缀与文案形态」零回归无绿测③片3 无报告自述完成
- 1.5：四个选择期失败码的「不创建 operation」语义无报告无绿测
- 1.6：用例在途（`provider_selection.rs` 9 个 test），无绿测
- 4.2：按态分流代码在盘，但片3 无报告、`production_policy` 过滤器绿测未取得（crate 不编译）
- 3.4：两处过期注释已在工作区删除（git diff 亲验），但属片3 在途中间态、无报告背书

**待片4（12 条，无实施报告；`begin_with_selection`（lifecycle:151）、`coordinator_tests_selection.inc.rs`、前端摘要行已在盘，均中间态）**
- 2.1：`handlers.inc.rs:98` 仍硬编码 `"claude_code"`，选择插入 0 命中
- 2.2：`ensure_recipe_trust_ready`（:591-615）无 `selected` 参、准入清单仍 `&[ClaudeCode]`
- 2.4：DTO（片1）+ 前端摘要行（`AggregateInitializationCard.tsx:283-288`）已落，但选择器接线（2.1）未完、片4 无报告——半成品
- 2.5/3.6：用例无报告无绿测
- 3.1：request 增 provider 字段及其 5 处调用点迁移归片4（片1 报告 §5-1 明确判归），未见报告
- 3.2/3.3：旧 `claude_code`/`claude_code_with_admission` 构造器仍在（:41/:58 亲验）、clean cutover 未发生
- 3.5：「必须保持 Claude」断言改写无报告
- 5.2：`details.revalidate`/`fallback_provider` 全无（dto grep 0 命中）
- 5.3：单家核验指引的「入口」（revalidate）归片4；码表文案侧已满足单家约束（table:269）
- 5.4：`details.candidates` 的三类失败产出依赖片3 选择器+片4 handler；结构体本身片1 已落

**待片2 残留（1 条）**
- 7.7：启动期 `ProviderAdapterError` → `cause=code`/`diagnostic.output=sanitize(...)` 未实现——片2 报告未提及（§1.15 只覆盖六条丢弃分支），`coordinator_provider_turn.inc.rs` 的该错误类型引用全在测试假件（:728-846）。Plan §3.1 把它派给片2 但片2 已收工未做。**建议 controller 裁度：并入片4（其正在改本文件）或收尾**

**待片7A（8 条，报告状态「施工中」，§2 TDD/§3 验收均为占位；Main 09:5x IRC：`provider_installation/**`+`web/handlers/provider_installation.rs` 21 条编译错误由 B2S7A 修复中、`compare_semver` stub 刚换真实现）**
- 11.1/11.2/11.3/11.4/11.6/11.7/11.9/11.11：产物文件均在盘且形态与任务吻合（本次逐一亲验：源表/argv/semver/24h 缓存/job 状态机/端点/路由/failure_classify/install_hint 新文案），但切片未收口、无 TDD/验收输出、crate 因其自身在途错误不编译——一律不打勾

**待统一绿测（6 条：代码与用例均已落盘并亲验，唯 cargo 绿测被在途编译错阻塞；09:46 统一全量 202 错零结果）**
- 4.4（零回归锁断言已写入 `aggregate_initialization_store_tests.inc.rs`）、4.5（分类器 5 用例）、7.12（脱敏/长度 6 用例）、8.7（环境前置 4 用例）、9.5（片6 T1-T9）、10.6（预算载荷/4GB/旧 receipt 用例）

**部分完成（8 条）**
- 5.6：三分类+重试类/产品缺陷类文案已落；环境类「复用 install_hint 原文+指向 POST /api/providers/recheck」无接线（跨片3/4/前端）
- 6.8：grep 门禁 0 已真实跑绿（片2 §8.2）；同构五字段已亲验；「最早记录兜底/七码命中」的 cargo 断言待绿测
- 7.5：D-8 纪律下以 `*Diagnosed` 新变体实现（ProviderTurn/SkillsPreparation 有载体、EnvironmentPrecondition 有），**Preflight 无诊断载体**、SkillsPreparationDiagnosed 用 `cause:String` 非 `TurnDiagnostic`——与原文「三变体各增 diagnostic」不同（Plan 批准的偏差，建议归档时对齐 tasks.md 措辞或补 Preflight 载体）
- 7.8：Unavailable/SyncFailed/digest 三类 cause 已落；`UpdateFailed{stage}` 落通用串丢 stage（`cadence_skills/types.rs:51` 变体存在但 `production_dependencies.inc.rs:19-24` 映射漏它）
- 8.4：①直取 code ✅；②绕过防御子码 `provider_command_missing` 未实现；③以 operation 错误码为键（controller 裁决①）替代原文「warning 含 codegraph_missing」口径
- 8.6：Stub 存在并用于测试；`dependencies.inc.rs` 未注入默认 stub（片5 C3：coordinator 私有字段不可注入，请 controller 裁决）；零回归无绿测
- 10.4：码表三要素已落；大项排行/成员仓数据无用户面渲染出口（Display 一行英文、该臂 stderr_summary=None、other 臂只放 to_string）——「见下方诊断」承诺落空，建议补 payload→details 的序列化出口
- 11.10：能力卡 codegraph 行 ✅（绿测）；后端机制在途；失败面（BootstrapCard）未挂安装按钮

**待 controller 收尾（8 条）**：12.1-12.8 —— 全量绿测/不改清单核对/五项真机验收/归档审查均未发生（12.6/12.7 的代码前置件已部分就绪）

**未证实：0 条**（没有任何一条「报告称完成但代码找不到」）

---

## 4. 亲自查证的代码位置清单（本轮全部亲验）

**Rust（src/）**
- `src/product/logical_codebase/capability_rejection.rs` :24-32,:41-48,:51,:99（+7 测试）
- `src/product/audit_snapshot_budget.rs` :27,:32-70,:73-81（全文件无 use crate）
- `src/product/json_store.rs` :28-34
- `src/product/logical_codebase/aggregate_initialization.rs` :199-208,:380-411,:455-530
- `src/product/logical_codebase/aggregate_initialization_coordinator.rs` :82,:101-112,:131,:167-300,:316-337,:482-495
- `src/product/logical_codebase/failure_copy.rs` :59-64,:196,:262,:281,:290,:354（+13 测试）
- `src/product/logical_codebase/failure_copy_table.inc.rs` :17-61,:81-127,:254-271,:316-321
- `src/cross_cutting/diagnostic_redaction.rs` :17-21,:71-94,:123-201,:210-253（+6 测试）
- `src/product/repository_store/initializer.rs` :433-435（本地实现已删、use 委派）
- `src/product/logical_codebase/bootstrap_types.inc.rs` :73-75
- `src/product/logical_codebase/bootstrap_projector.inc.rs` :40,:73-88,:110,:265,:560-580,:644-660,:683-684,:769-814,:864-880
- `src/product/logical_codebase/registration.rs` :252-258
- `src/web/handlers/support_parts/product_store_error.inc.rs` :25-34
- `src/web/handlers/aggregate_index.rs` :226-246
- `src/web/handlers/dto/aggregate_initialization.rs` :147-166（grep revalidate=0）
- `src/web/types/logical_codebase_bootstrap.rs` :28-46,:56-58
- `src/product/logical_codebase/environment_probe.rs` :32,:44-70,:72,:89-120,:122-146,:148（+3 测试）
- `src/product/logical_codebase/coordinator_lifecycle.inc.rs` :44-64,:113-114,:151,:370-395,:420-440
- `src/product/logical_codebase/coordinator_preflight.inc.rs` :72,:136-140
- `src/web/handlers/aggregate_initialization/production_dependencies.inc.rs` :5,:19-33,:54-58,:900-935
- `src/web/handlers/aggregate_initialization/dependencies.inc.rs`（grep Stub/probe=0）
- `src/product/logical_codebase/root_recipe_snapshot.inc.rs` :32-48,:90,:103-231,:237,:257-303,:325,:452-490,:498-530
- `src/product/logical_codebase/root_recipe_receipt_auditor.inc.rs` :77-126
- `src/product/logical_codebase/root_recipe_receipt_store.inc.rs` :584
- `src/product/logical_codebase/root_recipe_receipt_tests.inc.rs` :210,:747,:893,:954,:1089
- `src/product/logical_codebase/root_recipe_receipt_snapshot_tests.inc.rs` :13,:61,:106,:129,:211,:267,:299
- `src/product/logical_codebase/coordinator_provider_turn.inc.rs` :38-70（旧构造器仍在）,:268-340,:570-595（+git diff 全量比对）
- `src/product/logical_codebase/production_policy_resolvers.rs` :24,:248,:311-316（+git diff）
- `src/product/logical_codebase/provider_capability_store.rs` :734（+git diff）
- `src/product/logical_codebase/provider_gateway.rs` :460-472（+git diff 注释删除）
- `src/product/logical_codebase/provider_selection.rs` :44,:75,:96,:119,:141,:165,:206,:219,:396（09:47 在途快照，9 测试）
- `src/product/logical_codebase/coordinator_tests_environment.inc.rs`（4 用例）；`coordinator_tests_selection.inc.rs`（在途出现）
- `src/web/handlers/aggregate_initialization/handlers.inc.rs` :98,:103,:591-615
- `src/product/provider_installation/{mod,source_table,registry_version,job_store,runner,failure_classify}.rs`（关键行见 §2）
- `src/web/handlers/provider_installation.rs` :3-8,:251,:261-268,:308,:514,:621
- `src/web/app.rs` :27-35；`src/web/handlers/mod.rs`（pub mod/use）；`src/product/app_paths.rs`（路径函数）
- `src/web/handlers/providers.rs` :42,:83-108,:276
- `src/cross_cutting/semver.rs` :75；`src/cross_cutting/mod.rs`（diagnostic_redaction+semver 并存）
- `src/product/cadence_skills/types.rs` :44-64（UpdateFailed 变体存在）

**前端（web/src/）**
- `web/src/api/types/logical-codebase-bootstrap.ts` :43-65
- `web/src/api/types/aggregate-initialization.ts` :73
- `web/src/components/lifecycle/BootstrapFailureLine.tsx` :22-81（+test 文件存在）
- `web/src/components/lifecycle/LogicalCodebaseBootstrapCard.tsx` :24,:320
- `web/src/components/lifecycle/ProviderInstallActionButton.tsx` :1-12,:162-219（+test 11 用例）
- `web/src/components/lifecycle/ProviderCapabilityCard.tsx` :336-354
- `web/src/api/provider-install.ts`（存在）

**git 工作区**：`git status --porcelain`（90+ 文件 M）与 `git diff HEAD` 对 provider_gateway / provider_capability_store / production_policy_resolvers / coordinator_provider_turn 四文件逐段比对（确认片3 改动在途落盘、旧构造器未删）。

---

## 5. 收尾自证

2026-11-03 10:01 在 worktree 根目录执行的真实输出：

```
$ ls -la openspec/changes/fix-aggregate-cold-start-and-audit-budget/tasks.md
-rw-r--r-- 1 michaelche michaelche 20577 Nov  3 09:55 openspec/changes/fix-aggregate-cold-start-and-audit-budget/tasks.md
$ ls -la cadence/notes/2026-11-03_task对证_fix-aggregate-cold-start-and-audit-budget_v1.0.md
-rw-r--r-- 1 michaelche 31787 Nov  3 10:01 cadence/notes/2026-11-03_task对证_fix-aggregate-cold-start-and-audit-budget_v1.0.md
$ grep -c '^- \[x\]' openspec/changes/fix-aggregate-cold-start-and-audit-budget/tasks.md
34
$ grep -c '^- \[ \]' openspec/changes/fix-aggregate-cold-start-and-audit-budget/tasks.md
51
```

34 + 51 = 85，与总条数一致。备注：该 change 目录在 git 中未跟踪（`?? openspec/changes/fix-aggregate-cold-start-and-audit-budget/`），无 HEAD 基线可比对；勾选动作只经锚定单行替换完成（编辑锚 `#1E46` → `#CA48`，34 处、每处仅 `- [ ]` → `- [x]`），任务文字/编号/顺序/段落未动；清单文件亦为新建，未执行任何 `git add` / `git commit`。

---

## 6. 7.5 措辞修正：对齐 REQ-BOOT-13 + 块2 Plan D-8/D-9（2026-11-03 追加）

### 6.1 问题定性

7.5 原文要求「`SkillsPreparation` / `Preflight` / `ProviderTurn` 三变体各增 `diagnostic: Option<TurnDiagnostic>`」，该要求在 **spec 中没有任何对应条目**：

- `grep -n 'diagnostic\|TurnDiagnostic' openspec/changes/fix-aggregate-cold-start-and-audit-budget/specs/*/spec.md` → **0 命中**（五份 spec delta 全文，亲验）。
- 该措辞的唯一来源是 `design.md:179`（WP-B 第 3 条）的**实现描述**，而这条实现路径已被块2 Plan 的 D-8/D-9 裁决**有意偏离**（controller 2026-11-03 采纳）。

REQ-BOOT-13（`spec.md:157-184`）的实际要求只有三条本体 + 五个场景（原始输出被截断时如实标注 / 凭据与身份路径被脱敏 / 最早期记录也有兜底文案 / 成因不可判定时诚实标注 / 确定性成因被正确归因）。

⇒ 契约语义由「**新增 `*Diagnosed` 变体 + 结构化 `cause`**」满足；原文属**过规格**（要求的实现形态严于 spec，且 Plan 已明确裁决不加该字段）。

### 6.2 改前 → 改后（逐字）

改前（`tasks.md:80`）：

```
- [ ] 7.5 `SkillsPreparation` / `Preflight` / `ProviderTurn` 三变体各增 `diagnostic: Option<TurnDiagnostic>`；新增 `EnvironmentPrecondition { code, reason, retryable }`
```

改后（`tasks.md:80`）：

```
- [ ] 7.5 失败面结构与成因标注（依据 **REQ-BOOT-13** + 块2 Plan §14 **D-8/D-9**）：失败面 **MUST NOT** 出现 `no stderr summary` 这类无信息文案；进入失败面的原始输出须先**脱敏 + 截断标注**（凭据类 → `[REDACTED]`、home 前缀 → `~`、聚合根绝对前缀 → `<聚合根>`；上限 4096 字节 / 40 行，超出以 `…[truncated]` 结尾）；**可判定的成因 SHALL 记为结构化 `cause`**（`skills_unavailable_*` / `skills_update_failed_*` / `skills_sync_failed_*` / `skills_digest_failed` 等族），不可判定时 SHALL 诚实标为 `unknown`、MUST NOT 编造成因；机制形态 = **新增 `*Diagnosed` 变体 + 结构化 `cause`**（`ProviderTurnDiagnosed { step, reason, retryable, provider, diagnostic: TurnDiagnostic }`、`SkillsPreparationDiagnosed { reason, retryable, cause }`），旧变体 `ProviderTurn` / `SkillsPreparation` / `Preflight` 形状一字不动，**MUST NOT** 要求每个既有枚举变体都带 `diagnostic` 字段（D-9：`Preflight` 20 处字面量跨 5 文件、块2 内无消费者，不加字段也不加变体）；另新增 `EnvironmentPrecondition { code, reason, retryable }`（`code` 即最终 `reason_code`）
```

五点覆盖对照：

| # | 要求覆盖点 | 改后措辞落点 |
|---|---|---|
| ① | 失败面不得出现 `no stderr summary` | 「失败面 **MUST NOT** 出现 `no stderr summary` 这类无信息文案」 |
| ② | 脱敏 + 截断标注 | 「凭据类 → `[REDACTED]`、home 前缀 → `~`、聚合根绝对前缀 → `<聚合根>`；上限 4096 字节 / 40 行，超出以 `…[truncated]` 结尾」 |
| ③ | 结构化 `cause` / `unknown` | 「**可判定的成因 SHALL 记为结构化 `cause`**（`skills_unavailable_*` / `skills_update_failed_*` / `skills_sync_failed_*` / `skills_digest_failed` 等族），不可判定时 SHALL 诚实标为 `unknown`、MUST NOT 编造成因」 |
| ④ | 机制形态 + MUST NOT 逐变体加字段 | 「机制形态 = **新增 `*Diagnosed` 变体 + 结构化 `cause`**…旧变体…形状一字不动，**MUST NOT** 要求每个既有枚举变体都带 `diagnostic` 字段（D-9：`Preflight` 20 处字面量跨 5 文件、块2 内无消费者，不加字段也不加变体）」 |
| ⑤ | 依据标注 | 句首「（依据 **REQ-BOOT-13** + 块2 Plan §14 **D-8/D-9**）」 |

勾选框标记 `- [ ]` 保持原状（未勾、未改）。

### 6.3 同类过规格扫描

**判据**：任务文字要求 spec 没有对应要求 **且** Plan 有明确裁决「不加 / 不做」。

**检查范围**：`tasks.md` 组 7（WP-B，失败面结构层与脱敏）全部 12 条 —— 7.1、7.2、7.3、7.4、7.5、7.6、7.7、7.8、7.9、7.10、7.11、7.12（`tasks.md:74-85`），逐条对照 spec delta 与 Plan 裁决/偏离台账。

| 任务 | 判定 | 依据 |
|---|---|---|
| 7.1 | 不过规格 | REQ-BOOT-13 要求脱敏；新建 `diagnostic_redaction.rs` 是实现载体，design WP-B 与 Plan 均指派，无「不做」裁决 |
| 7.2 | 不过规格 | 与 spec 场景「凭据与身份路径被脱敏」（`spec.md:168-172`）逐项对应 |
| 7.3 | 不过规格 | spec 场景「原始输出被截断时如实标注」要求字节 + 行双上限与 `…[truncated]` 标记；4096/40 取既有 `AGGREGATE_TURN_OUTPUT_LIMIT` 同值，无反对裁决 |
| 7.4 | 不过规格 | REQ-BOOT-13「可判定的成因 SHALL 记为结构化 `cause`」直接要求 |
| **7.5** | **过规格 → 已修正** | 见 §6.1 / §6.2：spec 无 `diagnostic` 字段要求，Plan D-9 裁决 `Preflight` 不加 |
| 7.6 | 不过规格 | REQ-BOOT-13 无信息文案 + 结构化 cause 的落点 |
| 7.7 | 不过规格 | `cause = code.as_str()` + 输出脱敏属 spec 要求，载体是 `ProviderTurnDiagnosed`；「未实现」属部分完成（对证已记），不改文字 |
| 7.8 | 不过规格 | spec 场景「确定性成因被正确归因」（`spec.md:181-184`）明文点名 skills 准备并要求结构化子码 |
| 7.9 | 不过规格 | 同上场景点名「聚合根可写性探测」 |
| 7.10 | 不过规格 | 同上场景点名「skills 命令链接层校验」 |
| 7.11 | 不过规格 | REQ-BOOT-08（`spec.md:109-111`）单一文案来源 |
| 7.12 | 不过规格 | 对应 REQ-BOOT-13 的脱敏/截断/兜底场景用例 |

⇒ **组 7 内除 7.5 外无同类过规格措辞。**

**组外另见两处候选（本次未改** —— 边界为「只改 7.5」，且它们属组 5 / 组 8，请 controller 另裁/另派）：

- `8.1`「用与 index **同一个 `CodeGraphCli` 实例**装配」：spec REQ-BOOT-09 只要求「探测 SHALL 与索引构建使用**同一份** CLI **配置**」（`spec.md:190`），Plan **D-16** 明确「`CodeGraphCli` 无 `Clone`…真共享必须加 `Clone` 或换 `Arc`，那要改 `aggregate_index/**`…改为：探针用同一份配置自建」⇒ 同属「spec 不要求 + Plan 裁决不做」的过规格。
- `5.1`「`error_record()` 对 `ProviderTurn{reason}` 先过分类器，按三态产出 … **与对应中文主文案**」：spec REQ-BOOT-08（`spec.md:111`）只要求「同一个失败面文案来源…MUST NOT 在投影层 / DTO / 通知 / 布局引导侧分别维护同义文案」，未要求 `error_record()` 出中文；Plan **D-5** 裁决「`error_record()` 只产出稳定 `reason_code`；中文主文案由片2 `failure_copy` 在投影/DTO 层渲染」⇒ 同类。

### 6.4 勾选框计数自证

```
$ grep -c '^- \[x\]' openspec/changes/fix-aggregate-cold-start-and-audit-budget/tasks.md
34
$ grep -c '^- \[ \]' openspec/changes/fix-aggregate-cold-start-and-audit-budget/tasks.md
51
$ grep -c '^- \[.\] ' openspec/changes/fix-aggregate-cold-start-and-audit-budget/tasks.md
85
```

改前 = 34 / 51；改后 = 34 / 51（34 + 51 = 85 = 总条数，与改前一致）。本次只替换 7.5 单行文字，未触碰任何 `[ ]` / `[x]` 标记。

### 6.5 `validate --strict`

```
$ npx --no-install openspec validate fix-aggregate-cold-start-and-audit-budget --strict
Change 'fix-aggregate-cold-start-and-audit-budget' is valid
exit=0
```

### 6.6 依据链

**REQ-BOOT-13 正文（`spec.md:159`，逐字）**：

> 失败面 SHALL NOT 出现「no stderr summary」这类无信息文案。进入失败面的原始输出 SHALL 先脱敏：凭据类内容 SHALL 被替换为 `[REDACTED]`，身份类路径 SHALL 把 home 前缀替换为 `~`、聚合根绝对前缀替换为 `<聚合根>`；超长输出 SHALL 标注 `…[truncated]`。可判定的成因 SHALL 记为结构化 `cause`，不可判定时 SHALL 诚实标为 `unknown`，MUST NOT 编造成因。

**三条要求拆解**：①「失败面 SHALL NOT 出现『no stderr summary』这类无信息文案」；②「原始输出 SHALL 先脱敏（`[REDACTED]` / `~` / `<聚合根>`）；超长输出 SHALL 标注 `…[truncated]`」；③「可判定的成因 SHALL 记为结构化 `cause`，不可判定时 SHALL 诚实标为 `unknown`，MUST NOT 编造成因」。

**D-8（Plan §14）**：技术方案要求给 `ProviderTurn` 变体加 `provider` / `diagnostic` 两字段；实测 30 处字面量（18 处在 0 余量的 `coordinator_provider_turn.inc.rs`）⇒ **改为新增 `ProviderTurnDiagnosed` 变体**，`ProviderTurn` 原样不动，两者在 `error_record()` 共用同一臂。

**D-9（Plan §14，同构 D-8；controller 2026-11-03 采纳）**：
- `Preflight` **不加任何字段、不新增变体** —— 20 处字面量跨 5 个文件（含不属于任何切片的 `coordinator_profile.inc.rs`），而块2 内**无消费者**（环境前置类失败走 `EnvironmentPrecondition`，provider turn 类走 `ProviderTurnDiagnosed`）。
- `SkillsPreparation` 旧变体不动，**新增 `SkillsPreparationDiagnosed { reason, retryable, cause }`**，「结构化 stage → cause」由该变体满足。
- 复访条件：将来出现真正消费者时再补 `PreflightDiagnosed`（仍不改旧变体形状）。

**Plan §0-11（本块统一纪律）**：热变体不加字段、只加新变体 —— `ProviderTurn` / `Preflight` / `SkillsPreparation` 一律不改形状。

**实施形态（源码亲验，`src/product/logical_codebase/aggregate_initialization_coordinator.rs`）**：
- `:79-83` `SkillsPreparation { reason: String, retryable: bool }`（原样）
- `:82-90` `SkillsPreparationDiagnosed { reason, retryable, cause: String }`
- `:91-92` `Preflight { reason: String, retryable: bool }`（原样，无诊断载体）
- `:93-99` `ProviderTurn { step, reason, retryable }`（原样）
- `:101-110` `ProviderTurnDiagnosed { step, reason, retryable, provider: Option<String>, diagnostic: TurnDiagnostic }`
- `:112-118` `EnvironmentPrecondition { code: String, reason: String, retryable: bool }`（`code` 即最终 `reason_code`）
- `:131-138` `TurnDiagnostic { cause: Option<String>, output: Option<String> }`
- cause 族落点：`src/web/handlers/aggregate_initialization/production_dependencies.inc.rs:28-30`（`skills_unavailable_{stage}` / `skills_update_failed_{stage}` / `skills_sync_failed_{layer}`）、`:60`（`skills_digest_failed`）

### 6.7 改动面自证

只改 `tasks.md` 单行（7.5），未改 spec / design / proposal / Plan / 技术方案 / 源码；未 `git add` / `git commit`。

```
$ ls -la openspec/changes/fix-aggregate-cold-start-and-audit-budget/tasks.md
-rw-r--r-- 1 michaelche michaelche 21619 Nov  3 10:09 openspec/changes/fix-aggregate-cold-start-and-audit-budget/tasks.md
$ wc -l -c openspec/changes/fix-aggregate-cold-start-and-audit-budget/tasks.md
  149 21619 openspec/changes/fix-aggregate-cold-start-and-audit-budget/tasks.md
```

任务条数：改前 149 行 / 20577 字节 → 改后 **149** 行 / **21619** 字节（7.5 单行 176 字节 → 1218 字节，+1042；行数与条数 85 均不变，仅 7.5 单行被替换）。

本报告文件（本节为追加）：追加前 243 行 / 32389 字节 / sha256 `f415e9411b13…5937`；追加后前 32389 字节的 sha256 仍为 `f415e9411b13…5937`（原内容逐字节未动，仅追加本节）。

---

## 7. 8.1 / 5.1 措辞修正 + 12 组全扫（2026-11-03 追加，controller 批准）

判据同 §6：任务文字要求的东西 **spec 无对应要求 / 与 spec 原文不一致**，**且** Plan 有已采纳的裁决明确「不加 / 不做 / 不是那个形态」。**依据不是「代码已经这么写」。**

### 7.1 8.1（组 8）

**改前（`tasks.md:93`）**

```
- [x] 8.1 新增 `AggregateEnvironmentProbe` trait（`ensure_codegraph() -> Result<String, AggregateInitializationError>`），生产实现包一层 `CodeGraphCli`，并**用与 index 同一个 `CodeGraphCli` 实例**装配
```

**改后（`tasks.md:93`）**

```
- [x] 8.1 新增 `AggregateEnvironmentProbe` trait（`ensure_codegraph() -> Result<String, AggregateInitializationError>`），生产实现包一层 `CodeGraphCli`，并**用与索引构建同一份 CLI 配置**装配（runner + executable 字面量一致）——依据 `spec.md:190`「探测 SHALL 与索引构建使用**同一份** CLI 配置」+ 块2 Plan **D-16**（`CodeGraphCli` 无 `Clone`、索引侧按值取所有权，故探针按同一份配置自建；**MUST NOT** 要求「同一个 `CodeGraphCli` 实例」）
```

**依据链**
- `spec.md:190`（REQ-BOOT-09 正文）：「探测 SHALL 与索引构建使用**同一份** CLI 配置」——spec 要求的是**配置**同一，不是对象实例同一；原文「同一个实例」严于 spec。
- Plan **D-16**：「技术方案与旧稿写『探针与索引共用同一个 `CodeGraphCli` 实例』。实测 `CodeGraphCli`…**没有 `derive(Clone)`**，且 `AggregateIndexOperation::new(paths, CodeGraphCli, generator)` 按值取得所有权 ⇒ 真共享必须给 `CodeGraphCli` 加 `Clone` 或换成 `Arc`，那要改 `aggregate_index/**`…改为：**探针用同一份配置自建**…两处 runner + executable 字面量必须一致」。
- Plan **§13「明确不做」**：「**不为「共用 `CodeGraphCli` 实例」改装 `aggregate_index/**`**（`CodeGraphCli` 不加 `Clone`、不改持有方式；只要求两份配置字面量一致，§6.2）」。
- 未改动：`AggregateEnvironmentProbe` trait 形态、`ensure_codegraph()` 签名、勾选框 `[x]`。

### 7.2 5.1（组 5）

**改前（`tasks.md:52`）**

```
- [x] 5.1 `error_record()` 对 `ProviderTurn{reason}` 先过分类器，按三态产出 `aggregate_provider_capability_unverified` / `_denied` / `_evidence_stale` 与对应中文主文案
```

**改后（`tasks.md:52`）**

```
- [x] 5.1 `error_record()` 对 `ProviderTurn{reason}` 先过分类器，按三态产出**稳定 `reason_code`**（`aggregate_provider_capability_unverified` / `_denied` / `_evidence_stale`）；**中文主文案由片2 的 `failure_copy` 在投影 / DTO 层渲染**，`error_record()` MUST NOT 产出中文（片1 不得反向依赖片2，否则破坏依赖图、零回归锁无法逐字节断言）——依据 `spec.md:111`（同一文案来源、MUST NOT 各处分别维护同义文案）+ 块2 Plan **D-5**
```

**依据链**
- `spec.md:111`（REQ-BOOT-08 正文）：「…SHALL 由**同一个失败面文案来源**产出…系统 MUST NOT 在投影层 / DTO / 通知 / 布局引导侧**分别维护同义文案**」——spec 只要求单一来源，未要求 `error_record()` 出中文。
- Plan **D-5**（原文）：「tasks 5.1『`error_record()` 按三态产出主文案』 | `error_record()` **只产出稳定 `reason_code`**；中文主文案由片2 `failure_copy` 在**投影/DTO 层**渲染 | 若 `error_record()` 直接产出中文，**片1（最早）会反向依赖片2（`failure_copy`），破坏依赖图并让零回归锁难以逐字节断言**。」→ 该理由已写入新措辞。
- Plan **D-07**（偏离汇总行 `:1212`）：「`error_record()` **只产稳定 `reason_code`**，不产中文（中文由 `failure_copy` 在投影/DTO 渲染） | §14 D-5」。
- 未改动：分类器三态判定本身（spec `provider-capability-revalidation:55/60` 要求的三个稳定码）、勾选框 `[x]`。

### 7.3 12 组全扫结论

**检查范围**：`tasks.md` **全部 12 组 / 85 条**（`1.1`–`12.8`，行 8–149）。逐条与下列权威来源比对：

1. 五份 spec delta 全文（REQ-BOOT-07/08/09/10/11/12/13、REQ-PCR-01/02、REQ-PCI-01~06、REQ-AUD-01~04、REQ-IND-05）；
2. Plan **§13「明确不做（范围护栏）」** 全部 8 条；
3. Plan **§0 全局约束** 13 条（含 §0-6 不自动核验、§0-8 不改登记表、§0-9 owner 裁定不做项、§0-11 热变体不加字段）；
4. Plan **§14 偏差记录 D-1 ~ D-16**（含 D-07 汇总行）。

**判定为同类过规格并已修（3 处）**

| 任务 | 冲突点 | 已采纳裁决 |
|---|---|---|
| **5.1** | 「`error_record()` 产出中文主文案」 | spec `:111` 单一来源 + Plan **D-5** / **D-07** → 本轮已改 |
| **7.5** | 「三变体各增 `diagnostic`」 | REQ-BOOT-13 无此要求 + Plan **D-8/D-9** → 上轮已改 |
| **8.1** | 「同一个 `CodeGraphCli` 实例」 | spec `:190`「同一份 CLI 配置」+ Plan **D-16** / **§13** → 本轮已改 |

**经核不属本类（未改，理由逐条）**

- `1.4` 写「在 `provider_capability_store.rs` 抽出 `launch_cell_state` 方法」、`4.1` 写「在 `provider_gateway.rs` 稳定码常量旁新增 `capability_launch_state_token`」：与 Plan **D-2 / D-1** 的落点不同，但 Plan §14 明确「以上 7 条**均为落点调整、语义不变**」，且 `tasks.md` 头部已声明「**精确文件**、命令、测试与提交步骤由 `writing-plans` 写入 `cadence/plans/`；本文件只记高层工作包」⇒ 属落点细化，**不是过规格**（未要求任何超出 spec 的能力/字段/形态）。
- `11.6` 未写 Plan **D-6** 增列的 `GET /api/providers/install-targets`、`5.4` 的候选字段表未含 Plan **D-14 / R11** 的 `AggregateProviderCandidate.warning`：方向相反 —— 是 **tasks.md 欠记**（Plan 增量未回写），属**欠规格/漏记**，不是过规格；本次按边界未改，供 controller 另裁。
- `12.2`「明确不改：`with_snapshot_budget()` 生产接线、…R1 / R2、Degraded 语义、『已初始化』判定文件」与 Plan **§13 / §0-9** 一致；`5.3`「不得指向一键核验全部」与 **§13 / §0-6** 一致；`9.3`（禁 inotify）、`10.3`（禁先整读后计费）、`11.2`（一包一次装）、`11.7`（MUST NOT sudo）、`11.9`（MUST NOT 自动核验）均与 **§13 / §0-2/§0-3** 一致 —— 全部是**约束**而非超出 spec 的要求。
- 其余各条（1.1–1.3、1.5、1.6、2.x、3.x、4.2–4.5、5.2、5.4–5.6、6.1–6.8、7.1–7.4、7.6–7.12、8.2–8.7、9.1–9.5、10.1–10.6、11.1–11.11、12.1–12.8）逐条比对后，要求均落在 spec 的 REQ 或 Plan 的已采纳做法内，**未发现第二种「spec 没要求 + Plan 裁决不做」的过规格措辞**。

⇒ **已通读 12 组全部 85 条；除 5.1 / 7.5 / 8.1 外无同类过规格措辞。**

### 7.4 验收（真实输出）

```
$ npx --no-install openspec validate fix-aggregate-cold-start-and-audit-budget --strict
Change 'fix-aggregate-cold-start-and-audit-budget' is valid
exit=0

$ grep -c '^- \[x\]' openspec/changes/fix-aggregate-cold-start-and-audit-budget/tasks.md
34
$ grep -c '^- \[ \]' openspec/changes/fix-aggregate-cold-start-and-audit-budget/tasks.md
51
$ grep -c '^- \[.\] ' openspec/changes/fix-aggregate-cold-start-and-audit-budget/tasks.md
85
```

改前（本轮前）= 34 / 51；改后 = 34 / 51（34 + 51 = 85 = 总条数）。本轮 5.1 与 8.1 **均为 `[x]`**，只改文字、未触碰任何 `[ ]` / `[x]`。

### 7.5 改动面自证

只改 `tasks.md` 两条（5.1、8.1）与追加本报告；未改 7.5，未改 spec / design / proposal / Plan / 技术方案 / 源码；未 `git add` / `git commit`。

```
$ ls -la openspec/changes/fix-aggregate-cold-start-and-audit-budget/tasks.md
-rw-r--r-- 1 michaelche michaelche 22244 Nov  3 10:13 openspec/changes/fix-aggregate-cold-start-and-audit-budget/tasks.md
$ wc -l -c openspec/changes/fix-aggregate-cold-start-and-audit-budget/tasks.md
  149 22244 openspec/changes/fix-aggregate-cold-start-and-audit-budget/tasks.md
```

字节轨迹：20577（原始）→ 21619（§6 只改 7.5）→ **22244**（本节再改 5.1 + 8.1）；行数恒为 149，条数恒为 85。

本报告「仅追加」自证：本节追加前 `375` 行 / `43796` 字节 / sha256 `e77fe7e9ddb3…0d6d`；追加后前 `43796` 字节的 sha256 仍为 `e77fe7e9ddb3…0d6d`（原有内容逐字节未动）。§6 追加时更早的 32389 字节 sha256 `f415e941…5937` 亦保持不变。

---

## 8. 欠规格补齐（11.6 / 5.4）+ 双向扫描（2026-11-03 追加，controller 批准）

**判据（两个方向，别搞混）**
- **过规格**（任务文字**多了** spec 没要求的东西，且 Plan 有裁决不加/不做）⇒ **改写**任务文字（§6 改 7.5；§7 改 5.1 / 8.1）。
- **欠规格**（任务文字**漏了** spec / Plan 已有的要求）⇒ **先核实实现**：实现存在则补文字；实现不存在则**只报告、不写文字**（避免把「未实现」写成「已完成」）。

### 8.1 11.6 补 `GET /api/providers/install-targets`（依据 Plan **D-6**）

**实现核实：存在**（逐点亲验）

| 证据 | 位置 |
|---|---|
| 路由注册 | `src/web/app.rs:27-28` → `.route("/api/providers/install-targets", get(handlers::get_install_targets))` |
| handler | `src/web/handlers/provider_installation.rs:169` `pub async fn get_install_targets(State(state): State<WebAppState>) -> Json<InstallTargetsResponse>`；模块头 `:3` 同路径说明 |
| 导出 | `src/web/handlers/mod.rs:177` `pub use provider_installation::{get_install_job, get_install_targets, post_provider_install};` |
| 用例 | `src/web/handlers/provider_installation.rs:535-591` `install_targets_endpoint_lists_five_targets_with_actions`（五类对象 + 动作） |
| 前端 client | `web/src/api/provider-install.ts:122` 调 `/api/providers/install-targets`；DTO 见 `web/src/api/types/provider.ts:49` |

**改前（`tasks.md:131`）**

```
- [ ] 11.6 异步安装作业：`POST .../providers/{provider}/install` 立即返回 job_id；`GET .../providers/install/{job_id}` 轮询；作业状态 durable（刷新不丢进度）；流程 = running → 执行 → **装完重新探测**（`provider_health` refresh）→ completed + 结果
```

**改后（`tasks.md:131`）**

```
- [ ] 11.6 异步安装作业：`POST .../providers/{provider}/install` 立即返回 job_id；`GET .../providers/install/{job_id}` 轮询；作业状态 durable（刷新不丢进度）；流程 = running → 执行 → **装完重新探测**（`provider_health` refresh）→ completed + 结果；另列静态路径 `GET /api/providers/install-targets`：一次拿五类对象（四家 provider + codegraph）的本地版本 + 最新版本 + 应显示动作（能力卡一次渲染即可决定安装/更新/核验/无按钮；独立静态路径避免与 `install/{job_id}` 的通配段冲突）——依据块2 Plan **D-6**（原两条路径逐字保留）
```

**依据**：Plan **D-6**「tasks 11.6 只写 `POST .../install` + `GET .../install/{job_id}` | **增列 `GET /api/providers/install-targets`**（一次拿五类对象的本地版本 + 最新版本 + 应显示动作）| 能力卡需要在一次渲染里决定「安装/更新/核验/无按钮」；独立静态路径避免与 `install/{job_id}` 的通配段冲突。**原两条路径逐字保留**」——新增文字里原两条路径确实逐字未动。

### 8.2 5.4 补 `AggregateProviderCandidate.warning`（依据 Plan **D-14 / R11**）

**实现核实：存在**（逐点亲验）

| 证据 | 位置 |
|---|---|
| 字段 | `src/product/logical_codebase/aggregate_initialization.rs:395-404` `pub warning: Option<String>`，`#[serde(default, skip_serializing_if = "Option::is_none")]`（注释「A warning about this candidate (e.g. the known CLI command gaps). `None` means "not verified / normal"」） |
| 填值 | `src/product/logical_codebase/provider_selection.rs:263` 产出、`:341` 写入候选行 |
| 用例 | `provider_selection.rs:786` 断言 `table.candidates[0].warning.as_deref()` |
| Web DTO 同构 | `src/web/types/aggregate_initialization.rs:88-90` `pub warning: Option<String>`；映射 `src/web/handlers/dto/aggregate_initialization.rs:117-127` `warning: candidate.warning` |

**改前（`tasks.md:55`）**

```
- [ ] 5.4 「需要核验」「实测拒绝」「无可用」三类失败的 `details.candidates` 四家齐全、按 owner 顺序，每家含 `{provider, display_name, cli_available, cli_reason_code, capability_state, action_hint}`，`action_hint` 必须是可执行动作
```

**改后（`tasks.md:55`）**

```
- [ ] 5.4 「需要核验」「实测拒绝」「无可用」三类失败的 `details.candidates` 四家齐全、按 owner 顺序，每家含 `{provider, display_name, cli_available, cli_reason_code, capability_state, skipped_reason, action_hint, warning}`，`action_hint` 必须是可执行动作；`warning: Option<String>` 承载「已知 CLI 命令不可解析」等提示（`None` = 未发现异常，**不跳过也不改排序**，`#[serde(default, skip_serializing_if = "Option::is_none")]`）——`warning` 依据块2 Plan **D-14** / 风险项 **R11**
```

**依据**：Plan **D-14 / R11**「①实现侧：`AggregateProviderCandidate.warning: Option<String>`（§2.2 已加），**已知不可解析的一家填原因、不跳过也不改排序**」。

⚠️ **超出批准范围的一处（可单点回退）**：同一次编辑里我把同结构体已有的 **`skipped_reason`** 也列进了字段集（`aggregate_initialization.rs:391-393` 与 DTO `src/web/types/aggregate_initialization.rs:86-88` 均有该字段，属同一类欠记）。若只批 `warning`，把 `skipped_reason, ` 一处删掉即可，其余文字不变。

### 8.3 双向扫描结论（范围：`tasks.md` 组 1–12 全部 **85 条**，行 8–149）

对照权威源：五份 spec delta 的 12 条 REQ 及其**全部场景**、Plan **§13「明确不做」**（8 条）、**§0 全局约束**（13 条）、**§14 D-1 ~ D-16**。

**过规格（3 处，均已改写）**

| 任务 | 原措辞「多了」什么 | 裁决 |
|---|---|---|
| 5.1 | `error_record()` 产出中文主文案 | spec `:111` + Plan **D-5 / D-07**（§7.2） |
| 7.5 | 三热变体各增 `diagnostic` 字段 | REQ-BOOT-13 无此要求 + Plan **D-8 / D-9**（§6.2） |
| 8.1 | 「同一个 `CodeGraphCli` 实例」 | spec `:190`「同一份 CLI 配置」+ Plan **D-16 / §13**（§7.1） |

**欠规格（3 处）**

| 任务 | 漏了什么 | 实现核实 | 本轮处置 |
|---|---|---|---|
| **11.6** | Plan **D-6** 的 `GET /api/providers/install-targets` | **存在**（见 8.1） | ✅ 已补文字 |
| **5.4** | Plan **D-14 / R11** 的 `AggregateProviderCandidate.warning` | **存在**（见 8.2） | ✅ 已补文字 |
| **7.5** | REQ-BOOT-13 场景「成因不可判定时诚实标注」（`spec.md:178-180`）后半句：**「并列出可能的候选供用户自查」+「MUST NOT 把候选当成确定成因呈现」** | **存在**：`failure_copy.rs:210-214`（`cause == "unknown"` ⇒ `reason_zh` 明说「下面列的是候选，不是结论」+ `action_zh` 列三条候选 `provider_unauthorized / provider_command_missing / skills_commands_missing`）；用例 `:543-558` 测试点 #12b | ⛔ **未补**（边界：7.5 上轮已改好、本轮不要动）。**建议**在 7.5 的 `unknown` 从句后追加「，并列出可能的候选供用户自查（MUST NOT 把候选当结论）」 —— 等你点名即补 |

**核过但不列为欠规格（附理由，供复核）**

- `AggregateProviderSelection.decided_at`（`aggregate_initialization.rs:391`）：spec REQ-BOOT-07 只点名 `selected` / `candidates` / `rule_version` 语义，未点名 `decided_at`，无 D 项要求 ⇒ 不属 tasks 级缺口。
- Plan **D-13**「`bytes` 与 `entries` 都按变更量计」：spec REQ-AUD-01 只规定「只对变更文件读内容算哈希」与场景「存量体量不再压垮预检」；任务 **9.2**（口径改本次变更量）+ **9.5**（存量很大但无改动则不失败）已覆盖可观测结果，entries 计数属 Plan 内部实现口径 ⇒ 未单列任务。
- Plan **D-11**（`begin_with_selection`）、**D-12**（`MANAGED_CAPABILITY_SNAPSHOT_REF` → `pub(crate) const`）、**D-10 / D-4**（落地片归属）：均为 Plan 内部落点/施工手段，不是 spec 契约面 ⇒ 不进任务级要求。
- 组 1–4、6、9、10、12 其余各条逐条比对后，要求均能追到具体 REQ 或 Plan 已采纳做法，**未发现第四处过规格或第四处欠规格**。

### 8.4 验收（真实输出）

```
$ npx --no-install openspec validate fix-aggregate-cold-start-and-audit-budget --strict
Change 'fix-aggregate-cold-start-and-audit-budget' is valid
exit=0

$ grep -c '^- \[x\]' openspec/changes/fix-aggregate-cold-start-and-audit-budget/tasks.md
34
$ grep -c '^- \[ \]' openspec/changes/fix-aggregate-cold-start-and-audit-budget/tasks.md
51
$ grep -c '^- \[.\] ' openspec/changes/fix-aggregate-cold-start-and-audit-budget/tasks.md
85
```

11.6 与 5.4 **均为 `[ ]`**（未勾），只改文字，`[x]`=34 / `[ ]`=51 与改前一致。

### 8.5 改动面 + 仅追加自证

只改 `tasks.md` 两条（11.6、5.4）并追加本报告；7.5 / 8.1 / 5.1 未动；未改 spec / design / proposal / Plan / 技术方案 / 源码；未 `git add` / `git commit`。

```
$ ls -la openspec/changes/fix-aggregate-cold-start-and-audit-budget/tasks.md
-rw-r--r-- 1 michaelche michaelche 22894 Nov  3 10:16 openspec/changes/fix-aggregate-cold-start-and-audit-budget/tasks.md
$ wc -l -c openspec/changes/fix-aggregate-cold-start-and-audit-budget/tasks.md
  149 22894 openspec/changes/fix-aggregate-cold-start-and-audit-budget/tasks.md
```

`tasks.md` 字节轨迹：20577（原始）→ 21619（§6 只改 7.5）→ 22244（§7 改 5.1 + 8.1）→ **22894**（本节改 11.6 + 5.4）；行数恒 149、条数恒 85。

**本报告「仅追加」自证（含一处口径澄清）**：本节追加前 **479 行 / 52386 字节 / sha256 `9487d288b385…5d37`**；追加后前 `52386` 字节的 sha256 仍为 `9487d288b385…5d37`。
> 口径澄清：controller 引用的 `e77fe7e9…0d6d` 是**更早一状态**（43796 字节，即 §7 追加前）的 sha256；§7 追加后文件为 52386 字节、sha256 `9487d288b385…5d37`。两个状态各自的前缀在本次追加后**均保持不变**（下方两条校验同时为 True）。

---

## 9. 7.5 补「候选供自查」句（REQ-BOOT-13 场景；controller 批准）+ skipped_reason 保留确认

### 9.1 补句（7.5）

**欠规格性质**：REQ-BOOT-13 的「成因不可判定时诚实标注」场景（`spec.md:178-180`）要求「系统 SHALL 把成因标为 `unknown` **并列出可能的候选供用户自查**；MUST NOT 把候选当成确定成因呈现」，而 7.5 原措辞只写到「标为 `unknown`、MUST NOT 编造成因」，漏了前半句的「列出候选」。

**实现核实：存在**（先核实、后补文字）
- `src/product/logical_codebase/failure_copy.rs:210` 注释：「成因不可判定（`cause == "unknown"`）时的诚实标注：**给候选清单，不给结论**」
- `:213` `reason_zh` = 「已确认这一步失败，但现有诊断里没有任何一条判据能对上具体成因；**下面列的是候选，不是结论**。」
- `:214` `action_zh` = 「按顺序排查候选：provider 未授权（provider_unauthorized）、provider 命令缺失（provider_command_missing）、skills 命令链接缺失（skills_commands_missing）…」
- 用例：`failure_copy.rs:543-558`（测试点 #12b，标题「成因不可判定 → 诚实标注 + 候选清单，不把候选当结论」，`:558` 断言失败消息含「候选被写成了结论」）

**改法**：只插入 controller 指定的一句，7.5 其余文字一字不动。

**改前（片断，`tasks.md:80`）**

```
…不可判定时 SHALL 诚实标为 `unknown`、MUST NOT 编造成因；机制形态 = …
```

**改后（片断，`tasks.md:80`）**

```
…不可判定时 SHALL 诚实标为 `unknown`，并列出可能的候选供用户自查（MUST NOT 把候选当结论）、MUST NOT 编造成因；机制形态 = …
```

**「只加这一句」的字节级自证**：`tasks.md` 22894 → **22969** 字节，差值 **75 字节**，恰等于插入文本 `，并列出可能的候选供用户自查（MUST NOT 把候选当结论）` 的 UTF-8 长度 75；行数恒 149、条数恒 85、`[x]`=34 / `[ ]`=51 未变。

**依据**：REQ-BOOT-13 场景「成因不可判定时诚实标注」（`spec.md:178-180`）+ 实现证据（上表）。

### 9.2 `skipped_reason` 保留确认

controller 裁定：5.4 描述的是**候选字段集**，把同结构体已有的 `skipped_reason`（`aggregate_initialization.rs:391-393`、DTO `src/web/types/aggregate_initialization.rs:86-88`）列入是**更准确**、不是扩范围 ⇒ **保持现状，不回退**。§8.2 中「可单点回退」的提示据此作废（仅作历史记录保留）。

### 9.3 收口状态

- **过规格 3 处**（均已改）：5.1、7.5、8.1
- **欠规格 3 处**（均已改）：11.6、5.4、7.5（本节补句）
- **核过不列 6 项**：`decided_at`、D-13（entries 口径）、D-11、D-12、D-10、D-4 —— 理由见 §8.3
- tasks.md 措辞收口；剩余动作是等 Rust 证据回来后补勾（勾选框本轮未动）

### 9.4 验收（真实输出）

```
$ npx --no-install openspec validate fix-aggregate-cold-start-and-audit-budget --strict
Change 'fix-aggregate-cold-start-and-audit-budget' is valid
exit=0

$ grep -c '^- \[x\]' openspec/changes/fix-aggregate-cold-start-and-audit-budget/tasks.md
34
$ grep -c '^- \[ \]' openspec/changes/fix-aggregate-cold-start-and-audit-budget/tasks.md
51
$ ls -la openspec/changes/fix-aggregate-cold-start-and-audit-budget/tasks.md
-rw-r--r-- 1 michaelche michaelche 22969 Nov  3 10:19 openspec/changes/fix-aggregate-cold-start-and-audit-budget/tasks.md
$ wc -l -c openspec/changes/fix-aggregate-cold-start-and-audit-budget/tasks.md
  149 22969 openspec/changes/fix-aggregate-cold-start-and-audit-budget/tasks.md
```

### 9.5 仅追加自证

本节追加前 **600 行 / 62341 字节 / sha256 `e0a1c94484d8…6b24`**；追加后前 `62341` 字节的 sha256 仍为 `e0a1c94484d8…6b24`。历史三状态前缀同时保持：`52386`=`9487d288…45d37`、`43796`=`e77fe7e9…0d6d`、`32389`=`f415e941…5937`（三条校验全为 True）。未 `git add` / `git commit`；未改 spec / design / proposal / Plan / 技术方案 / 源码。
