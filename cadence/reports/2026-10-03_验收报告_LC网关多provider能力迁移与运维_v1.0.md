# 验收报告:LC 网关多 provider 能力迁移与运维 v1.0

> 计划:cadence/plans/2026-10-02_实施计划_LC网关多provider接入_v1.0.md §Task 14(14b Evidence)。
> 数据口径:四家终局矩阵全部 52 格从 `cadence/reports/lc-gateway-multi-provider/{provider}/matrix/**/cell.json` 实读(2026-10-09 汇总);实测版本/耗时取 cell.json `exact_version`/`timeline`;边界/OS 事实取 `boundary/*/evidence.json`;预算常量取 HEAD 源码。本文同时承载运维说明与永久行为变更记录,不另建重复 changelog。

## 1. 结论摘要

- 四家 provider(claude-code/codex/pi/kimi-code)在实测版本、Linux、真实 LC 五阶段入口上的可支持格全部 `Confirmed`;两格产品级不可支持(review resume、split_sync resume)带真实拒绝证据。
- 实测支持**仅逐格成立**:格 = provider × exact version × OS × action × entrypoint × fresh/resume。未实测的更新版本、Windows/macOS launcher、大 LC 新成本一律 `Unknown`,永不默认支持(`lcg_t14_unverified_os_version_or_cost_is_never_default_supported` 钉死)。
- 存量 v1 记录(无 action matrix)读取为全 Unknown,不产生正常会话 Confirmed;CLI 升级后旧行整体失效为 Unknown 并阻断至真实重探,probe 历史工件永不删除(`lcg_t14_cli_upgrade_keeps_history_and_blocks_until_reprobe` 钉死)。
- 迁移等待面已冻结为稳定判别码 + 逐维 missing materials + 恒定 Revalidate/Retry(`lcg_t14_legacy_matrix_unknown_projects_actionable_waiting` 钉死);不提供手工 JSON 改 Confirmed 的操作,也没有「迁移默认 allow」。

## 2. 冻结稳定判别码(与代码常量同源)

迁移等待面冻结清单(`PROVIDER_CAPABILITY_MIGRATION_REASON_CODES`,provider_capability_store.rs;诊断投影 `detailed_reason_code` 逐枚消费):

| 判别码 | 含义 |
|---|---|
| `provider_unsupported_for_gateway_launch` | provider 无 gateway 真实 dialect 映射(Fake/未知值 fail-closed) |
| `provider_capability_action_unknown` | action 行未探测(Unknown)/版本漂移失效,等待真实 probe |
| `provider_resume_unsupported` | resume 分格未 Confirmed,不得静默转 fresh |
| `provider_trust_missing` | workspace trust 未建立 |
| `provider_write_boundary_unverified` | write_boundary 分格未验证 |
| `provider_adapter_dialect_mismatch` | wire dialect 漂移,行整体失效 |
| `codex_danger_full_access_unsupported` | Codex danger-full-access 永久阻断 |
| `codex_target_boundary_unverified` | Codex target-only boundary 证据缺失 |

并存的 gateway 侧分格码:`provider_capability_launch_not_confirmed`、`provider_capability_write_boundary_not_confirmed`;policy/body 漂移码沿用 gateway Display 判别码(`provider_gateway_policy_drift: policy_body|policy_digest_chain|...`)与 admission waiting reason(`policy_digest_drift`/`authority_root_drift`/`provider_policy_artifact_missing`),不另立第二套。
admission 对 capability 拒绝的既有 waiting 面保持冻结:reason `provider_capability_not_satisfied` + allowed actions 恒 `[Revalidate, Retry]`,材料面不伪造 capability 证据材料。

## 3. 四家终局支持矩阵(52 格,cell.json 实读)

版本与 OS(逐格支持仅在此事实内成立):claude-code `2.1.283 (Claude Code)`、codex `codex-cli 0.155.1`、pi `pi 0.86.1`、kimi-code `kimi 2.0.2`;OS 全部为 `Linux 6.18.25-x64v3-xanmod1` x86_64(boundary evidence.json `os` 字段;bwrap `bubblewrap 0.11.2` + user namespace 实测可用)。

状态说明:`confirmed` = 真实会话证据落盘;`carried` = 承继格(不重新计票,证据链指向来源轮快照);`denied` = 产品级不可支持证据。时长 = cell.json timeline 首末事件差(秒);事件数为流量代理指标(cell.json 不含 token 计量,token 面未实测)。

