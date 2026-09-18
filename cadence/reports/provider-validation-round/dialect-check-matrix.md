# WP2.1 ACP 方言核对矩阵（dialect-check-matrix）

实跑版本（Step 1）：`kimi --version` = **0.43.0**（2026-09-18 实跑，留档 `/tmp/kimi-version.txt`；计划起草日参考值 0.43.0 一致）。

- 证据 transcript：`cadence/reports/provider-validation-round/dialect-transcript-rep1.txt`（命令 `KIMI_ACP_E2E=1 cargo test --locked --lib live_kimi_acp_dialect_wire_capture -- --ignored --nocapture 2>&1 | tee …`，结果 **PASS**：`1 passed; 0 failed`，finished in 33.57s，EXIT=0）。
- 探测驱动面：gated 测试 `src/cross_cutting/kimi_code_provider/tests/live_kimi_tests.rs::live_kimi_acp_dialect_wire_capture`——aria 生产同款请求直驱真实 `kimi acp`（initialize → notifications/initialized → session/new → session/prompt → session/load）。
- 实施基线：HEAD `c8101323`；计划锚点 `session.rs:142-151`（initialize 请求体）/`:163-168`（initialized）/`:188-192`（load）/`:210-214`（new）/`:224-234`（sessionId 消费）/`:716-721`（prompt 请求）/`:723-728`（selected 回写）实施前实读复核，与计划一致。

## §1 逐方法核对表

| 方法 | 请求 shape 基线（aria 发送） | 响应/通知 shape 冻结基线 | 实收判定 | 处置 |
|---|---|---|---|---|
| initialize | `session.rs:142-151`（protocolVersion:1+clientCapabilities+clientInfo） | `fixtures/initialize.jsonl:1-2`（agentCapabilities.loadSession/sessionCapabilities.resume/agentInfo.version） | 一致（§2.1：消费面字段全在，余为纯增量） | — |
| session/new | `:210-214`（cwd+mcpServers） | 生产消费面 `:224-234`（result.sessionId）+`kimi_acp_fast_fixture.sh` | 一致（§2.2：sessionId 在场，configOptions/modes 同键 richer 内容） | — |
| session/prompt | `:716-721`（sessionId+prompt 数组） | `text_turn.jsonl`/`tool_call_turn.jsonl`（session/update 通知族+result stopReason 终态） | 一致（§2.3：result `{"stopReason":"end_turn"}` 逐字同形） | — |
| session/request_permission | server→client（`:601` 分发；回写 `:723-728` outcome.selected） | `acp_request_permission_bash.redacted.json`（options+approve_once） | **本探测不主动触发**——登记「待 T3 验证轮实跑回填」（coding 链 bash 审批必然触发 `coding_permission_request`） | T3 回填 |
| session/load | `:188-192`（sessionId+cwd+mcpServers） | `kimi_acp_resume_fixture.sh`/`kimi_acp_load_failure_fixture.sh`（含同进程 load 探测形态注记） | 一致（消费面，§2.5：同进程成功响应不含 sessionId，生产 `:229` 显式回退持有 resume_id） | —（注记在案） |
| session/update | 通知族（parse.rs 消费：text/tool_call/tool_call_update） | `text_turn.jsonl`/`tool_call_turn.jsonl`+`acp_session_update_tool_call_update.redacted.json` | 已触发 4 族全一致（§2.6）；tool_call 族本轮未触发 | tool_call 族 T3 回填 |

## §2 实收明细（transcript 逐帧实读）

### 2.1 initialize
- 发送：`protocolVersion:1 + clientCapabilities{} + clientInfo{"name":"cadence-aria-dialect-probe"}`（=生产 :142-151 shape，clientInfo 标注探测身份）→ 无错误接受。
- 实收（0.43.0）关键消费面字段（`validate_initialize` session.rs:856-872）：`protocolVersion:1` ✓；`agentCapabilities.loadSession:true` ✓；`agentCapabilities.sessionCapabilities.resume:{}` ✓（resume 门字段全过）；`agentInfo{"name":"Kimi Code CLI","version":"0.43.0"}`（版本值较冻结 0.34.0 前移=版本记录项而非 wire shape 漂移；`MIN_KIMI_VERSION=0.34.0`（mod.rs:35）兼容判定面 0.43.0≥0.34.0 不变）。
- 冻结基线无、实收新增字段：`agentCapabilities.auth{logout}`、`agentCapabilities.mcpCapabilities{http,sse}`、`agentCapabilities.promptCapabilities{audio,embeddedContext,image}`、`sessionCapabilities.{additionalDirectories,close,delete,fork,list}`、顶层 `authMethods[]`——纯增量，生产无消费点 → 不构成漂移。

