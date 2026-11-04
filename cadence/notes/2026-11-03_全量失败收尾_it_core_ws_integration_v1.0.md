# 全量失败收尾：it_core workspace_ws_integration 74 条（v1.0 · 终态）

日期：2026-11-03（终态更新：2026-11-04）· 工作树：`.worktrees/bugfix-b-11-02` · 范围：`cargo test --locked --test it_core workspace_ws_integration`

## 结论速览（终态）

| 阶段 | passed / failed | 说明 |
|---|---|---|
| 基线（Main 实测） | 0 / 74 | issue 创建 400 `issue_not_bound_to_logical_codebase` |
| 第一层：夹具 LC 归属修复后 | 26 / 48 | 夹具链全通；剩 46 条 A 类（gateway 拒 Fake）+ 2 条 health |
| 交接时点（第二层被截停） | 53 / 21 | 三路子 agent 落盘状态未核实 |
| 本次第一遍迁移后 | 69 / 5 | 补齐替身 `start_validated` + 逐条换名/换 health |
| **终态** | **73 / 1** | **仅剩 Pi 条留红（Main 已裁定暂留红待立项）** |

终态命令与实测输出：

```
flock /tmp/aria-cargo.lock -c 'cargo test --locked --test it_core workspace_ws_integration -- --nocapture --test-threads=4'
test result: FAILED. 73 passed; 1 failed; 0 ignored; 0 measured; 124 filtered out; finished in 92.52s
failures:
    workspace_ws_integration::workspace_ws_abort_with_pi_reaches_cancelled_state_and_stops_output
```

**唯一红条 = Pi 条**：失败点已从「adapter 不支持 validated start」移到 Main 钉住的那条断言本身（`seen_inputs` 空 vs `[(Pi, Auto)]`），即 Main 裁定的「LC gateway 的 validated 启动无等价可观察点」现场（见 §6）。

---

## 1. 三路子 agent 落盘状态核实结论（交接时的真实状态）

核实方式：`git status --porcelain` + `git diff --stat` + 逐替身 `grep 'fn start_validated'` + 全量替身清点 + 编译核对。

| 子 agent | 负责 | 落盘状态 | 结论 |
|---|---|---|---|
| ConvertPart02 | part_01 / 02 / 03 | part_01 **只落了 import 块**（+13 行，零用例转换）；part_02 **零落盘**；part_03 落了 8 条用例转换 | **半成品**：part_01/part_02 未做，part_03 完整 |
| ConvertPart03 | part_03 | part_03 落盘（与 ConvertPart02 在 part_03 上重叠，无冲突/重复，diff 干净、编译绿） | **完整** |
| ConvertPart08 | part_08 / 09 | part_08 只转了 **1 条**用例 + `ChoiceHangingBroadcastProvider` 的 `start_validated`；part_09 **零落盘** | **半成品** |
| ConvertPart06 | part_06 / 06b | 16 条全转，编译绿 | **完整**（唯一残留 1 条根因在 part_04 的替身，已补） |

**交接时全族 12 个替身，只有 7 个有 `start_validated`**（part_05×5 + part_06×1 + part_08×1）。**5 个替身缺失**（三路 agent 均未覆盖）：

1. `PiHangingStreamingProvider`（part_05）
2. `ScriptedStreamingProvider`（part_05）
3. `WorkingDirRecordingStreamingProvider`（part_04）
4. `ChoiceThenArtifactProvider`（part_04）
5. `TextFallbackChoiceCompletingProvider`（part_08）

### 1.1 一个「只改了一半的替身」（半成品实例）

`part_05.rs` 的 `impl ScriptedStreamingProvider { fn gateway_session(...) }` 是**从 `ChoiceThenHangingStreamingProvider` 复制粘贴的错版**：它发的是 `ChoiceRequest(id="choice_hanging_001")`，而 `ScriptedStreamingProvider` 的真实行为是弹出脚本化输出、发 `TextDelta` + `Completed`。编译绿（只是 `dead_code` 警告），但一旦接上就是行为错。⇒ 已重写为 `scripted_session(output, contract)`（行为回归脚本化输出本体），并补 `start_validated`。

