# Task 4.1（B1a：native 资格与 Story/Design 上下文）实施报告

- 实施：ImplAnchorB41（glm53-task worker）
- worktree：`/home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0808-add-monorepo`
- 基线 HEAD：`dcc8bac9`（fix: B0f 四provider公共汇合与审计一致）
- 状态：实现完成、定向红→绿与范围化回归全绿；**未 commit**（plan Step 5：提交建议只供后续实施/controller 审查收口）

## 1. 交付内容（写面 = plan Task 4.1 Files 清单）

| 文件 | 变更 |
|---|---|
| `src/product/workspace_engine/lifecycle.rs` | 新增 `NativeResumeCandidate{native_id,role_run_seq}` / `NativeResumeCandidateError{WorkspaceRecord,ProviderAudit,ProviderIdentityInvalid,FrozenBindingUnavailable}`（Display 稳定类别前缀 `native_resume_candidate_*`）与 `provider_resume_session_candidate()` + 私有 `bind_native_run_identity()`；**删除** `provider_resume_session_id` / `author_provider_start_audit_native_id`（含两处 `.ok()` 吞错），无 shim；`persist_lc_superseded_wait` 迁 typed candidate（错误类别传播） |
| `src/product/lifecycle_store/tool_policy_run_audit.rs` | **删除** `latest_provider_start_native_id`；clean-cutover 为 `latest_provider_start_resume_candidate()` 返回 `ProviderStartCandidateLookup{Eligible{role_run_seq,native_id},AllSuperseded,NoMatch}`：旧 run `session_terminated` 与 durable tombstone 多代排除 |
| `src/product/workspace_engine/prompts.rs` | `build_streaming_input` / `build_work_item_plan_streaming_input`：先 `candidate_result?`（typed→String 保留类别），成功才 `map(native_id)` 决定 delta 合同与 resume ID；`build_work_item_plan_streaming_input_fresh` 保持从不查 candidate、ID=None |
| `src/product/workspace_engine/prompts/revision.rs` | `build_revision_input_with_resume`：仅 `allow_resume=true` 查 candidate；Err 立即传播；成功候选才驱动 delta/full 分流 |
| `src/product/workspace_engine/prompts/author_revision.rs` | `build_author_revision_prompt` 消费 `resumed_session`：fresh（无 resume）且有历史材料时注入 `compact_history`（Author 模式，verdict 随 `HistoryCompactionInput` 重放，trusted 优先级不变）；无历史材料不注入空小节；反馈只在「## 用户反馈」出现一次 |
| `src/product/workspace_engine/review/drive.rs` | `resolve_revision_root_launch`：record 读错带底层类别传播（不再 `.ok()` 折「cannot load」）；candidateErr → `Some(Err(..))`（含类别前缀），不落 `(None,_)` validate/fresh 臂；成功候选 map native_id；`RevisionLaunchDecision`/`SupersededRevisionWait` 补 `Debug`、前者 `pub(crate)`（pub(crate) 方法返回类型可见性要求） |
| `src/product/workspace_engine/provider_drive/author_root_launch.inc.rs` | E anchor 接线：`build_root_launch_request` 改经 `resolve_workspace_repository_context` 一次派生——business 字段给成员 checkout target（语义不变），deferred `aggregate_root` 臂取 `native_launch_anchor`（聚合根视图，scope 回写不迁锚），native anchor 派生失败 fail-closed |
| `src/product/workspace_engine/provider_drive.rs` | 检查后无需改动：author 首轮的 input 构造失败/gateway 解析失败错误收口（可见错误+`finish_failed_run`，零启动）已在 Task 2.1 闭包内满足本包合同 |
| tests：`part_12.rs` / `author_revision_loop.rs` / `f60_exit_contract_matrix.rs` | 旧接口两测试迁 `provider_resume_session_candidate_*`（断言 seq 绑定）；新增 5 个 `anchor_t11_*`（见 §3）；其余既有测试零改动全保留 |

## 2. 新接口语义（与 BP2 合同逐条对照）

