# Task 2.3 报告：Revision/ReviewOnly root cwd 分流（映射 tasks.md 2.3 Revision/ReviewOnly 面）

状态：DONE　|　worktree: feat-b-0808-add-monorepo　|　基线 HEAD: 21fae043

## 交付合同

| 面 | 变更 | 语义 |
|---|---|---|
| `review/drive.rs` `drive_revision_session` | 新增 LC 分支：`resolve_revision_root_launch`（gateway 存在时）先于 input 构建解析 root launch + resume 决策；`Some(Ok)` → cwd 重绑 `envelope.working_directory`（canonical root）、target（`input.working_dir`）保持成员 checkout，经 `gateway.start_streaming` 启动（validate→spawn 复验 + audit + tool-policy 接线）后 `drive_provider_session`；`Some(Err)` → 与 input 构建失败同形收口（Error 事件 + finish_failed_run），绝不静默回落 member cwd 直连；`None`（无 gateway）→ Legacy 直连调用图原样（单仓零变化） | Revision 面：cwd=root、独立 target、validated gateway、cwd-inclusive resume fingerprint |
| resume 决策（REQ-ENV-04 cwd 维度） | input 携带旧 native session 且引擎记有其发行 launch 指纹 → `gateway.resume_or_start` 全维度比对：指纹一致（cwd 未漂移）→ resume 放行（native session identity 原样续接、delta prompt）；漂移 → supersede 审计（`resume_fingerprint_mismatch`）+ StartNew（丢 resume id、full prompt 新 thread）。指纹记忆缺失（跨连接重建）→ fail-closed 新会话 | `revision_resume_cwd_drift_supersedes_session` 锁定 |
| `review/drive.rs` `start_review_session_via_gateway`（ReviewOnly） | cwd 从 `input.working_dir`（与 target 同源）迁 `gateway.authority_root()`（构造时自 manifest `provider_context_root` canonicalize，双工厂一致性 Task 2.8 断言）；target 保持独立 `aggregate_root` 锚（成员 worktree）；readable_roots 随 cwd root 化；validated 后 input 显式回填独立 cwd（spawn 前 effective cwd 复验以 envelope 为准） | ReviewOnly 面：cwd 与 target 分离（cwd 不再从 target/worktree 推导） |
| 复用/参数化 | `resolve_author_root_launch` 拆出 `build_root_launch_request(record, provider, action)` 参数化内核（解析链/错误文案与 2.1 字节一致）；revision 与 author 首轮共用同一 envelope/resume 面（PlanningReadOnly + author provider ref + C-2 集中映射） | 不立第二套解析约定 |
| 指纹记忆 | `WorkspaceEngine.logical_launch_fingerprints: HashMap<ProviderName, SessionResumeFingerprint>`（author 面 launch 冻结指纹；author 首轮/follow-up/revision 写入点各一处）；两构造器初始化 | in-memory：跨连接重建缺失按 fail-closed 新会话（不静默续接），持久化留待后续（需 ProviderConversationRef schema，超出本任务文件面） |
| `provider_gateway.rs` | 新增 `authority_root()` 只读 accessor（8 行） | review launch 组装方的独立 cwd 来源 |

## 两个新测试（brief 逐字命名，位于 `author_revision_loop_parts/root_cwd_revision.inc.rs`，1200 行守卫拆分）

- `logical_revision_rebinds_root_cwd_and_preserves_revision_target` ✅ —— LC revision：直连 sentinel 0 启动、gateway 恰 1 启动（audit stream_launches=1）、`working_directory==manifest root`、`working_dir==成员 checkout`（两者显式不等）、`tool_policy==deny_file_write_builtins`、audit_sink 绑定、完成回 AuthorConfirm；envelope 面 target=显式 member/checkout、cwd=root
- `revision_resume_cwd_drift_supersedes_session` ✅ —— 指纹一致轮：resume id `native-revision-r1` 原样续接 + cwd=root + supersede 0；manifest root 漂移后：resume id 丢弃（新 session/full prompt）、cwd=新 root、`supersede_count==1`、`last_supersede_reason=="resume_fingerprint_mismatch"`、直连恒 0

红→绿：先红=E0609（`logical_launch_fingerprints` 未定义；行为面=LC revision 现以 target/worktree 作 cwd 直连）→ 后绿。

## 验证（实际执行）

| 命令 | 结果 |
|---|---|
| `cargo test --locked --lib logical_revision_rebinds_root_cwd_and_preserves_revision_target` | 先红（E0609）→ 1 passed |
| `cargo test --locked --lib revision_resume_cwd_drift_supersedes_session` | 1 passed |
| `cargo test --locked --lib author_revision_loop`（19=17 基线+2 新） | 19 passed |
| `cargo test --locked --lib planning_resume` | 8 passed |
| `cargo test --locked --lib drive_review_session_via_gateway`（ReviewOnly 迁移回归，part_32 三例零改动全绿） | 3 passed |
| `cargo test --locked --lib logical_author_and_choice_followup / legacy_author_choice_followup_bytes`（2.1 回归） | 2 passed |
| `cargo test --locked --lib` | 3967 passed / 0 failed（3 ignored；基线 3965+2 新增。中途 1 failed=`revision_with_existing_author_provider_session_uses_delta_prompt`=本任务 bug（allow_resume 对无 gateway 会话误置 false），修复后全绿） |
| `cargo test --locked --test it_core large_file_guard` | 1 passed（author_revision_loop.rs 784 + 拆分件 631，其余改动文件均 <1200；provider_gateway.rs 1197、lifecycle.rs 1198 贴线未超） |
| rustfmt（改动文件，edition 2024） | 已格式化 |

## 范围边界与 carry

- `provider_run.rs` 零改动（Revision 臂入口不变——LC 分支在 `drive_revision_session` 内部，1200 压线无需拆分）。
- LC 分支不接线 artifact retry 与 Codex resume fallback（两者是直连专用路径，LC 下真实启动唯一经 gateway；失败可见收口不静默换道——与 2.1 author via-gateway 同取舍）。
- ⚠️ 指纹记忆 in-memory 边界：跨连接重建 engine 后 revision resume 恒走新会话（fail-closed，不静默续接旧 thread）；持久化需 ProviderConversationRef 增字段（后续任务/schema 面决策）。窄窗：run 失败于漂移后、cache 已更新而旧对话 id 未被替换时，重试可能把旧 id 误判可续接（失败 run 的对话本就陈旧，影响限于审计语义）。
- ReviewOnly 的成员身份 target（checkout id 形态）与 plan review builders 归 2.4（本任务仅分离 cwd/target，target 维持既有 aggregate_root 锚形态，part_32 契约测试零改动）。
- 追加文件（超出 brief Files 清单，均为最小支撑）：types.rs/lifecycle.rs（字段+初始化）、provider_drive.rs + author_root_launch.inc.rs（指纹写入点+解析器参数化拆分）、provider_gateway.rs（authority_root accessor）。