### 1.2 我补的东西

- 5 个缺失替身全部补齐 `start_validated`（共享方法 + 拆 input 后转发，不回调裸 start）。
- 18 条用例按已批准的 4 步固定手法迁移（夹具 `_with_author`/`_with_providers` → 替身注册名 Fake→ClaudeCode/Pi → `start_validated` → state 包 `with_available_provider_health`）。

---

## 2. 第一层（夹具 LC 归属）——已完成并复核（沿用 v1.0 结论，本轮未改动）

- 共用夹具唯一汇聚点：`tests/it_core/workspace_ws_integration/part_05.rs` `create_workspace_session_fixture_with_providers`（61 直调 + 13 变体，74 条无分叉）。
- 五类前置全部走生产形态：① `IdentityMigrationExecutor::ensure_identity_schema` 建真 LC 权威；② 生产 `aggregate-indexes/rebuild` 端点建索引；③ issue 请求补 `logical_codebase_id`；④ `git_repo` 补 `src/main.rs`（成员源码覆盖规则）；⑤ 四家 capability 行 Confirmed + involved 钉定。
- 五类错误码已清零：`issue_not_bound` / `aggregate_index_active_missing` / `member_coverage` / `launch_not_confirmed` / `involved_undetermined`。

## 3. 第二层（gateway 拒 Fake 的测试侧迁移）——本轮主体

### 3.1 根因（生产设计，不是夹具缺料）

块3 后 issue 恒 LC 归属 → story 会话 `aggregate_root_view` 带 logical id → 生产 manager 恒注入 gateway → `ProviderRef::from_provider_name` 对 `Fake` fail-closed（`provider_gateway.rs`：注释明言「Fake/测试路径不经此 gateway」+ 禁止静默回退）。⇒ **74 条以 Fake 脚本 provider 驱动会话的写法与「LC 会话必经 gateway」的生产设计冲突**，只能在测试侧迁移。

### 3.2 四步固定手法（本次全部沿用，未另造）

```
① 夹具调用换 _with_author(&root, "claude_code")（或 _with_providers(root, "claude_code", "<reviewer>", 1)）
② 替身注册名 Fake → ClaudeCode（reviewer 侧另有约束，见 §5）
③ 替身抽共享方法补 start_validated（拆 input 后转发，不回调裸 start）
④ state 构造包 with_available_provider_health（factory 直挂 always-available gate）
```

`with_available_provider_health` 的根因是**真实 health service 未探测恒 degraded**（不是 builder 时序）⇒ gateway launch 分格 `provider_gateway_unavailable`；照 `tests/it_web/issue_delivery_summary.rs` 先例，factory 直挂 always-available gate（it_web 该 helper 为 it_web target 私有，it_core 取不到，按其口径在 part_05 复刻）。

### 3.3 `ChoiceThenArtifactProvider` 的 `start_validated`（本次指定项）

替身事实：`part_04.rs` `ChoiceThenArtifactProvider`，消费 `input.resume_provider_session_id` / `input.prompt`，第 1 轮吐文本提问、第 2 轮吐 Story Spec；被 `part_06.rs` 的 `workspace_ws_cursor_reconnect_replays_pending_choice_after_replay_window` 使用（此前阻塞：替身无 `start_validated` ⇒ gateway 返回「streaming provider validated start is not supported by this adapter」）。

做法（与其余替身**同型**，未另造一套）：

1. 抽共享方法 `fn choice_then_artifact_session(&self, resume_provider_session_id: Option<String>, prompt: String) -> ProviderSession` —— 裸 `start` 与 `start_validated` 共用同一会话本体（记录 resume id / prompt、计数取输出、构造同一 ProviderSession）。
2. `start`：`Ok(self.choice_then_artifact_session(input.resume_provider_session_id.clone(), input.prompt))`（行为零漂移）。
3. `start_validated`：**拆 input 后转发**——不回调裸 start；从 `_validated.launch_view()` 派生两个事实：
   - `prompt`：`launch_view().prompt`（只读投影借出的正是本次 launch 的实际 prompt）；
   - `resume_provider_session_id`：validated 面**不外借**该字段（`into_parts` 是 `pub(crate)`），故由替身**自身状态**派生——新增 `last_native_session_id` 状态位，记录替身上一轮 completion 里发出的原生会话 id（`author-provider-session-1`），第二轮即以其续接，与裸路径语义同形。