- 真正已确认 ID：trim 非空（`provider_conversation_session_id` 既有过滤 + audit 侧 `!trim().is_empty()`）。
- 候选来源短路：内存 conversation → durable record conversation（record IO/JSON/身份错误 = `WorkspaceRecord` Err，不吞）→ Author 面 audit 兜底（读取错误 = `ProviderAudit` Err）。
- audit 兜底 clean-cutover：只返回有恢复资格候选——run 文件含 `session_terminated`（superseded_policy_drift/kill 终止）排除、durable tombstone 多代排除；有匹配但全部无资格 → `AllSuperseded` → `FrozenBindingUnavailable`（不捞更旧代、不当出生 fresh）；确证无匹配才 `Ok(None)`。
- `role_run_seq` 联合绑定（`bind_native_run_identity`）：
  - LC（gateway 在场）且同 (role,provider) 存在 durable fullmap 记录（**fullmap 纪元**）：native 必须命中 Active fullmap 绑定 → `Some(seq)`；native 在而绑定缺失/记录 Superseded/tombstone → `FrozenBindingUnavailable`（durable 等待、零 provider start）；audit 与 fullmap 同 native 异 seq → `ProviderIdentityInvalid`。
  - **纪元之前**（fullmap 无记录，当前生产与全部既有 fixture 形态——fullmap 写入归 Plan 5.2「prepared map before start」）：维持既有恢复行为，audit 绑定在场即携带其 seq，不伪造。
  - direct（无 gateway）：既有 conversation 可 `None` seq，不加 LC fullmap 硬门。
- 显式 fresh：`allow_resume=false` / `_fresh` builder 从头不查 candidate、ID=None。
- 三个生产 caller 错误闭包：builders `Err(String)`（类别前缀保留）；`resolve_revision_root_launch` `Some(Err)`；drive 收口可见错误+`finish_failed_run`，claim/gateway/直连启动计数不增、恢复面（recovery record/tombstone/conversations）不变。

> ⚠️ 设计张力备案（供 controller/独立审查裁决）：BP2 字面「LC 候选必须 Some(seq)」若在纪元之前也硬门（即 native 在而 fullmap 未写就 `FrozenBindingUnavailable`），会把 `author_revision_loop_parts/root_cwd_revision.inc.rs`（**4.2 独占文件，本包不可改**）中 Task 2.3/2.6 已绿的 `revision_resume_cwd_drift_supersedes_session` / `anchor_t07_superseded_waiting_finishes_provider_node` 第一轮 conversation-only resume 打红，且当前生产 fullmap 无任何写入方（5.2 才落），等于 B 波中段全量 LC resume 进入等待。本实现选择「fullmap 纪元激活即严格」（纪元由 durable fullmap 记录存在性判定），strict 合同在 5.2 写 map 后立即全量生效；anchor_t11 已按严格语义覆盖（fullmap 纪元 seeded 的 Frozen/身份冲突/Some(seq) 正例）。
> ✅ **k3 裁决（2026-10-12，Main 转达）：采纳兼容实现（「fullmap 纪元激活即严格」维持），本段张力已裁结。**

## 3. 新测试（filter `anchor_t11`，5 个）

1. `anchor_t11_native_candidate_excludes_superseded_audit`（part_12）：多代 audit（gen0/old terminated、revoked tombstone、fresh 当代）→ 候选= `audit-native-fresh@Some(3)`，superseded 全排除；全部吊销 → `FrozenBindingUnavailable`。
2. `anchor_t11_fresh_revision_context_survives_reconnect`（author_revision_loop）：durable 消息流经 `new_persistent` 重连重建后，reviewer fresh（ID=None + feedback/artifact/trustedReview/压缩历史）与 author 反馈 fresh（feedback/artifact/压缩历史、反馈小节唯一）自足。
3. `anchor_t11_candidate_read_error_stops_all_lc_launch_callers`（author_revision_loop）：损坏 session record JSON → `WorkspaceRecord` + 两 builder Err（类别前缀）+ launch `Some(Err)`；损坏审计文件（schema_version 篡改）→ `ProviderAudit` + launch `Some(Err)` + drive 零捕获/零 gateway/零直连启动 + 恢复面 durable 不变 + typed 类别上浮；fullmap 纪元 unbound native → `FrozenBindingUnavailable`（两 builder Err、launch 不落 fresh 臂）；Active 绑定一致 native → `Some(2)` 续接。
4. `anchor_t11_direct_birth_and_resume_are_unchanged`（f60）：direct 出生 `Ok(None)`+Full 完整装配；既有 conversation `author-session-1` 同 ID 恢复、seq `None` 不拒、Delta 一行短引用；plan resume 携 id / 显式 fresh builder ID=None。
5. `anchor_t11_deferred_launch_anchor_stays_native_root_without_checkout_identity`（author_revision_loop）：Story focus=None → 业务聚合根视图（无 checkout 身份）→ deferred target = E native anchor（canonical manifest root），cwd==root，非业务成员路径。

