# WP4 核账归档登记（REQ-PVR-05，close-provider-validation Task 5）

- 实施日期：2026-09-19
- 计划依据：`cadence/plans/2026-09-18_计划文档_阶段4-C1_provider验证与收尾_v1.0.md`「Task 5」段（v1.1）
- 核账基线：worktree `feat-b-0808-add-monorepo` HEAD=`9314296b`（BASE=`4cc834e2`）
- kimi 状态标注依据：T3 `provider-status-ledger.md`（受限登记）

## §1 add-kimi-code-provider 双向核账（tasks.md 38 行全量，18 任务项无沉默项）

| task 组 | 账面 | 实做锚（2026-09-19 实读复核） | 处置 |
|---|---|---|---|
| 1.1 目录/健康/状态/前端 catalog | 未勾 | `src/web/state.rs:477-478`（生产注册 `KimiCodeProvider::new(PathBuf::from("kimi"))`）+`:513`（测试注册）；`kimi_code_provider/mod.rs:34`（`KIMI_COMMAND="kimi"`）/`:49-56`（`kimi_version_command`/`parse_kimi_version`）/`:71-86`（`ensure_kimi_version_compatible`+`probe_kimi_version`，`MIN_KIMI_VERSION=0.34.0` 经 `provider_health.rs:19` 引入）；`web/handlers/providers.rs:99-102`（DTO：wire=`kimi_code`/display=`Kimi Code`/install_hint 逐字对齐 tasks）+`:249-276` 测试；前端 `web/src/state/provider-options.ts`（catalog+order 含 kimi） | 补勾 |
| 1.2 ProviderType/四映射/四入口/矩阵 | 未勾 | 四映射：`coding_workspace_engine/tool_format.rs:112`、`workspace_engine/mappings.rs:34`、`work_item_split_engine/types.rs:138`、`image_create/models.rs:121`；四入口拒绝：`task_run/provider_factory.rs:73`（`Err(incompatible_output)`，非 `unreachable!`）+`:325` 测试、`task_run/step_runner.rs:116`+`:156` 测试、`web/runtime/provider.rs:499`（`None`=不调度）+`:663-664` 测试、`web/runtime/utils.rs:63`；`adapter_compatibility.rs:68-107` `default_compatibility_matrix` 条目仅 ClaudeCode/Codex=无 Kimi 条目 | 补勾 |
| 1.3 仓库初始化隔离 | 未勾 | `web/src/components/lifecycle/CreateRepositoryDialog.tsx:75-80`：过滤 `kimi_code` 且带 capability policy 注释（「仅 Claude Code 可初始化仓库，Kimi 同 Pi 不参与」） | 补勾 |
| 2.1 ACP 流式会话（resume/退出码/Abort） | 未勾 | `kimi_code_provider/{mod,parse,session}.rs` 在场（session.rs 876 行：initialize→session/new→session/prompt 状态机、`stopReason` 终态判定）；resume=`session.rs:175-190`（`session/load`+digest 漂移拒绝转 new+superseded 审计）；`tests/session_tests.rs`（44K）+`tests/fixtures/`（initialize/text_turn/tool_call_turn/error_exit/acp_request_permission_bash/acp_session_update_tool_call_update）+`tests/fixtures/provider/kimi_acp_*.sh` 8 个脚本 fixture | 补勾 |
| 2.2 Supervised 审批往返 | 未勾 | `session.rs:600` 起 `Parsed::RequestPermission` 分发（`permissionMode` Auto→`auto`/Supervised→`default`；`:735` 起 `has_supported_permission_option`（allow_once/reject_once/reject）回写）；`approval_tests.rs`（40K） | 补勾 |
| 2.3 AskUserQuestion→ChoiceRequest | 未勾 | `session.rs:601-603`（`request.title == "AskUserQuestion"` 分支，`:606-621` 多问题守卫）；`:723-729` `selected_option_response`（`Selected(optionId)` 同轮回写） | 补勾 |
| 3.1 Workspace 角色 Auto/Supervised | 未勾 | `workspace_engine/mappings.rs:34`（`ProviderName::KimiCode → ProviderType::KimiCode`，无 kimi Auto-only 强制——与 pi 的强制分支 `:39-44` 形成对照）；UI 可切面见 6.1 | 补勾 |
| 3.2 guidance 双侧一致 | 未勾 | 后端 `web/workspace_context/prompts.rs:137-138`（KimiCode guidance 含结构化 permission request+AskUserQuestion 纪律）+前端 `web/src/state/workspace-ws-store-guidance.ts:11-12`（同文 guidance） | 补勾 |
| 4.1 Coding 三角色无 Auto-only 强制 | 未勾 | `coding_models/provider_config.rs` grep -i kimi 零命中=无 kimi Auto-only 强制（tasks 要求「不强制」即无专属分支） | 补勾 |
| 4.2 render/kimi_code.rs | 未勾 | `work_item_projection/render/kimi_code.rs` 在场（PROFILE：`provider_label="Kimi Code"`/`renderer_version="kimi-code-provider-projection-renderer-v1"`/Supervised tool hint/structured_output_wrapper）；`render.rs:4,24` 接线 | 补勾 |
| 5.1 image-create | 未勾 | `image_create/models.rs:121`（`From<ProviderName>` Kimi 分支）；`image_create/engine.rs:1029-1038` `image_create_runs_with_kimi_provider`（FakeSessionStore 脚本化 provider 回归）；前端 `web/src/api/types/image-create.ts:37`（dropdown `kimi_code`） | 补勾 |
| 5.2 retry 排除+reviewer repair | **已勾** | `workspace_engine/provider_drive.rs:119,:1134-1143`（kimi artifact retry 排除+`kimi_is_excluded_from_artifact_retry` 测试）；`review/drive.rs:75`（Kimi 复用一次 JSON 等值 repair、Pi 排除）+`:993`（`provider_allows_review_repair(&ProviderName::KimiCode)` 测试）；落实提交=`858fd65e`（2026-08-13，#54 全链路接入，git log -S KimiCode 定位） | 证实 |
| 5.3 Markdown author schema | **已勾** | `workspace_engine/artifact_constraints.rs:124,160,231-232,283`（`[TASK-*]` token_rule=validator-derived contract）；落实提交=`858fd65e` | 证实 |
| 6.1 前端配置展示 | 未勾 | `ProviderConfigPanel`/`CodingProviderConfigPanel` Auto+Supervised 控件（对应 .test 在案见 7.2）；`provider-options.ts:27,34` | 补勾 |
| 6.2 前端穷尽 union/match | 未勾 | 9 个非测试前端文件含 kimi：`api/types/provider.ts:1`（`RealProviderName` union 含 `kimi_code`）、`api/types/image-create.ts:37`、`image-create/SessionList.tsx`、`lifecycle/CreateRepositoryDialog.tsx:79`、`hooks/workspace-ws-message-handler.ts`、`pages/ChatWorkspacePageParts.tsx:377`、`state/provider-options.ts`、`state/workspace-ws-store-guidance.ts:11`、`whats-new/changelog.ts` | 补勾 |
| 7.1 后端回归族 | 未勾 | `provider_health.rs:193-204`（kimi 并行 probe）；`provider_registry.rs:52,148-156`（stable order+Fake 注册断言）；`web/handlers/providers.rs:249-276`（DTO 测试）+`:338`（missing 场景）；`provider_factory.rs:325`（拒绝不 panic）；凭证缺失映射=`kimi_code_provider/mod.rs:37,155-159`（`kimi_authentication_failure`→`kimi login` guidance）+`tests/fixtures/provider/kimi_acp_auth_fixture.sh` | 补勾 |
| 7.2 前端回归族 | 未勾 | 8 个 .test 文件覆盖：`CodingProviderConfigPanel.test.tsx`、`image-create/SessionList.test.tsx`、`lifecycle/CreateRepositoryDialog.test.tsx`（Kimi 被过滤）、`providers/ProviderAvailabilityGuard.test.tsx`、`workspace/ProviderConfigPanel.test.tsx`（Auto+Supervised）、`hooks/useWorkspaceWs.test.tsx`、`hooks/workspace-ws-message-handler.test.ts`、`pages/ChatWorkspacePageParts.test.tsx` | 补勾 |
| 7.3 质量门禁 | 未勾 | T1 `wp1-claude-absorption-ledger.md` §2（2026-09-18，HEAD=d7525220）：`cargo fmt --check` exit=0、`clippy -D warnings` exit=0、`cargo test --locked` lib 3368/0+集成族全绿——唯一红=it_web `web_work_item_plan_mode` 4 例（RR-3 定性：既有基线红，与 add-kimi diff 零交集）；T4 复跑全量 lib 3368/0（`wp3-flaky-eradication.md`）；本日补验前端门禁：`pnpm tsc -b` exit=0、`pnpm test` **176 文件/1509 测试全绿**（2026-09-19） | 补勾（RR-3 既有基线红如实注明，非本 change 面） |