4. 该用例转绿，且未改任何断言。

---

## 4. 逐条红→绿台账（交接时 21 条红）

| # | 用例名 | 根因 | 改法 | 终态 |
|---|---|---|---|---|
| 1 | `http_confirm_without_with_review_finalizes_directly_when_review_enabled` | `provider_unsupported_for_gateway_launch:Fake`（author 侧） | 夹具 author→`claude_code`；替身注册名 Fake→ClaudeCode；state 包 health | ✅ |
| 2 | `http_confirm_on_story_author_confirm_starts_reviewer_then_finalizes` | 同上（author 侧） | 同上 | ✅ |
| 3 | `http_confirm_on_story_without_reviewer_finalizes_without_review_node` | 同上（author 侧） | 同上 | ✅ |
| 4 | `http_confirm_during_cross_review_is_rejected_without_finalizing` | 同上（author 侧） | 同上 | ✅ |
| 5 | `http_confirm_after_fake_reviewer_skip_finalizes_through_engine` | 同上（author 侧） | 同上；**reviewer 侧必须保持 Fake**（见 §5.2） | ✅ |
| 6 | `post_review_request_revision_uses_author_feedback_prompt_not_reviewer_repair` | 同上（author 侧） | 同上 | ✅ |
| 7 | `story_author_confirm_request_revision_runs_revision_and_returns_to_gate` | 同上（author 侧） | 同上 | ✅ |
| 8 | `workspace_ws_start_generation_includes_context_note_in_author_prompt` | 同上（author 侧） | 同上 | ✅ |
| 9 | `workspace_ws_session_state_projects_pending_provider_choice_during_active_run` | 同上 | 同上 | ✅ |
| 10 | `workspace_ws_session_state_projects_text_fallback_pending_choice` | 同上 | 同上 | ✅ |
| 11 | `workspace_ws_pending_choice_projection_clears_after_run_completes` | 同上 | 同上 | ✅ |
| 12 | `workspace_ws_choice_pending_change_broadcasts_session_state_with_pending` | 同上 | 同上 | ✅ |
| 13 | `workspace_ws_text_fallback_choice_pending_change_broadcasts_session_state` | 同上 | 同上 | ✅ |
| 14 | `workspace_ws_plan_subdrive_choice_pending_projects_into_session_state` | 同上（会话 author/reviewer + StartGeneration provider_config 三处 Fake） | 三处 Fake→ClaudeCode；state 包 health | ✅ |
| 15 | `workspace_ws_review_drive_choice_pending_projects_into_session_state` | 同上（会话 reviewer_provider=Fake ⇒ 恢复的 review 拉起 Fake） | 会话 reviewer→`pi`；持久化 reviewer 节点 agent→Pi；替身注册名→Pi；state 包 health | ✅ |
| 16 | `workspace_ws_user_message_interrupts_active_stream_before_completion` | 同上（默认 fake registry 的 Fake 槽位） | 夹具 author→`claude_code`；默认 registry 的 ClaudeCode 槽位本就是生产 `TestControlledFakeStreamingProvider`（其 `start_validated` 与裸 start 同路径），无需另造替身；state 包 health | ✅ |
| 17 | `workspace_ws_abort_discards_partial_stream_without_completion` | 同上 | 同上 | ✅ |
| 18 | `workspace_ws_disconnect_during_active_run_writes_no_disconnect_terminal` | 同上 | 夹具 author→`claude_code`；替身注册名 Fake→ClaudeCode；state 包 health | ✅ |
| 19 | `workspace_ws_runs_provider_from_repository_path` | 同上 | 夹具 author→`claude_code`；替身注册名→ClaudeCode；`WorkingDirRecordingStreamingProvider` 补 `start_validated`（working_dir 观察点迁到 `launch_view().working_dir`）；state 包 health | ✅ |
| 20 | `workspace_ws_cursor_reconnect_replays_pending_choice_after_replay_window` | `provider_gateway_adapter: … streaming provider validated start is not supported by this adapter`（`ChoiceThenArtifactProvider` 缺 `start_validated`） | §3.3 共享方法 + `start_validated` | ✅ |
| 21 | `workspace_ws_abort_with_pi_reaches_cancelled_state_and_stops_output` | 见 §6 | 替身补 `start_validated`（会话行为逐字复刻）；**断言未动** | ❌ 留红（Main 裁定） |

