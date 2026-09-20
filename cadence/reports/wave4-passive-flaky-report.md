# wave4-passive-flaky：codex 写失败注入单例确定性根治（RR-3 销账）

- 目标测试：`codex_provider_request_user_input_emits_protocol_error_on_write_failure`
- 基线：工作树 feat-b-0808-add-monorepo，HEAD=82ad3017（改造前）；commit 见 §5
- 台账登记：`stage4-c3-t5-report.md` §3.6（"历史首跑红/单跑绿=写失败注入 flaky，RR-3 已登记"）+ `legacy-protocol-retirement/wp5-handoff-notes.md` #5/#6（"lib 全量 1 例首跑红/单跑绿=写失败注入 flaky（改动面零交集，RR-3：首败事实+定向复跑佐证）"）

## 1. 首败事实（RR-3 纪律：不以复跑覆盖首败）

- 首败：stage4-c3 T5 收尾轮全量 `cargo test --locked --lib` 首跑，本测试 1 红（原始 panic 输出未留档，台账仅记形态）；同日两次全量终跑未再发作。
- 定向复跑（本会话，旧形态基线）：隔离单跑稳定绿（与台账"单跑绿"一致）。
- 本会话未在全量语境下再次复现（全量首跑属冷缓存+满并发一次性语境；复现尝试受同仓四线并行会话污染不可控，如实登记）。

## 2. 根因

机制链（改造前）：fixture `codex_request_user_input_peer_closes_fixture.sh` 在回放 `item/tool/requestUserInput` 后 `exec 0<&-` 关闭 stdin 读端再 `sleep 5`；provider 在 choice 往返后写 JSON-RPC 应答 → EPIPE → `emit_request_user_input_protocol_error`（`request_user_input_unresolved`）。

- **EPIPE 注入本身因果确定**：fixture 关闭读端先于 provider 任何可能写（写被 choice 事件往返因果后置，kernel 管道语义保证读端全关后写必 EPIPE）。
- **flaky 维度=真实进程链 + TEST_TIMEOUT(5s) 墙钟窗口**：全量首跑=冷页缓存+16 测试线程满并发（codex/kimi 数十真实进程 fixture 同时拉起 bash+sed），测试两侧事件窗口（bash spawn+initialize/thread/start/turn/start 三往返的握手跨度、choice 往返+写失败传播）可被调度饿死超 5s → 首跑红；单跑热缓存低负载毫秒级完成 → 绿。与 RR-3① kimi terminal 家族同谱系（真实进程+紧墙钟预算，单跑即过）。
- 排除项：bridge/命令泵无定时器（`approval_bridge/mod.rs`+`commands.rs` 纯通道映射）；写成功竞态（读端未关窗口仅一条 bash 指令宽，物理不可达）；fixture sleep 5 到期先于 choice 应答时写仍 EPIPE（进程退出关全部 fd），结论不改变红向只有墙钟超时一族。

## 3. 修法（验证方式迁移，d7525220 kimi terminal 同款先例；零生产代码改动）

- 测试改造为受控内存流直驱会话循环：`tokio::io::duplex` 双通道构造 `JsonRpcPeer`，直接调用 `run_codex_session_loop`（构造 `CodexSessionHandshake` 跳过握手，turn/start 由测试泵应答），requestUserInput 行逐字取自已退役 fixture（parse 路径不变）。
- **确定性注入**：drop stdin 读端严格先于发出触发写的 `ChoiceResponse`——写失败时序由因果定序锁定（duplex 对端 drop 后写返回 BrokenPipe，与真实管道 EPIPE 同语义），无真实进程、无墙钟竞态。
- **断言不放松**：`code=request_user_input_unresolved`/message 双 contains/`question_id="confirm"` 逐字保留；新增会话循环必须以 Err 终止的断言（增强）。
- **反向验证（红腿实证）**：临时移除 drop → 写成功 → 无 ProtocolError → 5s 超时 `provider should emit events: Elapsed` FAILED——证明写失败注入被测试真实承载（注入是承载性的，非恒真断言）。
- 清切：孤儿 fixture `codex_request_user_input_peer_closes_fixture.sh` 删除（全仓无产码/测试引用，仅历史归档文档提及）。

## 4. 证据

| 项 | 结果 |
|---|---|
| 最终形态 10 次隔离复跑 | **10/10 绿，每轮 0.00s**（零墙钟暴露；日志 /tmp/wave4-flaky-10x.log） |
| 反向验证（红腿） | 移除 drop → FAILED（5.00s 超时，注入承载性实证） |
| codex_provider 全族回归 | `cargo test --locked --lib codex_provider::` **50 passed / 0 failed**（1.03s） |
| 真实进程用户输入族保留 | `bridges_request_user_input_and_completes` + `bridges_all_request_user_input_questions`（happy×2）+ `request_user_input_emits_protocol_error_on_bridge_failure`（bridge 失败×1）——覆盖不减少 |
| fmt | `cargo fmt --check` exit=0 |
| clippy | 本 change 面零告警（会话内全量 clippy 撞并行线 F17 中间态 E0061=kimi sandbox/terminal，与本 diff 零交集，如实登记） |

## 5. 提交

- `src/cross_cutting/codex_provider/tests/mod.rs`（测试区单文件）
- `tests/fixtures/provider/codex_request_user_input_peer_closes_fixture.sh`（删除）

## 6. 台账销账行（追加，不回写历史）

- RR-3 codex 写失败注入单例（stage4-c3-t5 §3.6/wp5 #5）销账：`codex_provider_request_user_input_emits_protocol_error_on_write_failure` 确定性化完成——duplex 因果定序注入替代真实进程+墙钟形态，断言逐字保留+新增 Err 终止断言；10 次隔离复跑 10/10（0.00s/轮）+全族 50/0；孤儿 fixture 退役。真实进程用户输入族 3 例保留（happy×2+bridge_failure×1）。