| provider | stage | fresh/resume | entrypoint | action | state | reason/denied | 时长 | 事件数 |
|---|---|---|---|---|---|---|---|---|
| claude-code | story | fresh | workspace_streaming_plan/split | — | carried | 承继格:快照 20261008T130537Z,构建 9cafa49b | — | 0 |
| claude-code | story | resume | workspace_streaming_plan/split | planning_read_only | confirmed | — | 113s | 520 |
| claude-code | design | fresh | workspace_streaming_plan/split | — | carried | 承继格:同上 | — | 0 |
| claude-code | design | resume | workspace_streaming_plan/split | planning_read_only | confirmed | — | 142s | 520 |
| claude-code | plan | fresh | split_sync | planning_read_only | confirmed | 真实探针(spawn=1) | 0s | 2 |
| claude-code | plan | fresh | workspace_streaming_plan/split | — | carried | 承继格:同上 | — | 0 |
| claude-code | plan | resume | split_sync | planning_read_only | denied | sync split 无 native session contract:resume=Unknown,零 spawn(provider_start=0) | — | 0 |
| claude-code | plan | resume | workspace_streaming_plan/split | review_read_only | confirmed | — | 469s | 555 |
| claude-code | split | fresh | split_sync | — | carried | 承继格:同上 | — | 0 |
| claude-code | review | fresh | workspace_streaming_plan/split | planning_read_only | confirmed | — | 802s | 526 |
| claude-code | review | resume | workspace_streaming_plan/split | planning_read_only | denied | 服务端错误帧收口(protocol_error):INVALID_MESSAGE_FOR_STAGE,request_revision not allowed in stage completed | 0s | 3 |
| claude-code | coding | fresh | workspace_streaming_plan/split | review_read_only | confirmed | — | 2235s | 10685 |
| claude-code | coding | resume | workspace_streaming_plan/split | review_read_only | confirmed | native id 同 id 恢复确认 | 0s | 1 |
| codex | story | fresh | workspace_streaming_plan/split | — | carried | 承继格:快照 20261008T232626Z,构建 b44a44ae | — | 0 |
| codex | story | resume | workspace_streaming_plan/split | planning_read_only | confirmed | — | 32s | 520 |
| codex | design | fresh | workspace_streaming_plan/split | — | carried | 承继格:同上 | — | 0 |
| codex | design | resume | workspace_streaming_plan/split | planning_read_only | confirmed | — | 25s | 520 |
| codex | plan | fresh | split_sync | planning_read_only | confirmed | 真实探针(spawn=1) | 0s | 2 |
| codex | plan | fresh | workspace_streaming_plan/split | — | carried | 承继格:同上 | — | 0 |
| codex | plan | resume | split_sync | planning_read_only | denied | sync split 无 native session contract:resume=Unknown,零 spawn | — | 0 |
| codex | plan | resume | workspace_streaming_plan/split | review_read_only | confirmed | — | 89s | 569 |
| codex | split | fresh | split_sync | — | carried | 承继格:同上 | — | 0 |
| codex | review | fresh | workspace_streaming_plan/split | planning_read_only | confirmed | — | 132s | 519 |
| codex | review | resume | workspace_streaming_plan/split | planning_read_only | denied | INVALID_MESSAGE_FOR_STAGE(同 claude-code,产品无「评审续开」) | 0s | 3 |
| codex | coding | fresh | workspace_streaming_plan/split | review_read_only | confirmed | — | 380s | 3913 |
| codex | coding | resume | workspace_streaming_plan/split | review_read_only | confirmed | thread 同 id 恢复确认 | 0s | 1 |
| pi | story | fresh | workspace_streaming_plan/split | — | carried | 承继格:快照 20260909T032236Z 轮,构建 e78a97c6 | — | 0 |
| pi | story | resume | workspace_streaming_plan/split | planning_read_only | confirmed | — | 18s | 519 |
| pi | design | fresh | workspace_streaming_plan/split | — | carried | 承继格:同上 | — | 0 |
| pi | design | resume | workspace_streaming_plan/split | planning_read_only | confirmed | — | 20s | 519 |
| pi | plan | fresh | split_sync | planning_read_only | confirmed | 真实探针(spawn=1) | 0s | 2 |
| pi | plan | fresh | workspace_streaming_plan/split | — | carried | 承继格:同上 | — | 0 |
| pi | plan | resume | split_sync | planning_read_only | denied | sync split 无 native session contract:resume=Unknown,零 spawn | — | 0 |
| pi | plan | resume | workspace_streaming_plan/split | review_read_only | confirmed | — | 405s | 698 |
| pi | split | fresh | split_sync | — | carried | 承继格:同上 | — | 0 |
| pi | review | fresh | workspace_streaming_plan/split | planning_read_only | confirmed | — | 64s | 519 |
| pi | review | resume | workspace_streaming_plan/split | planning_read_only | denied | INVALID_MESSAGE_FOR_STAGE(同上) | 0s | 3 |
| pi | coding | fresh | workspace_streaming_plan/split | review_read_only | confirmed | — | 529s | 15916 |
| pi | coding | resume | workspace_streaming_plan/split | review_read_only | confirmed | session id 同 id 恢复确认 | 0s | 1 |
| kimi-code | story | fresh | workspace_streaming_plan/split | — | carried | 承继格:快照 20261009T070600Z,构建 46cd5465 | — | 0 |
| kimi-code | story | resume | workspace_streaming_plan/split | planning_read_only | confirmed | — | 33s | 519 |
| kimi-code | design | fresh | workspace_streaming_plan/split | — | carried | 承继格:同上 | — | 0 |
| kimi-code | design | resume | workspace_streaming_plan/split | planning_read_only | confirmed | — | 21s | 519 |
| kimi-code | plan | fresh | split_sync | planning_read_only | confirmed | 真实探针(spawn=1) | 0s | 2 |
| kimi-code | plan | fresh | workspace_streaming_plan/split | — | carried | 承继格:同上 | — | 0 |
| kimi-code | plan | resume | split_sync | planning_read_only | denied | sync split 无 native session contract:resume=Unknown,零 spawn | — | 0 |
| kimi-code | plan | resume | workspace_streaming_plan/split | review_read_only | confirmed | — | 171s | 597 |
| kimi-code | split | fresh | split_sync | — | carried | 承继格:同上 | — | 0 |
| kimi-code | review | fresh | workspace_streaming_plan/split | planning_read_only | confirmed | — | 170s | 526 |
| kimi-code | review | resume | workspace_streaming_plan/split | planning_read_only | denied | INVALID_MESSAGE_FOR_STAGE(同上) | 0s | 3 |
| kimi-code | coding | fresh | workspace_streaming_plan/split | review_read_only | confirmed | — | 488s | 9787 |
| kimi-code | coding | resume | workspace_streaming_plan/split | review_read_only | confirmed | ACP session/load 同 id 恢复确认 | 0s | 1 |