### 4.1 第二轮新增根因（第一遍迁移后暴露的 4 条）

第一遍迁移后剩 5 条，其中 4 条的根因**不是 Fake**，而是 gateway 的另一道生产门：

- **Codex/KimiCode 的用户级 workspace trust**：`provider_trust::requires_workspace_trust` = `Codex | KimiCode`；gateway 在 spawn 前用只读 trust source 复验，读**真实 HOME** 的 `~/.codex/config.toml`。集成测试既无法也不应写用户 home ⇒ reviewer 侧由 Codex 改挂 **Pi**（Pi 无该前置，且 reviewer 替身与 author 替身仍能各自独立注册）。
  - `http_confirm_on_story_author_confirm_starts_reviewer_then_finalizes` / `http_confirm_during_cross_review_is_rejected_without_finalizing` / `post_review_request_revision_uses_author_feedback_prompt_not_reviewer_repair`：夹具 reviewer `codex`→`pi`、registry `Codex`→`Pi`、`provider_config.reviewer`→`Pi`。
  - `workspace_ws_review_drive_choice_pending_projects_into_session_state`：会话 reviewer_provider→`pi`、持久化 reviewer 节点 `agent`→Pi、registry→Pi（该用例由 review 驱动真实拉起 reviewer）。
- `http_confirm_during_cross_review_is_rejected_without_finalizing` 另有一处次生现象：Codex reviewer 启动被 trust 拒绝 ⇒ 评审没进入在途窗口 ⇒ 第二次 confirm 返回 200 而非 409；换成可启动的 reviewer 后恢复 409（断言未改）。

### 4.2 ⚠️ 已知偏离：reviewer 由 Codex 改挂 Pi（被验对象被换，覆盖面缩小）

**Main 裁决：登记为已知偏离 + 后续立项**（生产侧「测试用 trust source 注入」seam），**不许静默接受**。逐条点名（4 条用例的 reviewer provider 由 `codex` 改成 `pi`）：

| # | 用例 | 原 reviewer | 现 reviewer | 改点（夹具 / registry / provider_config） | 断言是否变 |
|---|---|---|---|---|---|
| D1 | `http_confirm_on_story_author_confirm_starts_reviewer_then_finalizes`（part_02） | `codex` | `pi` | 夹具 `_with_providers(root, "claude_code", "pi", 1)`；registry `ProviderName::Codex`→`ProviderName::Pi`；`provider_config.reviewer`→`Some(Pi)` | 否 |
| D2 | `http_confirm_during_cross_review_is_rejected_without_finalizing`（part_02） | `codex` | `pi` | 同上三处（reviewer 替身是 `HangingStreamingProvider`，注册名 Codex→Pi） | 否 |
| D3 | `post_review_request_revision_uses_author_feedback_prompt_not_reviewer_repair`（part_09） | `codex` | `pi` | 同上三处 | 否 |
| D4 | `workspace_ws_review_drive_choice_pending_projects_into_session_state`（part_08） | `codex`（会话 reviewer_provider 由 `fake` 改 `pi`） | `pi` | 夹具 `_with_providers(root, "claude_code", "pi", 1)`；持久化 reviewer 节点 `agent`→`Some(Pi)`；registry `ClaudeCode`→`Pi` | 否 |

偏离性质（与 §4.1 的普通迁移区分）：这 4 条**被验对象换了**——原意是验 Codex reviewer 的行为，现在验的是 Pi reviewer ⇒ 覆盖面缩小。根因是 gateway 对 `Codex | KimiCode` 强制**用户级 workspace trust**（读真实 HOME 的 `~/.codex/config.toml`），集成测试无法（也不应）写用户 home；`Pi` 无该前置。后续立项方向：生产侧「测试用 trust source 注入」seam，恢复 Codex reviewer 的可测性。