## 4. 红→绿证据（本 worker 会话实测）

1. **基线绿**（旧接口）：`cargo test --locked --lib provider_resume_session_id` → `3 passed`（bg_319，78s）。
2. **接口切割编译红**（clean-cutover 证明，无 shim）：迁移后首跑 `cargo test --locked --lib anchor_t11` → `E0599: no method named provider_resume_session_id found`（旧 caller 已删、旧接口不复存在）+ `E0624 resolve_revision_root_launch is private` 等，直至实现补齐才可编译。
3. **行为红**：`anchor_t11_candidate_read_error_stops_all_lc_launch_callers` 首跑 FAILED @author_revision_loop.rs:1070（`session record 不变` 字节断言）——按既有失败收口会写 status/updated_at 的事实修正断言为恢复面不变（recovery/tombstone/conversations），复跑过。
4. **行为红（deferred 臂）**：首版 deferred 测试（剥离成员 checkout 身份）FAILED `product_store_identity_mismatch`——严格投影身份门正确拒绝，改用真实可达形态（Story focus=None 聚合根视图）后过。
5. **绿**（最终，全部本 worktree 实测命令）：
   - `cargo test --locked --lib anchor_t11` → **5 passed; 0 failed**
   - `cargo test --locked --lib provider_resume` → 3 passed（含迁移后两测试 + coding 隔离测试）
   - `cargo test --locked --lib author_revision` → 28 passed（4.1/4.2 范围化回归行；含 root_cwd 全部 Task 2.3/2.6 既有用例）
   - `cargo test --locked --lib product::workspace_engine` → **831 passed; 0 failed**
   - `cargo test --locked --lib single_candidate` → 161 passed
   - `cargo test --locked --lib interrupted_run` → 7 passed
   - `cargo test --locked --lib logical_codebase` → 458 passed
   - `cargo test --locked --lib web::workspace_ws_handler` → 182 passed
   - `cargo test --locked --lib coding_workspace_engine` → 575 passed（direct/Coding 不回归）
   - `cargo test --locked --lib f60_exit_contract` → 8 passed；`logical_revision` → 1 passed
   - `cargo check --locked --lib` → 0 error；写面文件无新增 warning（prompts.rs `reredrive_prompt_tests` unused import 为基线既有）
6. 未执行（Main/owner 门）：`cargo test --lib` 全量、`--test it_core/it_web/it_product`、release build、独立 review。

## 5. 事故与处置

- 一次 `write` 相对路径误写主仓 `src/product/workspace_engine/tests/part_12.rs`：已核对 diff 全部为本 worker 内容后 `git checkout --` 还原主仓（无第三方改动受损），文件搬运至 worktree。
- part_12 整文件重写时两个无关 design gate 测试正文凭截断视图重构失真（accept/reject），已逐字节恢复 HEAD 原版并以全量 diff 核对（删除行仅剩旧接口迁移相关）。

## 6. 提交建议（plan Step 5，本次未执行；仅 stage 显式清单）

