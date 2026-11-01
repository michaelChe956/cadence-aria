# P38B1 Task 1 报告：中性连接归因与前端 close 关联

## 做了什么

- 为 `WsOutMessage::SessionState` 新增可选 `connection_id` wire 字段；`WorkspaceEngine::build_session_state()` 固定生成 `None`，连接 UUID 只在 WebSocket 发送层覆盖，不进入 durable session 或 engine 所有权。
- WebSocket 发送路径以连接级 outbound 转发器统一装饰 `session_state` JSON；初始快照与 cleanup 快照也使用同一 socket-local connection id。已覆盖当前 handler 内由 `send_json_outbound` 发送的 session snapshot，以及初始直接发送路径。
- 将 receiver loop 改为显式归类 `close_frame`、`eof`、`read_error`；idle 标志置位后投影为 `server_idle`。直接 TCP 丢弃的 tungstenite “reset without closing handshake” 归为 EOF 连接事实。
- 将连接活性拆分为 client/server 两个独立活动记录：idle 仍按任一方向活动判断；诊断保存真实的 RFC3339 最后 client/server 活动时间。
- 新增仅观察性的 `provider_drive_depth`、一次 JSON 序列化的 `ConnectionDiagnostic` stderr 记录，以及只在测试控制启用时写入的每 session TestControls 镜像。
- 在既有 `append_aborted_by_disconnect` detail 追加同一 `connection_id`；未改变 node type、失败状态、title、stage 过渡或既有 cleanup 调用路径。
- 前端新增独立 `connectionCloseDiagnostics`（与 `protocolDiagnostics` 分离、上限 50）；记录浏览器 close code/reason/wasClean、connection id、visibility、ping/pong-or-server-message 时间；同会话重连保留、跨 session 替换清空。
- 新增 server idle、客户端 4000 close、页面卸载 1000 close、TCP drop EOF 四种 integration 覆盖，并保持现有 disconnect cleanup、run 存活、idle hold 回归。

## 验证证据

红灯阶段：

- `cargo test --locked --lib session_state_serializes_optional_connection_id`：因 `SessionState` 尚无 `connection_id` 字段编译失败。
- `cd web && pnpm test src/hooks/useWorkspaceWs.actions.test.tsx`：新增两条 close 归因用例失败，原因是 store/action 尚不存在。

绿灯阶段：

```text
cargo test --locked --test it_core workspace_ws_client_close_4000_records_connection_diagnostic
cargo test --locked --test it_core workspace_ws_page_unload_close_1000_records_connection_diagnostic
cargo test --locked --test it_core workspace_ws_tcp_drop_records_eof_connection_diagnostic
cargo test --locked --test it_core workspace_ws_idle_timeout_records_server_idle_connection_diagnostic
```

上述四条均通过。

```text
cargo test --locked --test it_core workspace_ws_disconnect_during_active_run_writes_aborted_by_disconnect
cargo test --locked --test it_core workspace_ws_disconnect_does_not_cancel_active_provider_run
cargo test --locked --test it_core workspace_ws_idle_timeout_holds_reconnect_while_surviving_run_drives
```

上述既有 cleanup/run 存活/idle hold 回归均通过。

```text
cargo test --locked --lib workspace_ws
```

结果：`150 passed`。

```text
cd web && pnpm tsc -b
cd web && pnpm test src/hooks/useWorkspaceWs.actions.test.tsx src/hooks/useWorkspaceWs.timeline.test.tsx
```

结果：TypeScript build 通过；hook 定向测试 `44 passed`。

完整前端验证：

```text
cd web && pnpm test
```

结果：`166 passed` 个测试文件、`1403 passed` 个测试。运行期间已有 lifecycle 测试输出 jsdom “navigation (except hash changes)” stderr；该测试文件仍通过，且本 Task 未改动该 lifecycle 代码。

## 未覆盖清单与边界

- 已审查当前 socket handler 的 session snapshot 发送面：初始直接发送路径显式装饰；其余经 `send_json_outbound` 的路径由连接级 outbound 转发器统一装饰。当前未发现未覆盖的 `SessionState` 发送点。
- 本 Task 仅提供中性归因半链：连接退出分类、活动时间、run/token/depth 快照和浏览器 close 关联。它不改变既有 `aborted_by_disconnect` 业务语义，也不构成对断连问题的根治结论。
- TestControls 镜像是测试内存观察面；生产侧仅保留唯一 `[aria-connection-diagnostic]` stderr JSON 记录，未新增 JSONL、timeline node、runtime event 或 durable session 字段。

## 提交

```text
da795042 feat: 记录 workspace 连接归因
```

未 push。

## 自审

- R-3：既有 aborted/cleanup 调用顺序、node/status/title/stage 均保持；仅追加 detail 中的 `connection_id`。
- R-2：诊断字段表达连接事实，不将 close 重编码为业务终态；本文不宣称问题已根治。
- `connection_id` 保持 socket-local：engine snapshot 固定为 `None`，仅发送层附加。
- 前端 close 记录未混入 `protocolDiagnostics`，且跨会话清理、同会话保留均有测试覆盖。

## Round 1/5 修复：活动 run drop 集成锚

- 按 k3 P2，在 `workspace_ws_disconnect_during_active_run_writes_aborted_by_disconnect` 的已落盘 `AbortedByDisconnect` 断言块中补充 `summary` 含 `connection_id:`，并精确断言整条 timeline 中该节点类型仅有一个，防止 detail 丢失连接 ID 或重复追加审计节点未被覆盖。
- 定向验证：`cargo test --locked --test it_core workspace_ws_disconnect_during_active_run_writes_aborted_by_disconnect`，结果：`1 passed; 0 failed`。
