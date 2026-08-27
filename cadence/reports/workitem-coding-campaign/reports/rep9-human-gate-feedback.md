# codex rep9 人工门反馈（主会话扮演人工）

## 反馈正文（RequestChange 用）

整批复审意见全部成立，按以下确定性约定返修三个 draft（backend/frontend/integration 同步对齐）：

1. **服务生命周期契约（唯一调用形式）**：`server/server.js` 以 CommonJS 导出唯一入口 `start(options)`，签名为 `start({ port, dataPath, staticRoot })` → `Promise<{ origin, port, stop }>`；`port` 支持 0（临时端口），`dataPath` 默认 `data/levels.json`，`staticRoot` 默认 `web/`；`origin` 为 `http://127.0.0.1:<实际端口>`。`stop()` 返回 `Promise<void>`，幂等（重复调用安全），await 返回时监听器必须已关闭。不再单独导出 `createServer`（createServer 语义并入 start 的 options）。
2. **五关卡 canonical 数据**：`data/levels.json` 的期望值在契约中固定为恰好 5 项，字段 id/name/difficulty/unlocked，difficulty ∈ {简单, 普通, 困难}；契约中给出全部 5 条完整记录（id 为 level-1..level-5），backend/frontend/integration 测试至少校验这些稳定值。
3. **前端模块与入口约定**：`web/level-select.js` 不使用 ESM/CommonJS 模块语法，采用浏览器全局挂接：文件内定义 `globalThis.initLevelSelect = function ({ document, fetchImpl }) {...}`，`fetchImpl` 缺省时使用 `globalThis.fetch`；`web/index.html` 以 `<script src="./level-select.js">` 加载并在 `DOMContentLoaded` 时显式调用 `initLevelSelect({ document, fetchImpl: window.fetch })`。前端测试以 Node 内置 vm/全局替身按同一全局约定加载调用。
4. **错误响应契约**：API 错误（500/404/405）统一返回 `Content-Type: application/json; charset=utf-8`，体为 `{"error":{"code": string, "message": string}}`；500 固定 code `LEVEL_DATA_UNAVAILABLE`；静态资源 404 保持文本响应（非 JSON），与 API 404 区分。
5. **术语统一**：全部写作「同源根路径 `/api/levels`」，不再使用「相对路径」表述。

## 操作序列

1. `POST /api/workspace-sessions/workspace_session_0018/takeover` → 拿到新 interactive session id
2. WS 连接新 session，等人工门节点
3. 发送 HumanConfirm::RequestChange + 上述反馈
4. 等返修 + 复评
5. 复评通过后在最终确认门发 Confirm → plan Confirmed + work items 发布