```bash
git add -- src/product/workspace_engine/lifecycle.rs src/product/workspace_engine/provider_drive.rs src/product/workspace_engine/provider_drive/author_root_launch.inc.rs src/product/workspace_engine/prompts.rs src/product/workspace_engine/prompts/revision.rs src/product/workspace_engine/prompts/author_revision.rs src/product/workspace_engine/review/drive.rs src/product/lifecycle_store/tool_policy_run_audit.rs src/product/workspace_engine/tests/part_12.rs src/product/workspace_engine/tests/author_revision_loop.rs src/product/workspace_engine/tests/f60_exit_contract_matrix.rs
git commit -m "fix: B1a native资格与Story Design上下文"
```

注：`prompts.rs` 工作树中另有 5 行先于本包存在的 rustfmt 重排（并行格式化遗留，非本包语义改动）；controller stage 该文件时会一并带入，如需剥离请先裁决归属。

## 7. 移交

- `review/drive.rs` 本包提交后按 plan 串行移交 4.2（fresh 协议 Replay/真实发送闭包）。
- fullmap 写入（prepared-map-before-start）与「纪元之前」宽松臂的最终收紧归 5.2；StartGeneration.resume_recovery 入参归 4.2。

## 8. 提交回执（controller 裁决后补）

- 独立 commit：`0be830bc fix: B1a native资格与Story Design上下文`（10 文件显式清单，+1070/−120；prompts.rs 5 行并行 fmt 重排随入；本报告未 git add）。

## 9. abort_with_pi 类 it_core 失败分诊（Main 指令，2026-10-12）

**结论：存量 flaky，早于本包（1f253939 即败），非 4.1 引入；按 Main 指令回报转 Main 分诊，未硬修、未触碰 review/drive.rs（无需协调 ImplTrustT6R2）。**

证据链（全部实测）：

1. **当前 HEAD（6acdd4fc）复测**：`cargo test --locked --test it_core workspace_ws_abort_with_pi` 单跑首跑 ok；`abort` 批过滤 3/7 FAILED（abort_with_pi / abort_discards_partial_stream / secondary_connection_takes_lease）；连续 3 次单跑 abort_with_pi → ok / FAILED / ok——**同 HEAD 内非确定性**。
2. **提交前基线 1f253939 定界**（独立临时 worktree `/tmp/wt-b1a-1f253939`，复测后已删除；首次因干净 checkout 缺 web/dist 构建失败，复制主 worktree web/dist 后重跑）：
   - 第一次 `abort` 批：**3/7 FAILED**（含 `workspace_ws_abort_with_pi_reaches_cancelled_state_and_stops_output`）；
   - 第二次 `abort` 批：1/7 FAILED（换成了 `workspace_ws_abort_after_choice_response_returns_prepare_context`）——**失败用例非固定，同一签名**。
3. **失败签名**（失败与通过运行一致）：Abort 已受理（`[aria-cancellation] ws_abort_message` → `abort_workspace_run cancelling runner token` 已打点），随后 `recv_json` 3s 超时（part_05.rs:624）等待 `ProviderStatus::Aborted` / `StageChange(prepare_context)`；每次失败运行中 Pi 均已启动且首 chunk 断言已过（Abort 在其后才发出）。
4. **与 4.1 改动的耦合排除**：abort 链（`inbound.rs::Abort` 臂 → `manager/runs.rs::abort_active_run/cancel_run` → drive 循环取消收尾）本包未触碰（runs.rs 上次实质变更为计划前 3be1aa49/5c7f93e2）；`abort_active_run` 为同步临界区+token cancel（ms 级返回）；candidates/builders 只在 run 启动前调用，且失败运行中该路径已完成（chunk 到达）。失败相位在取消后引擎 run 收尾事件的 3s 接收预算，与后台 release 编译满载 CPU + it_core 并行测试线程的负载形态一致（`[aria-cursor-resubscribe] initial attach grace elapsed` 打点亦表明事件节奏已超常缓慢）。
5. **建议**（供 Main 分诊，未执行）：该类用例为负载敏感时序测试（3s recv / 1s notify / 150ms 无尾随窗）；可在无满载窗口复测定率，或由 owning 波次评估放宽预算/串行化 it_core websocket 用例；本包不据此改产品代码。