未改 reviewer 命名的 2 条（reviewer 永不启动 ⇒ 不经 gateway ⇒ 无 trust 门）：`http_confirm_without_with_review_finalizes_directly_when_review_enabled`、`http_confirm_on_story_without_reviewer_finalizes_without_review_node`（均仍为 `codex`）。

---

## 5. 断言观察点迁移清单（逐处点名，语义不减）

**全族 diff 中 `assert` 行零删除、零改写**（唯一新增的一行 `assert_eq!(status, StatusCode::OK, "{index_rebuild}")` 属第一层夹具，非本次）。观察点的迁移全部发生在**替身内部**（adapter 侧裸 input → validated 启动事实的只读投影），断言文本与被断言语义均未变：

| 处 | 观察点 | 迁移方式 | 语义 |
|---|---|---|---|
| `part_04.rs` `WorkingDirRecordingStreamingProvider` | provider 收到的 spawn cwd | `input.working_dir` → `launch_view().working_dir` | 投影借出的正是本次 launch 的实际 spawn cwd；断言「等于 repo canonical path」不变，实测转绿 |
| `part_04.rs` `ChoiceThenArtifactProvider` | author prompt | `input.prompt` → `launch_view().prompt` | 投影借出的正是本次 launch 的实际 prompt |
| `part_04.rs` `ChoiceThenArtifactProvider` | resume 事实 | `input.resume_provider_session_id` → 替身自身上一轮发出的原生会话 id | validated 面不外借该字段；无断言消费该字段 |
| `part_05.rs` `ScriptedStreamingProvider` | author prompt | `input.prompt` → `launch_view().prompt` | 断言「prompt 含 context note」不变，实测转绿 |
| `part_05.rs` `ScriptedStreamingProvider` | 结构化输出契约（nonce） | `input.structured_output_contract` → 从 prompt 的 sentinel 模板复原（生产把同一 nonce 写进 prompt） | ⚠️ **临时措施 + 耦合 prompt 模板形状**（Main 裁决：接受为临时措施，登记后续立项——`launch_view` 增契约投影更稳）。仅 reviewer 角色需要；author 角色本就无契约，保持 None |
| `part_05.rs` `PiHangingStreamingProvider` | provider_type / permission_mode | **无法迁移** | 见 §6 |

---

## 6. Pi 条留红复核（确认未动）

用例：`workspace_ws_abort_with_pi_reaches_cancelled_state_and_stops_output`（`tests/it_core/workspace_ws_integration/part_04.rs:291`）。

- **断言未动**：`part_04.rs:333-340` 仍钉 adapter 侧观察点 `(ProviderType::Pi, ProviderPermissionMode::Auto)`，逐字未改（**未**改成「只验 role==Author」这类弱断言）。
- **替身只补了 `start_validated`**（族 C/族 D 已批准手法第 ③ 步）：会话行为（首段输出 → 挂起 → 取消后收尾）逐字复刻；**不**向 `seen_inputs` 写入任何伪造值。
- **失败点位移**：迁移前红在 `provider_gateway_adapter: ProviderExecutionFailed … streaming provider validated start is not supported by this adapter`（还没走到断言）；迁移后红在**钉住的那条断言本身**：
  ```
  assertion `left == right` failed: workspace must invoke Pi in Auto mode
    left: []
    right: [(Pi, Auto)]
  ```
  这正是 Main 裁定的现场：LC gateway 的 validated 启动只外借 `launch_view()`（role/working_dir/prompt），`provider_type` / `permission_mode` **不外借**（`into_parts` 是 `pub(crate)`，集成测试 crate 看不见）⇒ **无等价可观察点** ⇒ 暂留红待立项。
- 本次**未**动生产给 `launch_view` 加 provider/permission 投影（已明确不授权）。

---

## 7. 硬约束遵守情况

