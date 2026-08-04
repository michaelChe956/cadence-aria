## Completed

已完成图片生成 32.7 秒超时的诊断、日志增强和验证。

### 根因判断

**当前最高概率的直接错误类型是 reqwest 0.12.28 在 Linux 上默认的 30 秒 `TCP_USER_TIMEOUT`，而不是 600 秒总请求超时，也不是普通 connect timeout。**

证据：

- `Cargo.lock` 锁定 `reqwest 0.12.28`。
- 该版本 Linux 默认配置包含：
  - `connect_timeout: None`
  - `tcp_user_timeout: Some(Duration::from_secs(30))`
- `TCP_USER_TIMEOUT` 独立于 `.timeout(Duration::from_secs(600))`，用于限制已发送数据保持未确认状态的时间。
- 建立了同版本最小复现：服务端接受 TCP 后不读取 POST body，客户端总超时设为 600 秒，最终在 **30.214 秒**失败：
  - `is_timeout=true`
  - `is_connect=false`
  - `is_request=true`
  - `is_body=false`
  - source 最底层为 `Os { code: 110, kind: TimedOut, message: "Connection timed out" }`
- 该复现的 Display 同生产日志一致：`error sending request for url (...)`，时序也与生产的 32.713 秒高度吻合。
- 对 colorflowai 的匿名探测中，DNS、TCP、TLS 和 nginx 响应约 0.719 秒完成，因此 connect/TLS 超时概率较低。

仍需 controller 重启后端并进行一次真实请求复测，由新增的 source 链最终确认。造成 TCP 数据未确认的下游原因可能是网关在接收 multipart 时停止读取、网络丢包或上游背压；目前没有公开证据证明 colorflowai 固定配置了 30 秒生成超时。

### 诊断日志改进

发送失败日志现在会打印：

- reqwest 顶层 Display 和 Debug
- `is_timeout()`
- `is_connect()`
- `is_request()`
- `is_body()`
- 完整 `std::error::Error::source()` 链，每层同时打印 Display 和 Debug

提交：

- **Commit:** `eb39ab7762481862350df4e84b316a39fb954622`
- **Message:** `fix(image-create): 延长 image 请求超时到 600s + vite 代理超时对齐 + 详细诊断日志`

按 supervisor 批准，该提交也包含同一调试工作中已有的后端和 Vite 开发代理 600 秒 timeout 配置。

### 修复建议

先用增强日志进行真实复测，再采取对应修复：

1. 若确认 source 为 IO `TimedOut`，且 flags 为 `timeout=true, connect=false, request=true`：
   - 在图片专用 client 显式设置 `.tcp_user_timeout(Duration::from_secs(600))`；或
   - 设置 `.tcp_user_timeout(None)`，由应用层 600 秒总 deadline 控制；
   - 同时建议增加独立、较短的 `.connect_timeout(...)`，避免连接建立无限等待。
2. 若出现 `Connection reset by peer`、`UnexpectedEof` 或 HTTP 502/504：
   - 检查 colorflowai/nginx 的 `proxy_read_timeout`、`proxy_send_timeout`、upstream deadline；
   - 使用 `X-Oneapi-Request-Id` 和时间戳关联网关日志；
   - 必要时改为异步 job、callback 或 polling。
3. gpt-image-2 编辑超过 30 秒属于正常场景：
   - OpenAI 文档指出复杂请求可能接近 2 分钟；
   - 公开兼容网关示例的编辑请求耗时为 83.859 秒。
4. Vite 的 600 秒代理配置只能处理前端到本地后端这一层，不能单独解释本次后端到 colorflowai 的 `send()` 失败。

未重启后端，未使用真实 API key 发起生成请求。

## Files Changed

- `src/cross_cutting/image_client.rs` - 后端总超时调整到 600 秒，并增加 reqwest kind、Debug 和完整 source 链诊断日志。
- `web/dev-server-proxy.ts` - 开发环境 `/api` 代理的 `timeout` 和 `proxyTimeout` 对齐到 600 秒。
- `/tmp/diagnose-timeout-report.md` - 完整诊断报告；指定的 `.superpowers/sdd/2026-08-03_计划文档_功能开发_图片创作agent_v1.9/` 目录不存在，因此按要求使用 fallback 路径。

## Verification

- `cargo fmt --check`：通过。
- `cargo build --locked --bin aria`：提交后通过，约 6.48 秒。
- `cargo test --locked --lib cross_cutting::image_client::tests`：通过，4 passed、0 failed。
- `cd web && pnpm tsc -b`：通过。
- `git diff --check`：通过。
- TCP timeout 最小复现：通过，在 30.214 秒复现 reqwest 0.12.28 默认 `TCP_USER_TIMEOUT` 的错误形态。
- 暂存区为空；工作树仅有既存未跟踪目录 `.pi-subagents/`。

## Notes

- 新日志尚未在真实 colorflowai 请求上执行，因此最终 source 链仍需用户复测获取。
- 没找到 colorflowai 公开文档明确声明 `/v1/images/edits` 存在 30 秒网关超时。
- 未新增测试文件；诊断日志不改变返回值和错误归一化行为，使用现有 image client 单元测试验证。