**§1 尾部双向缺口登记**：无「勾而未落」项；无「落而缺证据」项。7.3 的 it_web 4 例红=RR-3 已登记既有基线红（T1 §2 定性在案，与本 change diff 零交集），非 add-kimi 实做缺口。

## §2 pi 遗留 defer 登记（REQ-PVR-05 场景 2，D5）

- add-pi tasks 2.2（aria-ask.ts 结构化提问扩展）：显式 defer——不实施。
  现状限制（如实标注）：pi 结构化提问维持文本暂停信号现状，
  ask_user→extension_ui_request(select)→ChoiceRequest 链路未接入。
- add-pi tasks 2.3（版本范围检测）：显式 defer——不实施。
  现状限制（如实标注）：Pi CLI 版本兼容范围未检测，Extension API
  不兼容时无启动前可操作失败提示。
- 后续触发：如有真实需求另行立项（不预设）。

### add-pi 已勾项抽查补证（抽查面≥每节 1 项）

| 节 | 抽查锚 |
|---|---|
| §1 目录/入口 | `models/provider.rs:8`（`Pi` 变体+serde `\"pi\"` 测试 `:19-21`）；`pi_provider/mod.rs:186-190`（`probe_pi_version[_with_timeout]`）；`web/state.rs:473`（生产注册）；`task_run/provider_factory.rs:68`（Err 拒绝）+`:307` 测试、`step_runner.rs:115`、`web/runtime/provider.rs:498` |
| §2 会话适配 | `pi_provider/{mod,parse,session,usage_tests}.rs` 在场（2.1 面）；2.2/2.3 defer 见上 |
| §3/§4 角色与权限 | `workspace_engine/mappings.rs:33,39-44`（Pi Auto-only 强制「Pi does not support per-tool approval」）；`coding_workspace_engine/tool_format.rs:111`（coding 侧 Pi 映射） |
| §5 前端 | `web/src/state/provider-options.ts:27,34`（Pi 条目） |
| §6 回归 | `tests/it_provider/` 族在场（execution_chain/final_followup_routes/planning_chain 等） |