- **未改断言凑绿**：全族 diff `assert` 行零删除/零改写（§5）。
- **未改生产实现**：`src/` 零改动；全部改动落在 `tests/it_core/workspace_ws_integration/`。
- **未恢复** `RepositoryRouting::Legacy` 变体 / 单仓语义；**未自造** reason_code；**未改** spec / Plan / 技术方案；**未删**任何测试。
- **未碰** `tests/it_web/`（另一 worker 在改）与 `tests/it_core/large_file_guard.rs`（登记由 Main 统一改，本报告 §8 给出行数）。
- **未** git add / commit / stash / checkout。
- **mtime 避让**：开工前 part_03（00:20:14）/part_04（00:15:44）/part_05（00:12:22）/part_06（00:19:09）/part_06b（00:19:22）均在 20 分钟窗口内 ⇒ 先做 part_01/02/07/08/09/10/11，待窗口过后才动这五个文件；三路子 agent 均已 idle，无并发写者。
- cargo 全程 `flock /tmp/aria-cargo.lock` 串行，未并行起第二个 cargo。

---

## 8. 被改文件行数表（供 large_file_guard 登记）

**本轮（FinishItCoreBatch）改动过的文件**（下表「累计 +/-」为相对 HEAD 的全量差额，含第一层夹具与上一轮三路子 agent 的落盘；「本轮增量」为交接时点→现在的差额）：

| 文件 | 当前行数 | 累计 +/-（vs HEAD） | 本轮增量 | 本轮内容 |
|---|---|---|---|---|
| tests/it_core/workspace_ws_integration/part_01.rs | 196 | +33/-8 | +18 | 1 条用例迁移（夹具换 author、替身注册名、state 包 health） |
| tests/it_core/workspace_ws_integration/part_02.rs | 1095 | +91/-46 | +137 | 6 条用例迁移 + reviewer Codex→Pi |
| tests/it_core/workspace_ws_integration/part_03.rs | 1048 | +113/-62 | +36 | 3 条用例迁移 |
| tests/it_core/workspace_ws_integration/part_04.rs | 829 | +111/-39 | +101 | 三个替身补 `start_validated`（`WorkingDirRecording` / `ChoiceThenArtifact` / `PiHanging`）+ 共享方法抽取 |
| tests/it_core/workspace_ws_integration/part_05.rs | 1025（拆分后） | +477/-85 | +115 | `ScriptedStreamingProvider` 错版 `gateway_session` 重写 + `start_validated`；`PiHangingStreamingProvider` 共享方法 + `start_validated`；`scripted_provider_completion` 契约派生 helper（拆分后另有 part_05b.rs 269 行，纯搬移） |
| tests/it_core/workspace_ws_integration/part_08.rs | 1040 | +158/-72 | +176 | 7 条用例迁移 + `TextFallbackChoiceCompletingProvider` 补 `start_validated` + 两处会话手术数据换名 |
| tests/it_core/workspace_ws_integration/part_09.rs | 361 | +31/-16 | +47 | 2 条用例迁移 + reviewer Codex→Pi |

**上一轮（三路子 agent + 前一位 worker）已落盘，本轮未再改动的：**

| 文件 | 当前行数 | 累计 +/-（vs HEAD） |
|---|---|---|
| tests/it_core/workspace_ws_integration/part_06.rs | 941 | +91/-51 |
| tests/it_core/workspace_ws_integration/part_06b.rs | 1045 | +81/-54 |

未改动（零 diff）：part_07 / part_10 / part_11（分别 265 / 677 / 174 行）——这三条文件内的用例本轮全绿且不经 gateway，无需迁移。

✅ **超线已解决**：`tests/it_core/workspace_ws_integration/part_05.rs` 拆后 **1025 行**，新增 `tests/it_core/workspace_ws_integration/part_05b.rs` **269 行**（HTTP/WS 收发工具群经 `include!` 挂载，**纯搬移**，见 §8.2 交接与拆分报告 `cadence/notes/2026-11-03_越线拆分_it_core_part05_v1.0.md`）⇒ 两文件均 ≤1200 ⇒ `large_file_guard::product_source_and_test_files_stay_under_line_limit` **实测转绿**（2 passed / 0 failed）。

