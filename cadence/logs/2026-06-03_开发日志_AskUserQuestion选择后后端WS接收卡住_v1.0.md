# AskUserQuestion 选择后后端 WS 接收卡住调试记录

## 背景

分支：`fix_e2e_test`

测试场景：Story Spec workspace 中，Claude Code author 先输出文本选择题，daemon 通过 `text_fallback` 兜底生成选择卡。用户选择后，daemon 使用 delta-only compact QA resume 同一个 Claude Code 会话。随后 resumed Claude Code 又发出结构化 `AskUserQuestion`，用户选择后流程卡住，Claude 进程持续存在，页面显示 Provider 运行中。

## 已确认现象

1. 第一次 `text_fallback` 选择链路正常：
   - 前端发送 `choice_response`。
   - 后端收到 `author_choice_msg_003`。
   - 因当时没有 active run，后端走文本 fallback follow-up。
   - 新 author run 使用 `--resume 74fcd531-80af-480b-8d30-ebb236a614f4`，prompt 是 compact QA，不是完整 prompt。

2. 第二次结构化 `AskUserQuestion` 发出正常：
   - 后端日志出现：
     - `claude received assistant tool_use AskUserQuestion tool_use_id=tool_m7FaF2j7ujuPdAHu8Id1gfox`
     - `bridge emitting choice_request id=tool_m7FaF2j7ujuPdAHu8Id1gfox source=ask_user_question options=2`
     - `ws outbound choice_request session=workspace_session_0001 id=tool_m7FaF2j7ujuPdAHu8Id1gfox source=ask_user_question`

3. 前端确实发送了第二次选择：
   - 浏览器 Console 有：
     - `[aria-choice-diag] frontend choice_response send attempt`
     - `[aria-choice-diag] frontend choice_response send result { sent: true }`
   - Network / WS Frames 中有：
     - `{"type":"choice_response","id":"tool_m7FaF2j7ujuPdAHu8Id1gfox","selected_option_ids":["opt_0"],"free_text":null}`

4. 后端没有读到第二次选择：
   - 后端日志没有出现第二次：
     - `ws inbound choice_response ... id=tool_m7FaF2j7ujuPdAHu8Id1gfox`
   - 因此也没有后续：
     - `engine forwarding author choice_response`
     - `bridge received choice_response`
     - `claude writing control_response`
     - `claude writing AskUserQuestion tool_result`

5. TCP 层显示数据到达后端但未被应用读取：
   - `ss` 显示 `127.0.0.1:4317` 后端连接存在 `Recv-Q`，例如 `Recv-Q=433/580`。
   - 这说明数据已经经过浏览器/Vite proxy 到达 aria 后端 socket，但 aria 进程没有把数据读走。

## 当前判断

问题不在前端按钮没有发送，也还没有进入 provider/Claude 层。

当前断点是：

`浏览器 WS frame -> Vite proxy -> aria 4317 TCP socket` 已到达；但 `handle_workspace_socket` 的后端 WebSocket 接收循环没有继续读出该 frame，所以没有生成 `WsInMessage::ChoiceResponse`。

更具体的怀疑是：`handle_workspace_socket` 使用 `socket.split()` 后，发送任务 `send_task` 在 `ws_sender.send(Message::Text(...)).await` 处可能卡住并占用底层 socket，导致接收半边不能继续 poll，从而后端 TCP `Recv-Q` 堆积。

## 已加临时诊断日志

统一前缀：`[aria-choice-diag]`

前端：

- `web/src/hooks/useWorkspaceWs.ts`
  - `frontend choice_response send attempt`
  - `frontend choice_response send result`
  - 输出 choice id、selected ids、source、node id、connection status、WebSocket readyState。

后端 WS handler：

- `src/web/workspace_ws_handler.rs`
  - `ws outbound choice_request`
  - `ws inbound choice_response`
  - `ws forwarding choice_response to active run`
  - `ws forwarded choice_response to active run`
  - `ws failed to forward choice_response`
  - `ws has no active run ... trying text fallback follow-up`
  - 新增下一轮重点日志：
    - `ws send_task sending outbound type=...`
    - `ws send_task sent outbound type=...`
    - `ws send_task failed outbound type=...`

engine：

- `src/product/workspace_engine.rs`
  - `engine forwarding author choice_response`
  - `engine forwarded author choice_response`
  - `engine failed to forward author choice_response`
  - reviewer 对应日志也已加。

bridge：

- `src/cross_cutting/approval_bridge.rs`
  - `bridge emitting choice_request`
  - `bridge received choice_response`
  - `bridge matched pending choice_response`
  - `bridge resolved choice_request`
  - `bridge missing pending choice_response`

Claude provider：

- `src/cross_cutting/claude_code_provider.rs`
  - `claude received control_request AskUserQuestion`
  - `claude received assistant tool_use AskUserQuestion`
  - `claude got choice decision ...`
  - `claude writing control_response`
  - `claude writing AskUserQuestion tool_result`

## 已执行验证

- `cargo check --locked` 通过。
- `pnpm --dir web exec tsc --noEmit` 通过。
- 前端相关测试通过：
  - `pnpm --dir web test -- --run web/src/hooks/useWorkspaceWs.test.tsx web/src/components/chat-workspace/entries/ChoiceRequestEntry.test.tsx`

## 当前服务状态

最后一次重启后：

- 后端日志 session：`49769`
- 后端进程：
  - `cargo-watch` PID：`127584`
  - `target/debug/aria web` PID：`127623`
- 前端 Vite：
  - `pnpm dev --port 5173` PID：`62610`
  - Vite node PID：`62622`
- 地址：
  - 前端：`http://127.0.0.1:5173`
  - 后端：`http://127.0.0.1:4317`

健康检查通过：

- `curl --noproxy '*' -sS http://127.0.0.1:4317/api/health`
- `curl --noproxy '*' -sS http://127.0.0.1:5173/api/health`
- `curl --noproxy '*' -sS -o /tmp/aria-vite-index.html -w '%{http_code}\n' http://127.0.0.1:5173/`

## 明天继续步骤

1. 重新打开 `http://127.0.0.1:5173`，新建或重新走一个 workspace，不复用已经卡住的旧现场。
2. 复现到第二次结构化 `AskUserQuestion`。
3. 选择选项后看后端日志 session `49769`。
4. 重点判断：
   - 如果出现 `ws send_task sending outbound type=choice_request ...` 但没有 `ws send_task sent outbound type=choice_request ...`，说明发送半边阻塞，接收半边无法读 socket。
   - 如果 `ws send_task sent outbound type=choice_request ...` 已出现，但仍没有 `ws inbound choice_response`，则继续查 axum/tungstenite split 接收半边是否被其他 await 阻塞，或 Vite proxy 与后端 WS 的帧转发状态。
   - 如果出现 `ws inbound choice_response`，但没有 `engine forwarded`，则问题转到 active run command channel。
   - 如果出现 `bridge resolved` 和 `claude writing ...`，但 Claude 仍不继续，则问题转到 Claude Code AskUserQuestion 协议回写格式。

## 注意事项

- 当前诊断日志是临时插桩，不应作为最终产品日志直接保留。
- 结构化 `AskUserQuestion` 事件目前没有持久化到 timeline detail；刷新后现场恢复不完整，这是另一个已观察到的问题，后续需要单独修。
- 前端“已选择”是乐观 UI：`sendJson` 返回 true 后立即本地 `resolveChoiceRequest`，不代表后端已处理成功。