### 2.2 session/new
- 发送：`{cwd:<临时目录>, mcpServers:[]}`（=生产 :210-214）→ 接受。
- 实收：`result.sessionId="session_30f221cd-010e-4642-aaaf-72e35a7422e2"` ✓（生产消费面 :224-234 直配，测试断言亦过）；同 result 含 `configOptions`（model/thinking/mode 三组 select，非空）与 `modes{availableModes[4],currentModeId:"default"}`——冻结 `text_turn.jsonl`/`tool_call_turn.jsonl` 的 session/new result 已含同键（`configOptions:[]`/`modes{currentModeId,availableModes}`）空值形态，结构同形、内容 richer，无新键。
- 注记：冻结 jsonl 请求侧另带 `permissionMode:"auto"`（合成 fixture 面；生产 :210-214 不发送该参数）。本探测按生产 shape（不带 permissionMode）发送被 0.43.0 接受 → 生产请求形状现行有效，fixture 请求侧属超集无需回退。

### 2.3 session/prompt
- 发送：`{sessionId, prompt:[{type:"text",text:"Reply with exactly: dialect-ok"}]}`（=生产 :716-721 content-block 数组）→ 接受。
- 实收 result：`{"stopReason":"end_turn"}`——与 `text_turn.jsonl` 冻结终态逐字同形 ✓（`parse.rs:195-196` stopReason 消费面不变）。

### 2.4 session/request_permission（未触发，如实登记）
- server→client 方法；本探测 prompt 无工具语义，无法主动触发。
- 冻结基线 `acp_request_permission_bash.redacted.json`（0.38.0 真实捕获）对应适配器消费面（`parse.rs:158-181` options{optionId,name,kind}/toolCall{toolCallId,title,content}；`session.rs:601` 分发、`:723-728` outcome.selected 回写）零变化。
- 处置：待 T3 验证轮实跑回填（coding 链 bash 审批必然触发 `coding_permission_request`）。

### 2.5 session/load（同进程探测形态，按注记口径判定）
- 发送：`{sessionId, cwd, mcpServers:[]}`（=生产 :188-192）→ **成功响应，无错误**（`kimi_acp_load_failure_fixture.sh` 的 -32001 错误语义未命中）。
- 实收 result：`{configOptions, modes}`——**不含 `sessionId` 字段**。生产消费面 `:224-234` 在 result 无 sessionId 时经 `or_else(|| resume_id.clone())`（:229 显式回退）持原 sessionId → 消费面无漂移，无需适配。
- 注记（计划口径）：同进程 load 属探测形态（生产为跨进程 resume），不据此单独定漂移结论；跨进程 resume 形态如 T3 验证轮遇 resume 流再证。

### 2.6 session/update
本轮实触发 4 族（transcript `DIALECT~` 全帧在档，键序以实收为准）：
- `agent_thought_chunk` `content{type:"text",text}` —— 与 `text_turn.jsonl` 冻结 shape 逐字同形 ✓（`parse.rs:106` 消费）。
- `agent_message_chunk` `content{type:"text",text}` —— 同上 ✓（`parse.rs:109`）。
- `session_info_update` `{title}` —— `parse.rs:129` `SessionInfoUpdate` 在册 ✓（本轮 title=探测 prompt 文本）。
- `available_commands_update`（`update.availableCommands[]` + `update.sessionUpdate` key）—— `parse.rs:152` `AvailableCommandsUpdate` 在册 ✓（消费为 no-op，session.rs:597）。
- `tool_call`/`tool_call_update`：本探测无工具调用未触发；冻结 shape（`tool_call_turn.jsonl`+`acp_session_update_tool_call_update.redacted.json` 0.38.0 真实捕获）与适配器消费面（`parse.rs:112-128`）零变化 → live 确认待 T3 coding 链回填（真实 bash 调用必然触发）。

## §3 结论

**验证轮开始前无未处置漂移。**

- 六方法中五个（initialize/session/new/session/prompt/session/load）本轮 live 实触发，消费面全部与冻结基线一致；session/update 已触发 4 通知族全部同形。
- 未触发项（`session/request_permission`、`tool_call`/`tool_call_update` 族）为覆盖面如实登记（待 T3 回填），非漂移。
- 0.43.0 较 0.34.0 冻结面的全部差异均为**纯增量字段**（initialize 能力面/session-new 配置面），无既有键改名/删除/类型变化，生产消费点零受影响。
- Step 5 漂移处置链**未触发**：零 fixture 改动、零 `session.rs`/`parse.rs` 改动（本 Task 代码面仅新增 gated 捕获测试）。