**⇒ 登记项未新增**（Main 裁决：登记表只允许删项）——拆分后随行数下降自然回到硬线内，无需豁免。本轮对 part_05 的净增约 +25 行（4 处编辑：错版 `gateway_session` 重写为 `scripted_session`、两个替身补 `start_validated`、契约派生 helper），其余增量来自第一层夹具与上一轮 7 个替身的 `start_validated` 批次。拆分交接见 §8.2（已逐条直发拆分 worker `SplitItCorePart05`）。

### 8.1 ⛔ Main 裁决：MUST NOT 新增 large_file_guard 登记项

登记表自身的规矩是「只允许删项；确需新增项须用户裁决」——给一个本波涨了约 400 行的文件新开豁免正是那条规矩要防的。**Main 已另派 worker 拆 part_05.rs**（`include!` 文本包含手法，与前 4 个文件同一手法）⇒ **不给 part_05.rs 新增登记项**；本轮不动拆分。

### 8.2 📦 part_05.rs 拆分交接（给拆分 worker —— 本轮改动全量清单，勿切错）

**本轮（FinishItCoreBatch）在 part_05.rs 的全部改动**（新增/改写的条目，拆分时必须原样带走）：

| 条目 | 类型 | 说明 |
|---|---|---|
| `impl ScriptedStreamingProvider { fn scripted_session(&self, prompt: String, contract: Option<StructuredOutputContract>) -> ProviderSession }` | **本轮新增**（替换上一轮那版**错版** `gateway_session`——那版是从 `ChoiceThenHangingStreamingProvider` 复制粘贴的：发 `ChoiceRequest(id="choice_hanging_001")` 而非脚本化输出，编译绿但行为错） | 与 `impl StreamingProviderAdapter for ScriptedStreamingProvider` **必须同文件**（私有方法调用） |
| `impl StreamingProviderAdapter for ScriptedStreamingProvider` 的 `start` | 本轮改写 | 改为 `Ok(self.scripted_session(input.prompt, input.structured_output_contract))`（行为零漂移） |
| 同上的 `start_validated` | **本轮新增** | 从 `validated.launch_view()` 取 prompt；从 prompt 的 sentinel 模板复原 reviewer 契约；**不回调裸 start** |
| `fn scripted_provider_completion(output, contract)` | 本轮改写 | 新增「输出无 JSON fence ⇒ 原样透传 + 交契约 fail-closed」分支（避免 reviewer 提示类产物 panic） |
| `fn review_output_parts(output) -> Option<(&str, &str)>` | 本轮改写 | 返回类型由 `(&str,&str)` 改为 `Option`（无 fence ⇒ None） |
| `fn reviewer_contract_from_prompt(role, prompt) -> Option<StructuredOutputContract>` | **本轮新增** | 仅 `AdapterRole::Reviewer` 返回契约（author 侧本就无契约 ⇒ None） |
| `fn nonce_from_prompt(prompt) -> Option<String>` | **本轮新增** | 取 prompt 里最后一个非 `EXAMPLE_NONCE` 的 sentinel nonce |
| `impl PiHangingStreamingProvider { fn hanging_session(&self, cancel) -> ProviderSession }` | **本轮新增** | 会话本体抽取（首段输出 → 挂起 → 取消后收尾 + `abort_observed.notify_one()`） |
| `impl StreamingProviderAdapter for PiHangingStreamingProvider` 的 `start` | 本轮改写 | 改为先记 `seen_inputs`（裸路径专有）再 `Ok(self.hanging_session(cancel))` |
| 同上的 `start_validated` | **本轮新增** | 只复刻会话行为；**不**向 `seen_inputs` 写任何值（provider_type/permission_mode 不外借 ⇒ 该用例留红，见 §6） |

**上一轮已落盘、拆分时不得切错/不得重复实现的共享条目**（都在 part_05.rs）：