evidence_ref:每格证据即上表路径下的 `cell.json`(+`frozen-facts.json`/`pre|post-snapshot.json`/`provider-events.jsonl`/`manifest.sha256`),例如 `cadence/reports/lc-gateway-multi-provider/claude-code/matrix/coding/workspace_streaming_plan-split/fresh/cell.json`。
两格 denied 为四家一致的产品级证据:review resume(产品无「评审续开」功能,用户裁定留待未来 change)与 split_sync resume(计划冻结语义:sync split 无 native session contract,resume=Unknown 且零 spawn)。

边界 probe 证据(写边界 Confirmed 的来源):四家 `boundary/{coding_target_write,planning_read_only}-*/evidence.json` + `shape-validation.json`,最终会话 `20261005T171*` 轮;写边界验收详见《2026-10-03_验收报告_LC网关多provider写边界_v1.0.md》。

## 4. 实测耗时/流量汇总(cell.json timeline)

- 单格 fresh 重头:claude-code coding fresh 2235s/10685 事件;pi coding fresh 529s/15916 事件;kimi coding fresh 488s/9787 事件;codex coding fresh 380s/3913 事件。
- resume 格(可恢复阶段)普遍 18–171s/约 520 事件;plan 聚合评审 resume 405–469s。
- token 计量:cell.json 无 token 字段,token 面未实测(如实注明,不以事件数冒充)。

## 5. 迁移等待面与操作顺序(运维 SOP)

对任一 `Unknown`/漂移格,按以下顺序操作,**不存在手工 JSON 改 Confirmed 的捷径,也没有「迁移默认 allow」**:

1. **补前置**:按 waiting `missing_materials` 逐维补齐前置(trust 建立、规则/政策材料、CLI 在位);
2. **真实 probe**:运行真实 boundary/version probe(如 `boundary/` 探针),产出 evidence 工件;
3. **校验证据**:三方一致性(shape validation:evidence/projection/record 的 version/digest/artifact_ref 逐字段);
4. **产品 Revalidate/Retry**:经产品动作 `BootstrapActionKind::Revalidate`/`Retry` 重走 admission(等待面 allowed actions 冻结为这两项,不扩大);
5. **完整 fresh admission**:导入 verified probe 行(`import_verified_probe_row`,原子落盘)后,完整 fresh admission 放行;resume 仍须 resume 分格独立 Confirmed。