## §3 唯一 owner 确认（归档前置）

- 确认时间：2026-09-19（本 Task 实施前）
- 确认面：controller 已核——本会话唯一持有三个 change 的归档动作，无并行会话推进，台账无记录（controller 呈报确认在案）；`hub` 并行 agent 面核对（C1T3K3 idle，无归档动作持有者）
- 结论：**无并行持有**，归档动作放行
## §4 归档执行记录（2026-09-19，顺序钉死执行）

### 4.1 add-kimi-code-provider（sync 路径）

| 步骤 | 命令/动作 | 结果 |
|---|---|---|
| instructions | `openspec instructions archive --change add-kimi-code-provider --json` | exit=0，context 读入 |
| status | `openspec status --change add-kimi-code-provider --json` | artifacts 4/4 done（proposal/specs/design/tasks） |
| tasks | tasks.md 勾选态 | 18/18 `[x]`（§1 核账补勾后），0 未勾 |
| validate | `openspec validate add-kimi-code-provider --strict` | **exit=0 干净通过——计划预填的 SHALL-less 警告路径未触发**（delta 全文 SHALL/MUST 齐备），无需带警告归档确认 |
| specs 快照 | `openspec instructions specs --change add-kimi-code-provider --json` | exit=0，rules 2 条（MUST/SHALL+scenario 规范）作用于主 spec 形态——本次为逐字迁移，天然满足 |
| sync | delta `kimi-code-provider-integration` → 主 specs | 新建 `openspec/specs/kimi-code-provider-integration/spec.md`（Purpose 逐字+10 Requirements 逐字，`## ADDED Requirements`→`## Requirements`）；`openspec validate kimi-code-provider-integration --strict` exit=0（仅 INFO 长度提示 ×3） |
| 状态标注 | Step 4.4 按 T3 结论 | 主 spec 尾部追加「验证状态登记」节：**受限登记**（Coder/Code Reviewer 已到真实终态；受限面 1-3+Internal Reviewer 升级路径；不使用「验证失败挂重」措辞；台账引用 provider-status-ledger.md）；追加后复验 strict exit=0 |
| mv | → `openspec/changes/archive/2026-09-19-add-kimi-code-provider/` | `.openspec.yaml` 随目录移动 ✓ |