- 五个已带 `start_validated` 的替身及其共享方法：`impl HangingStreamingProvider { fn gateway_session }`、`impl SignalledCompletionStreamingProvider { fn gateway_session }`、`impl ChoiceThenCompletingStreamingProvider { fn gateway_session }`、`impl SequencedChoiceCompletingProvider { fn gateway_session }`、`impl ChoiceThenHangingStreamingProvider { fn gateway_session }`（每个的 adapter impl 都已带 `start_validated`，**不回调裸 start**）。
- 夹具与前置链：`create_workspace_session_fixture` / `_with_author` / `_with_providers`、`confirmed_capability_record`、`seed_confirmed_gateway_capabilities`、`always_available_gate`、`struct StubSyncAdapter` + 其 `ProviderAdapter` impl、`with_available_provider_health`、`git_repo`、`replace_workspace_session_messages`、`request_json` / `send_json` / `recv_json` / `recv_until_*` 一组收帧 helper、`scripted_provider_completion` 的调用方（part_04 的 `ScriptedStreamingProvider` adapter impl 依赖它）。

**跨文件耦合（拆分后仍须同属一个编译单元，include! 列表要全）**：

- `tests/it_core/workspace_ws_integration.rs` 是唯一 include! 清单（part_01…part_11）——新增拆分文件必须补进该清单。
- `ScriptedStreamingProvider` 的**结构体 + `impl ScriptedStreamingProvider { fn new }` 在 part_04.rs**，而它的 adapter impl / `scripted_session` 在 part_05.rs ⇒ 拆分后两者仍在同一 crate 作用域即可（Rust item 顺序无关），但**不要**把 `fn new` 与 adapter impl 拆到两个 crate。
- `VALID_STORY_SPEC`、`PI_POST_ABORT_OUTPUT` 常量在 **part_01.rs**，被 part_05 的多个替身引用。
- part_05 用到的 `ChoiceRequestData` / `ChoiceOptionData` / `ChoiceRequestSource` / `ProviderStatus` 来自共享 import 块（**part_01.rs** 顶部的 `use` 区，include! 后全局可见）。
- part_04.rs 的 `WorkingDirRecordingStreamingProvider` / `ChoiceThenArtifactProvider` / `PiHangingStreamingProvider`（本轮补的 `start_validated`）与 part_05 的替身共享同一批 `use` import。

---

## 9. Concerns

1. ~~**part_05.rs 超 1200 硬线**~~ —— **已解决**：拆分后 part_05.rs 1025 行 + part_05b.rs 269 行（纯搬移），`large_file_guard` 实测**转绿**（2 passed / 0 failed）；**未新增任何登记项**（Main 裁决：登记表只允许删项）。拆分报告见 `cadence/notes/2026-11-03_越线拆分_it_core_part05_v1.0.md`。
2. **⚠️ 已知偏离：4 条用例的 reviewer 由 Codex 改挂 Pi**（逐条点名见 §4.2）——被验对象被换，覆盖面缩小，**登记为后续立项**（生产侧「测试用 trust source 注入」seam）。根因：gateway 对 Codex/KimiCode 强制读真实 HOME 的用户级 workspace trust，集成测试不应写用户 home。
3. **结构化输出契约在 validated 面不可见**（临时措施 + 耦合 prompt 模板形状，§5 第 5 处）：替身靠「生产把同一 nonce 写进 prompt 的输出模板」复原（`nonce_from_prompt`）。**Main 裁决：接受为临时措施，登记后续立项**（`launch_view` 增契约投影更稳）。生产若改 prompt 模板，替身需同步改。
4. **Pi 条留红**（§6）：无等价可观察点。**Main 裁决：维持暂留红**，登记为后续立项（另立可观察面或给 `launch_view` 增投影，都需独立 change）。**未**向 `seen_inputs` 写伪造值。
5. **两条「reviewer 永不启动」的用例仍保留 Codex 命名**（`http_confirm_without_with_review_finalizes_directly_when_review_enabled` / `http_confirm_on_story_without_reviewer_finalizes_without_review_node`）：reviewer 不启动 ⇒ 不经 gateway ⇒ 无 trust 门，保持最小 diff；若将来这些用例真启动 reviewer，须同样换 Pi。
6. **it_web 侧同类问题**（裸建 issue / Fake provider 驱动）由另一 worker 在改，本轮未触碰 `tests/it_web/`。