CLI 升级(版本漂移)语义:旧行整体失效为 Unknown,阻断至重探;旧行 durable 字节与旧 probe 工件原样保留(历史可审计,任何迁移步骤不删除工件);重探导入新行后解除阻断。

## 6. 预算与成本

- 既有机械预算证据保留沿 HEAD:probe 快照预算 `RootRecipeSnapshotBudget::DEFAULT` = 20_000 条目/64 MiB(root_recipe_receipt_types.inc.rs);inventory 预算 soft 4096B/hard 8192B(`COMPACT_INVENTORY_{SOFT,HARD}_BUDGET_BYTES`,aggregate_index/freshness.rs);boundary 观测面实测 ≈1.1–1.4KB/8192B。
- 大 LC(更多成员仓/更深聚合)的新成本未实测,为 Unknown;本报告不把当前发现矩阵外推为五阶段安全保证。
- Pi 最低 0.83.0 / Kimi 最低 0.34.0 是旧产品底线(历史版本探测口径),不是本矩阵的支持声明;本矩阵支持仅限上表实测版本。

## 7. 边界与分层事实

- **Codex 共享 home 撤销 digest 敏感边界**:`~/.codex` 承载 sessions、projects trust 与 config,是 boundary launcher 唯一放行的 HOME 下窄面写目录之一(`PROVIDER_RUNTIME_WRITABLE_DIRS`,provider_boundary.rs:188-199,只绑存在的、不得整 HOME 宽写)。撤销/清理 `~/.codex`(如撤销 workspace trust)同时改变 trust digest 与 native thread 状态,而 resume fingerprint 含 version/dialect/trust/projection digest(design.md)——撤销后旧 Confirmed/resume 证据立即失效,必须重探;该目录共享于宿主用户会话,产品侧不提供自动恢复或旁路。
- **Kimi native/Aria MCP 分层**:Kimi 不消费通用 tool policy,沿 ClientServicePolicy 生成 role policy;Aria 注入 MCP bundle 与原生项目 MCP 自发现**分开审计、明确分层,不伪造统一控制面**(design.md);resume fingerprint 仅纳入 Aria 注入通道的 bundle digest。当前 TerminalIsolation 不可用,无 OS boundary 证据不放行。
- **#8 状态**:policy authority-root 正文发布通道(#8)已于 2026-10-03 收口(commit c4706aa9);policy/body 漂移码(`provider_gateway_policy_drift: policy_body`、digest 链维度)在 gateway 预检/复验中保持 fail-closed 消费。
- **无 fixture 门**:E2E/边界矩阵测试全部 `#[ignore]`,仅在 `LC_GATEWAY_E2E=1` 且真实 CLI/trust/#8 政策就绪时执行(boundary_matrix.rs/failure_matrix.rs);常规单测不冒充真实现场,`test_provider_enabled` 只按角色豁免 Fake(Pi/KimiCode 不豁免)。

## 8. 未实测面(Unknown)清单

- 未实测的更新 CLI 版本(任何 ≠ 上表 exact_version):行读取 Unknown,永不默认支持;
- Windows / macOS launcher:无证据,Unknown;
- 大 LC(更大成员数/更深聚合)成本:未实测,Unknown;
- token 计量面:无数据,Unknown。

## 9. 永久行为变更记录(Task 14 落地)

1. `provider_capability_store.rs`:新增冻结判别码常量族 + `PROVIDER_CAPABILITY_MIGRATION_REASON_CODES` 全集 + `CapabilityMigrationWaiting` 迁移等待投影(`ProviderCapabilityRecord::migration_waiting`):只读诊断,逐维 missing materials(version/wire/projection/OS/预算/trust),allowed actions 恒 `[Revalidate, Retry]`,全 Confirmed 行返回 None;不写 store、不铸 Confirmed。
2. `automation_gateway_preflight.rs`:`detailed_reason_code` 先逐枚消费冻结迁移判别码全集,再沿既有 gateway 分格码;policy/body 漂移 Display 判别码透传;未命中回退 waiting reason(零回归)。
3. 测试锁定:`lcg_t14_legacy_matrix_unknown_projects_actionable_waiting`(先红后绿)、`lcg_t14_cli_upgrade_keeps_history_and_blocks_until_reprobe`、`lcg_t14_unverified_os_version_or_cost_is_never_default_supported`、`lcg_t14_diagnostic_projection_consumes_frozen_migration_reason_codes`。
4. admission 既有 waiting 面零变更:capability 拒绝仍投影 `provider_capability_not_satisfied` + `[Revalidate, Retry]`,材料面不伪造 capability 证据材料。
