# Task 2.1 报告：Author/ChoiceFollowup root launch（映射 tasks.md 2.3 Author 面）

状态：DONE　|　worktree: feat-b-0808-add-monorepo　|　基线 HEAD: 9457200b → 提交 8f80896e

## 交付合同

| 面 | 变更 | 语义 |
|---|---|---|
| `provider_run.rs` Author/AuthorChoiceFollowup 两臂 | 改调 `handle_user_message_from_run`/`handle_author_choice_followup_from_run`（携带 `session_record`）；四路由入口 match 分发保持主文件 | LC 会话经 root launch；单仓零变化（Revision/ReviewOnly 臂未动，归 2.3/2.4） |
| `provider_drive.rs` + 新 `provider_drive/author_root_launch.inc.rs` | `WorkspaceEngine::resolve_author_root_launch(&self, &WorkspaceSessionRecord) -> Option<Result<ValidatedSessionLaunchPolicy, String>>`；`handle_author_message_with_prompt_mode` 增 `gateway_launch` 参数 | `None`=非逻辑会话（未注入 gateway）→ Legacy 直连原样；`Some(Ok)`=LC 分支：`input.working_directory = Some(envelope.working_directory)`（root cwd），`input.working_dir` 保持成员 checkout（target），组装 `ValidatedStreamingProviderInput` 经 `drive_author_provider_session_via_gateway`（gateway validate→start_streaming，audit/tool-policy 接线在 via_gateway 内部）；`Some(Err)`=解析/校验 fail-closed——与 input 构建失败同形（Error 事件 + finish_failed_run），绝不静默回落 member cwd 直连 |
| root cwd 来源 | `RepositoryAuthorityResolver::resolve_for_issue` → `manifest.provider_context_root` | cwd=canonical root（envelope.authority_root 同源）；target=`workspace_repository_for_session` 唯一成员 checkout（Story focus/Design involved/Plan selection，与 manager 建链同源），身份成对时 `PolicyTarget::checkout`（显式 member/checkout），否则镜像 gateway_start.rs deferred 口径 `aggregate_root` 锚 |
| 策略/resume | action=`PlanningReadOnly`、`tool_policy` 不变（deny_file_write_builtins）、provider ref 经 `ProviderRef::from_provider_name`（C-2：Pi/KimiCode 显式失败） | choice 内容不构造新 target、follow-up 不降为无策略 Executor；同会话两次解析 envelope/fingerprint 相等（root cwd 已纳入 `SessionResumeFingerprint`，Task 2.5 字段） |
| carry 接线（2.8→2.1） | `provider_admission_check.inc.rs` 步骤 6 复验 cwd 由 `request.target.worktree` 改 `request.working_directory`（保留相对路径 join manifest root 口径） | cwd≠target 的 root 形态经 admission 复验；现状消费方（聚合 driver、SC 预检）cwd==target 零变化 |

兼容边界：`handle_user_message`/`handle_author_choice_followup_message` 公开签名不变（内部委托 `None`，30+ 既有测试调用点零改动）；无 gateway 注入的 engine 完全走原路径。

## 行数守卫（拆分处置）

`provider_drive.rs` 1197→1118（搬移 attach_tool_policy_audit×2 + drive_author_provider_session_via_gateway 至新 inc，含新增解析器/入口）；`provider_run.rs` 1195→1200（注释收紧 −2、两臂改写 +7）；`provider_run_events.rs` 1070→1072（doubles+两测试主体拆入新 `provider_run_events_parts/root_cwd_launch.inc.rs` 249 行，include! 同模块域，测试路径/名不变）。`it_core large_file_guard` 1 passed。

## 两个新测试（brief 逐字命名）

- `logical_author_and_choice_followup_keep_root_cwd_and_target_separate` ✅——事件驱动真 spawn 链（`spawn_provider_run_from_event`）：两轮 input 的 `working_directory==canonical root`、`working_dir==成员 checkout`、策略未放宽、follow-up 内容原样；直连 sentinel 0 启动（不绕 gateway）、gateway 恰 2 启动；envelope 断言 target 显式 member id/checkout、cwd=root、两次解析 fingerprint 相等
- `legacy_author_choice_followup_bytes_and_cwd_are_unchanged` ✅——单仓两轮 `working_directory==None`、`working_dir==repository_path`、策略不变、直连 2 启动、DeltaOnly 透传

红→绿：先红=E0599（`resolve_author_root_launch` 未定义；行为面=LC 会话现走 repository_path 作 cwd 直连）→ 后绿。

## 验证（实际执行）

| 命令 | 结果 |
|---|---|
| `cargo test --locked --lib logical_author_and_choice_followup_keep_root_cwd_and_target_separate` | 先红（E0599）→ 1 passed |
| `cargo test --locked --lib legacy_author_choice_followup_bytes_and_cwd_are_unchanged` | 1 passed |
| `cargo test --locked --lib author_revision_loop` | 17 passed（Revision 面零回归） |
| `cargo test --locked --lib` | **3965 passed / 0 failed**（3 ignored；基线 3963+2 新增。中途一次 2 failed=已知负载 flaky enrolled_start_coding 两例（progress 登记），复跑两次均绿） |
| `cargo test --locked --test it_core large_file_guard` | 1 passed |
| `cargo test --locked --lib provider_run_events / single_candidate_lc_admission / lc_` | 12 / 3 / 26 全绿 |
| `cargo test --locked --test it_web provider_gateway_envelope` | 6 passed（admission carry 消费面） |
| rustfmt（改动文件，edition 2024） | 已格式化；entry.rs:95 与 provider_run.rs:63 为存量漂移未触碰 |

## 范围边界与 carry

- Revision/ReviewOnly 迁移归 2.3/2.4（`review/drive.rs`、`drive_revision_session`、ReviewOnly 臂均未动）；`LogicalPlanLaunch`/`logical_planning_launch` 的 root 化（planning 面 Story/Design/Plan 首轮）归 impl-split 2.2——本任务以 `SessionLaunchRequest` 结构体字面量显式注入 root cwd（provider_gateway.rs 注释预留的 Task 2.2/2.8 接线口径），2.2 落地后可回收为统一 launch 类型。
- gateway 注入但 `repository.logical_repository_id` 缺失（与 manager 注入谓词不一致）→ fail-closed（新增防线，无既有测试命中）。
- `provider_run.rs` 现 1200 行（守卫上限）；后续任务追加需先拆分。