### 4.2 add-pi-provider（sync 路径，带 defer 警告如实处理）

| 步骤 | 命令/动作 | 结果 |
|---|---|---|
| instructions+status | 同上流程 | exit=0；artifacts 4/4 done |
| tasks | tasks.md 勾选态 | 13/15 `[x]`；2.2/2.3 未勾=**显式 defer**（§2 台账在案，tasks.md 头部核账说明已加；3.6 先例：限制如实标注可归档，controller 已核） |
| validate | `openspec validate add-pi-provider --strict` | exit=0 干净 |
| specs 快照 | `openspec instructions specs --change add-pi-provider --json` | exit=0，rules 同上 |
| sync | delta `pi-provider-integration` → 主 specs | 新建 `openspec/specs/pi-provider-integration/spec.md`（Purpose 逐字+6 Requirements 逐字含尾部 ProviderType 注记）；`openspec validate pi-provider-integration --strict` exit=0 |
| mv | → `openspec/changes/archive/2026-09-19-add-pi-provider/` | `.openspec.yaml` 随目录移动 ✓；**带 2 项未勾任务归档（defer 在案）** |

### 4.3 fix-claude-code-ask-user-question-headless（Archive without syncing——吸收处置，D5/R6）

| 步骤 | 命令/动作 | 结果 |
|---|---|---|
| instructions+status | 同上流程 | exit=0；artifacts 4/4 done；tasks 0/16 勾（被吸收处置——三提交 3164f6a7/e367a84e/20493b65 已于 08-21 落地，tasks 全未勾=吸收态如实保留，非待实施） |
| validate | `openspec validate fix-claude-code-ask-user-question-headless --strict` | **exit=1：5 条 SHALL-less WARNING**（五个 ADDED requirement 均缺 SHALL/MUST）——**D5/R6 判定实证**：该 delta 若 sync 会在主 specs 落 SHALL-less 版本，与 close-provider-validation 的 `claude-code-structured-interaction` delta（SHALL/MUST 齐备的唯一权威版本）形成双版本 |
| sync 决策 | 技能 prompt 处选 **「Archive without syncing」**（计划钉死，controller 在案授权） | 不写任何主 spec；`claude-code-structured-interaction` 维持**零版本**（该 capability 唯一权威版本=close-provider-validation 自归档时落地） |
| mv | → `openspec/changes/archive/2026-09-19-fix-claude-code-ask-user-question-headless/` | 头部「[已被吸收]」标记随文件夹保留 ✓；`.openspec.yaml` 随目录移动 ✓；**带 16 项未勾任务+5 条 SHALL-less 警告归档（均如实登记）** |

### 4.4 归档后校验

- `openspec list`：三 change 不在活跃清单 ✓（close-provider-validation 仍在案 0/19 tasks——controller 验收后统一处理）
- `openspec validate close-provider-validation --strict`：exit=0（归档动作未破坏父 change）✓
- `openspec validate --specs --strict`：kimi-code-provider-integration / pi-provider-integration / kimi-acp-client-services 均 valid 0 errors；总表 19 failed 全部为**其他活跃 change 的既有 delta 问题**（adopt-review-findings/coding-attempt-deletion 等——与本次归档零交集，属 T6/controller 后续处置面，如实登记不扩权）

## §5 无双版本复核（WP1.4 验证口径，2026-09-19 实跑）

```text
$ ls openspec/specs/ | grep -E 'kimi-code-provider-integration|pi-provider-integration|claude-code-structured-interaction'
kimi-code-provider-integration
pi-provider-integration
$ ls openspec/changes/ | grep -E 'add-kimi|add-pi|fix-claude-code' || echo "three-folders-archived"
three-folders-archived
$ ls openspec/changes/archive/ | grep 2026-09-19
2026-09-19-add-kimi-code-provider
2026-09-19-add-pi-provider
2026-09-19-fix-claude-code-ask-user-question-headless
```

**判定：PASS**——主 specs 恰含 `kimi-code-provider-integration`（10 Requirements+受限登记标注）+`pi-provider-integration`（6 Requirements）各唯一版本；`claude-code-structured-interaction` **不在主 specs**（零版本=无双版本；唯一权威版本待 close-provider-validation 自归档时落地）；changes/ 下三文件夹已移除；三归档目录（含 `.openspec.yaml` 与头部吸收标记）完整。

